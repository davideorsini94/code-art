//! Ricerca nel progetto: in sola lettura, a blocchi e annullabile.
use grep_matcher::{Captures, Matcher};
use grep_regex::{RegexMatcher, RegexMatcherBuilder};
use grep_searcher::{BinaryDetection, Searcher, SearcherBuilder, Sink, SinkMatch};
use ignore::overrides::{Override, OverrideBuilder};
use ignore::WalkState;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Manager, State, WebviewWindow};
use unicode_normalization::UnicodeNormalization;

use crate::store::Store;

pub const CHUNK: usize = 200;
pub const MAX_HITS: usize = 10_000;
pub const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;
pub const PREVIEW_CHARS: usize = 240;
/// Corrispondenze per riga oltre le quali si passa alla riga dopo (file minificati).
pub const MAX_MATCHES_PER_LINE: usize = 1000;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SearchQuery {
    pub text: String,
    pub regex: bool,
    pub case_sensitive: bool,
    pub whole_word: bool,
    pub include: Vec<String>,
    pub exclude: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    pub path: String,
    pub line: u32,
    pub col: u32,
    pub len: u32,
    pub preview: String,
    /// Offset (UTF-16, da 0) della corrispondenza dentro `preview`.
    pub preview_col: u32,
    /// Lunghezza (UTF-16) della corrispondenza dentro `preview`, tagliata alla finestra.
    pub preview_len: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchSummary {
    pub files: u32,
    pub hits: u32,
    pub truncated: bool,
    pub cancelled: bool,
    pub error: Option<String>,
}

struct Shared<'a, F> {
    sink: F,
    cancel: &'a AtomicBool,
    buf: Vec<SearchHit>,
    files: u32,
    hits: usize,
    truncated: bool,
}

impl<F: FnMut(Vec<SearchHit>)> Shared<'_, F> {
    /// Aggiunge le corrispondenze di un file e consegna i blocchi pieni; `false` = fermarsi (limite o annullamento).
    /// Dopo l'annullamento non consegna più nulla.
    fn push(&mut self, hits: Vec<SearchHit>) -> bool {
        if self.cancel.load(Ordering::Relaxed) {
            return false;
        }
        self.files += 1;
        let room = MAX_HITS - self.hits;
        self.hits += hits.len().min(room);
        self.buf.extend(hits.into_iter().take(room));
        if self.hits >= MAX_HITS {
            self.truncated = true;
        }
        while self.buf.len() >= CHUNK && !self.cancel.load(Ordering::Relaxed) {
            let rest = self.buf.split_off(CHUNK);
            (self.sink)(std::mem::replace(&mut self.buf, rest));
        }
        !self.truncated && !self.cancel.load(Ordering::Relaxed)
    }
}

/// Raccoglie le corrispondenze di un file; scarta tutto se il file risulta binario.
struct Collector<'a, M: Matcher> {
    matcher: &'a M,
    path: &'a str,
    cancel: &'a AtomicBool,
    hits: Vec<SearchHit>,
    binary: bool,
}

impl<M: Matcher> Sink for Collector<'_, M> {
    type Error = std::io::Error;

    fn matched(&mut self, _: &Searcher, m: &SinkMatch<'_>) -> Result<bool, Self::Error> {
        if self.cancel.load(Ordering::Relaxed) {
            return Ok(false);
        }
        let line_no = m.line_number().unwrap_or(0) as u32;
        let line = String::from_utf8_lossy(m.bytes());
        let line = line.trim_end_matches(['\n', '\r']);
        let cancel = self.cancel;
        let mut found: Vec<(usize, usize)> = Vec::new();
        let _ = self.matcher.find_iter(line.as_bytes(), |mm| {
            if mm.end() > mm.start() {
                found.push((mm.start(), mm.end()));
            }
            found.len() < MAX_MATCHES_PER_LINE && !cancel.load(Ordering::Relaxed)
        });
        // una sola passata: le corrispondenze sono ordinate e non si sovrappongono
        let long = line.len() > PREVIEW_CHARS && line.chars().count() > PREVIEW_CHARS;
        let (mut pos, mut utf16) = (0usize, 0u32);
        for (s, e) in found {
            if cancel.load(Ordering::Relaxed) {
                return Ok(false);
            }
            utf16 += utf16_len(&line[pos..s]);
            pos = s;
            let len = utf16_len(&line[s..e]);
            let (preview, preview_col, preview_len) = if long {
                window(line, s, e)
            } else {
                (line.to_string(), utf16, len)
            };
            self.hits.push(SearchHit { path: self.path.to_string(), line: line_no, col: utf16 + 1, len, preview, preview_col, preview_len });
            if self.hits.len() >= MAX_HITS {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn binary_data(&mut self, _: &Searcher, _: u64) -> Result<bool, Self::Error> {
        self.binary = true;
        Ok(false)
    }
}

fn utf16_len(s: &str) -> u32 {
    s.encode_utf16().count() as u32
}

/// Finestra di al più `PREVIEW_CHARS` caratteri che contiene l'inizio della corrispondenza `s..e` (byte di `line`):
/// (testo, offset UTF-16 della corrispondenza nel testo, sua lunghezza UTF-16 tagliata alla finestra).
/// Costo limitato dalla dimensione della finestra, non dalla lunghezza della riga.
fn window(line: &str, s: usize, e: usize) -> (String, u32, u32) {
    let mlen = line[s..e].chars().take(PREVIEW_CHARS).count();
    let back = (PREVIEW_CHARS - mlen) / 4;
    let start = line[..s].char_indices().rev().take(back).last().map_or(s, |(i, _)| i);
    let end = line[start..].char_indices().nth(PREVIEW_CHARS).map_or(line.len(), |(i, _)| start + i);
    (line[start..end].to_string(), utf16_len(&line[start..s]), utf16_len(&line[s..e.min(end)]))
}

fn build_matcher(q: &SearchQuery) -> Result<RegexMatcher, String> {
    RegexMatcherBuilder::new()
        .case_insensitive(!q.case_sensitive)
        .word(q.whole_word)
        .fixed_strings(!q.regex)
        .build(&q.text)
        .map_err(|e| format!("Espressione non valida: {e}"))
}

/// Un `Override` per radice: `include` = whitelist, `exclude` = `!glob` (sintassi gitignore).
fn build_overrides(roots: &[PathBuf], q: &SearchQuery) -> Result<Vec<(PathBuf, Override)>, String> {
    roots
        .iter()
        .map(|r| {
            let mut b = OverrideBuilder::new(r);
            for g in q.include.iter().map(|g| g.trim().trim_start_matches('!')).filter(|g| !g.is_empty()) {
                b.add(g).map_err(|e| format!("Filtro non valido «{g}»: {e}"))?;
            }
            for g in q.exclude.iter().map(|g| g.trim().trim_start_matches('!')).filter(|g| !g.is_empty()) {
                b.add(&format!("!{g}")).map_err(|e| format!("Filtro non valido «{g}»: {e}"))?;
            }
            Ok((r.clone(), b.build().map_err(|e| e.to_string())?))
        })
        .collect()
}

fn nfc(p: &Path) -> String {
    p.to_string_lossy().nfc().collect()
}

/// Cerca `q` nei file sotto `roots`. Le corrispondenze arrivano a `sink` in blocchi di al più `CHUNK`, da thread
/// diversi ma mai in parallelo; `cancel` ferma la ricerca appena possibile.
pub fn search(roots: &[PathBuf], q: &SearchQuery, cancel: &AtomicBool, sink: impl FnMut(Vec<SearchHit>) + Send) -> SearchSummary {
    let mut summary = SearchSummary::default();
    if q.text.is_empty() || roots.is_empty() {
        return summary;
    }
    let (matcher, overrides) = match (build_matcher(q), build_overrides(roots, q)) {
        (Ok(m), Ok(o)) => (m, o),
        (Err(e), _) | (_, Err(e)) => {
            summary.error = Some(e);
            return summary;
        }
    };
    let noise_roots = roots.to_vec();
    let overrides = Arc::new(overrides);
    let mut wb = ignore::WalkBuilder::new(&roots[0]);
    for r in &roots[1..] {
        wb.add(r);
    }
    wb.hidden(false).follow_links(false).require_git(false).max_filesize(Some(MAX_FILE_BYTES));
    {
        let overrides = overrides.clone();
        wb.filter_entry(move |e| {
            if crate::fs_tree::is_noise(&noise_roots, e.path()) {
                return false;
            }
            let is_dir = e.file_type().is_some_and(|t| t.is_dir());
            match overrides.iter().find(|(r, _)| e.path().starts_with(r)) {
                Some((_, ov)) => !ov.matched(e.path(), is_dir).is_ignore(),
                None => true,
            }
        });
    }
    let shared = Mutex::new(Shared { sink, cancel, buf: Vec::new(), files: 0, hits: 0, truncated: false });
    wb.build_parallel().run(|| {
        let matcher = matcher.clone();
        let shared = &shared;
        let mut searcher = SearcherBuilder::new().binary_detection(BinaryDetection::quit(0)).line_number(true).build();
        Box::new(move |res| {
            if cancel.load(Ordering::Relaxed) {
                return WalkState::Quit;
            }
            let Ok(entry) = res else { return WalkState::Continue };
            if !entry.file_type().is_some_and(|t| t.is_file()) {
                return WalkState::Continue;
            }
            // `max_filesize` vale anche per i file: ricontrollo esplicito, per non dipendere da un dettaglio
            if entry.metadata().map(|m| m.len() > MAX_FILE_BYTES).unwrap_or(true) {
                return WalkState::Continue;
            }
            let path = nfc(entry.path());
            let mut c = Collector { matcher: &matcher, path: &path, cancel, hits: Vec::new(), binary: false };
            if searcher.search_path(&matcher, entry.path(), &mut c).is_err() || c.binary {
                return WalkState::Continue; // illeggibile o binario
            }
            let mut sh = crate::lock(shared);
            let more = sh.push(c.hits);
            if more && !cancel.load(Ordering::Relaxed) { WalkState::Continue } else { WalkState::Quit }
        })
    });
    let mut sh = shared.into_inner().unwrap_or_else(std::sync::PoisonError::into_inner);
    if !cancel.load(Ordering::Relaxed) && !sh.buf.is_empty() {
        let rest = std::mem::take(&mut sh.buf);
        (sh.sink)(rest);
    }
    summary.files = sh.files;
    summary.hits = sh.hits as u32;
    summary.truncated = sh.truncated;
    summary.cancelled = cancel.load(Ordering::Relaxed);
    summary
}

// ---------- comandi ----------

const MAX_TEXT_CHARS: usize = 1000;
const MAX_GLOBS: usize = 50;
const MAX_GLOB_CHARS: usize = 200;

/// Una ricerca attiva per progetto: (id, flag di annullamento).
#[derive(Default)]
pub struct Searches {
    next: AtomicU64,
    active: Mutex<HashMap<String, (u64, Arc<AtomicBool>)>>,
}

impl Searches {
    /// Annulla e dimentica la ricerca del progetto (chiusura della finestra).
    pub fn cancel_project(&self, project_id: &str) {
        if let Some((_, flag)) = crate::lock(&self.active).remove(project_id) {
            flag.store(true, Ordering::Relaxed);
        }
    }
}

#[derive(Serialize, Clone)]
struct HitsEvent {
    id: u64,
    hits: Vec<SearchHit>,
}

#[derive(Serialize, Clone)]
struct DoneEvent {
    id: u64,
    summary: SearchSummary,
}

/// La pagina non è fidata: lunghezze e numero di filtri limitati.
fn validate(q: &SearchQuery) -> Result<(), String> {
    if q.text.chars().count() > MAX_TEXT_CHARS {
        return Err("Testo da cercare troppo lungo".into());
    }
    if q.include.len() > MAX_GLOBS || q.exclude.len() > MAX_GLOBS || q.include.iter().chain(&q.exclude).any(|g| g.chars().count() > MAX_GLOB_CHARS) {
        return Err("Filtri troppo numerosi o troppo lunghi".into());
    }
    Ok(())
}

/// Avvia una ricerca nelle cartelle del progetto della finestra chiamante e restituisce il suo id; la precedente
/// della stessa finestra viene annullata. Risultati su `search:hits`, fine su `search:done`.
#[tauri::command]
pub fn search_start(app: AppHandle, window: WebviewWindow, store: State<'_, Store>, searches: State<'_, Searches>, query: SearchQuery) -> Result<u64, String> {
    validate(&query)?;
    let project_id = crate::projects::window_project_id(&window)?;
    let roots = crate::fs_tree::roots_of(&window, &store).map_err(|e| e.message)?;
    let id = searches.next.fetch_add(1, Ordering::Relaxed) + 1;
    let cancel = Arc::new(AtomicBool::new(false));
    if let Some((_, old)) = crate::lock(&searches.active).insert(project_id.clone(), (id, cancel.clone())) {
        old.store(true, Ordering::Relaxed);
    }
    let handle = app.clone();
    std::thread::spawn(move || {
        let emit_id = project_id.clone();
        let summary = search(&roots, &query, &cancel, |hits| {
            crate::projects::emit_to_project(&handle, &emit_id, "search:hits", HitsEvent { id, hits });
        });
        crate::projects::emit_to_project(&handle, &project_id, "search:done", DoneEvent { id, summary });
        let searches = handle.state::<Searches>();
        let mut active = crate::lock(&searches.active);
        if active.get(&project_id).is_some_and(|(cur, _)| *cur == id) {
            active.remove(&project_id);
        }
    });
    Ok(id)
}

/// Annulla la ricerca `id` se è quella attiva della finestra chiamante.
#[tauri::command]
pub fn search_cancel(window: WebviewWindow, searches: State<'_, Searches>, id: u64) -> Result<(), String> {
    let project_id = crate::projects::window_project_id(&window)?;
    if let Some((cur, flag)) = crate::lock(&searches.active).get(&project_id)
        && *cur == id
    {
        flag.store(true, Ordering::Relaxed);
    }
    Ok(())
}

// ---------- sostituzione ----------

const MAX_REPLACE_PATHS: usize = 10_000;
const MAX_REPLACEMENT_CHARS: usize = 5000;
pub const BTN_REPLACE: &str = "Sostituisci";
pub const BTN_CANCEL: &str = "Annulla";

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplaceOutcome {
    pub files: u32,
    pub replacements: u32,
    /// File non modificati (non UTF-8, troppo grandi, cambiati su disco dopo la lettura, illeggibili).
    pub skipped: Vec<String>,
    /// L'utente ha risposto «Annulla» alla conferma: nessun file toccato.
    pub cancelled: bool,
}

/// Sostituisce riga per riga (come la ricerca: nessuna corrispondenza attraversa un a capo, nessuna riga vuota
/// sostituita); `None` se il testo non contiene corrispondenze. Senza `regex` il testo nuovo è letterale, con
/// `regex` valgono `$1`, `${nome}` e `$$`.
fn replace_text(matcher: &RegexMatcher, q: &SearchQuery, replacement: &str, text: &str) -> Result<Option<(String, u32)>, String> {
    let mut caps = matcher.new_captures().map_err(|e| e.to_string())?;
    let (mut out, mut count) = (String::with_capacity(text.len()), 0u32);
    for line in text.split_inclusive('\n') {
        let body = line.trim_end_matches(['\n', '\r']);
        let eol = &line[body.len()..];
        let mut dst: Vec<u8> = Vec::with_capacity(body.len());
        let mut n = 0u32;
        matcher
            .replace_with_captures(body.as_bytes(), &mut caps, &mut dst, |caps, dst| {
                if caps.get(0).is_some_and(|m| m.end() > m.start()) {
                    n += 1;
                    if q.regex {
                        caps.interpolate(|name| matcher.capture_index(name), body.as_bytes(), replacement.as_bytes(), dst);
                    } else {
                        dst.extend_from_slice(replacement.as_bytes());
                    }
                } else if let Some(m) = caps.get(0) {
                    dst.extend_from_slice(&body.as_bytes()[m.start()..m.end()]);
                }
                true
            })
            .map_err(|e| e.to_string())?;
        if n == 0 {
            out.push_str(line);
        } else {
            count += n;
            out.push_str(&String::from_utf8(dst).map_err(|_| "Sostituzione non valida".to_string())?);
            out.push_str(eol);
        }
    }
    Ok((count > 0).then_some((out, count)))
}

/// Ricalcola le corrispondenze sul disco per ogni file di `paths` (già confinati), chiede conferma con
/// `confirm(file, occorrenze)` e scrive con `fs_tree::write` (rifiutato se il file è cambiato dopo la lettura).
pub fn replace_in_files(paths: &[PathBuf], q: &SearchQuery, replacement: &str, confirm: &dyn Fn(u32, u32) -> bool) -> Result<ReplaceOutcome, String> {
    let mut out = ReplaceOutcome::default();
    if q.text.is_empty() {
        return Ok(out);
    }
    let matcher = build_matcher(q)?;
    let mut plan: Vec<(&PathBuf, String, u32, i64)> = Vec::new();
    for p in paths {
        let read = || -> Option<(String, i64)> {
            let meta = std::fs::metadata(p).ok().filter(|m| m.is_file() && m.len() <= MAX_FILE_BYTES)?;
            let text = String::from_utf8(std::fs::read(p).ok()?).ok().filter(|t| !t.contains('\0'))?;
            Some((text, crate::fs_tree::mtime_ms(&meta)))
        };
        let Some((text, mtime)) = read() else {
            out.skipped.push(nfc(p));
            continue;
        };
        if let Some((new, n)) = replace_text(&matcher, q, replacement, &text)? {
            plan.push((p, new, n, mtime));
        }
    }
    let total: u32 = plan.iter().map(|x| x.2).sum();
    if plan.is_empty() {
        return Ok(out);
    }
    if !confirm(plan.len() as u32, total) {
        out.cancelled = true;
        return Ok(out);
    }
    for (p, new, n, mtime) in plan {
        match crate::fs_tree::write(p, &new, Some(mtime)) {
            Ok(_) => {
                out.files += 1;
                out.replacements += n;
            }
            Err(_) => out.skipped.push(nfc(p)),
        }
    }
    Ok(out)
}

/// Sostituisce `query` con `replacement` nei file `paths` del progetto della finestra chiamante: solo progetto
/// fidato, percorsi confinati, conferma NATIVA del core («Annulla» predefinito). Le corrispondenze si ricalcolano
/// sul disco: i risultati della pagina non sono fidati.
#[tauri::command]
pub async fn search_replace(
    app: AppHandle,
    window: WebviewWindow,
    store: State<'_, Store>,
    query: SearchQuery,
    replacement: String,
    paths: Vec<String>,
) -> Result<ReplaceOutcome, String> {
    use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind, MessageDialogResult};
    validate(&query)?;
    if query.text.is_empty() {
        return Err("Testo da cercare vuoto".into());
    }
    if replacement.chars().count() > MAX_REPLACEMENT_CHARS || paths.len() > MAX_REPLACE_PATHS {
        return Err("Sostituzione troppo grande".into());
    }
    let resolved = replace_targets(&store, window.label(), &paths)?;
    let Some(_guard) = app.state::<crate::claude::delete::DeletePrompts>().acquire(window.label()) else {
        return Err("Un'altra conferma è già aperta".into());
    };
    let (handle, win) = (app.clone(), window.clone());
    tauri::async_runtime::spawn_blocking(move || {
        let confirm = |files: u32, n: u32| {
            let answer = handle
                .dialog()
                .message(replace_message(files, n))
                .title("Sostituire nel progetto?")
                .kind(MessageDialogKind::Warning)
                .buttons(MessageDialogButtons::OkCancelCustom(BTN_CANCEL.into(), BTN_REPLACE.into()))
                .parent(&win)
                .blocking_show_with_result();
            matches!(answer, MessageDialogResult::Custom(l) if l == BTN_REPLACE)
        };
        replace_in_files(&resolved, &query, &replacement, &confirm)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Percorsi da modificare per la finestra `label`: progetto fidato, percorsi dentro il progetto (link simbolici
/// risolti), senza doppioni.
pub(crate) fn replace_targets(store: &Store, label: &str, paths: &[String]) -> Result<Vec<PathBuf>, String> {
    let project_id = crate::projects::project_id_of_label(label).ok_or("questa finestra non è un progetto")?;
    crate::trust::ensure_project_trusted(store, project_id)?;
    let mut resolved: Vec<PathBuf> = Vec::new();
    let mut seen: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    for p in paths {
        let r = crate::fs_tree::mutable_path(store, label, p).map_err(|e| e.message)?;
        if seen.insert(r.clone()) {
            resolved.push(r);
        }
    }
    Ok(resolved)
}

fn replace_message(files: u32, n: u32) -> String {
    let occ = if n == 1 { "occorrenza" } else { "occorrenze" };
    format!("Sostituire {n} {occ} in {files} file?")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use std::sync::atomic::Ordering;

    fn q(text: &str) -> SearchQuery {
        SearchQuery { text: text.into(), ..Default::default() }
    }

    fn run(root: &Path, query: &SearchQuery) -> (Vec<SearchHit>, SearchSummary) {
        let mut hits = Vec::new();
        let s = search(&[root.to_path_buf()], query, &AtomicBool::new(false), |c| {
            assert!(c.len() <= CHUNK);
            hits.extend(c)
        });
        hits.sort_by(|a, b| (&a.path, a.line, a.col).cmp(&(&b.path, b.line, b.col)));
        (hits, s)
    }

    fn names(root: &Path, hits: &[SearchHit]) -> Vec<String> {
        let mut v: Vec<String> = hits.iter().map(|h| h.path.strip_prefix(&format!("{}/", root.display())).unwrap().to_string()).collect();
        v.dedup();
        v
    }

    fn w(root: &Path, rel: &str, content: impl AsRef<[u8]>) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, content).unwrap();
    }

    #[test]
    fn finds_literal_with_line_and_col() {
        let d = tempfile::tempdir().unwrap();
        w(d.path(), "a.txt", "uno\n  due needle tre needle\nquattro\n");
        let (hits, s) = run(d.path(), &q("needle"));
        assert_eq!(hits.len(), 2);
        assert_eq!((hits[0].line, hits[0].col, hits[0].len), (2, 7, 6));
        assert_eq!((hits[1].line, hits[1].col), (2, 18));
        assert_eq!(hits[0].preview, "  due needle tre needle");
        assert!(hits[0].path.starts_with('/'));
        assert_eq!((s.files, s.hits, s.truncated, s.error), (1, 2, false, None));
    }

    #[test]
    fn col_counts_utf16_units() {
        let d = tempfile::tempdir().unwrap();
        w(d.path(), "a.txt", "è😀x needle\n");
        let (hits, _) = run(d.path(), &q("needle"));
        assert_eq!(hits[0].col, 6); // è=1, 😀=2, x=1, spazio=1
    }

    #[test]
    fn regex_and_whole_word() {
        let d = tempfile::tempdir().unwrap();
        w(d.path(), "a.txt", "foo foobar f.o\n");
        let (hits, _) = run(d.path(), &SearchQuery { text: "fo+".into(), regex: true, ..Default::default() });
        assert_eq!(hits.len(), 2);
        let (hits, _) = run(d.path(), &SearchQuery { text: "foo".into(), whole_word: true, ..Default::default() });
        assert_eq!(hits.len(), 1);
        // senza regex il punto è letterale
        let (hits, _) = run(d.path(), &q("f.o"));
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn case_insensitive_default() {
        let d = tempfile::tempdir().unwrap();
        w(d.path(), "a.txt", "Hello hello HELLO\n");
        assert_eq!(run(d.path(), &q("hello")).0.len(), 3);
        let cs = SearchQuery { text: "hello".into(), case_sensitive: true, ..Default::default() };
        assert_eq!(run(d.path(), &cs).0.len(), 1);
    }

    #[test]
    fn respects_gitignore_and_noise() {
        let d = tempfile::tempdir().unwrap();
        w(d.path(), ".git/config", "needle\n");
        w(d.path(), ".gitignore", "ignorato/\n*.log\n");
        w(d.path(), "ignorato/x.txt", "needle\n");
        w(d.path(), "x.log", "needle\n");
        w(d.path(), "node_modules/m/i.js", "needle\n");
        w(d.path(), "target/t.txt", "needle\n");
        w(d.path(), "src/ok.txt", "needle\n");
        let (hits, _) = run(d.path(), &q("needle"));
        assert_eq!(names(d.path(), &hits), vec!["src/ok.txt"]);
    }

    #[test]
    fn include_exclude_globs() {
        let d = tempfile::tempdir().unwrap();
        w(d.path(), "src/a.rs", "needle\n");
        w(d.path(), "src/b.ts", "needle\n");
        w(d.path(), "docs/c.rs", "needle\n");
        let inc = SearchQuery { text: "needle".into(), include: vec!["*.rs".into()], ..Default::default() };
        assert_eq!(names(d.path(), &run(d.path(), &inc).0), vec!["docs/c.rs", "src/a.rs"]);
        let exc = SearchQuery { text: "needle".into(), exclude: vec!["docs/".into(), "*.ts".into()], ..Default::default() };
        assert_eq!(names(d.path(), &run(d.path(), &exc).0), vec!["src/a.rs"]);
        let both = SearchQuery { text: "needle".into(), include: vec!["src/**".into()], exclude: vec!["*.ts".into()], ..Default::default() };
        assert_eq!(names(d.path(), &run(d.path(), &both).0), vec!["src/a.rs"]);
    }

    #[test]
    fn skips_binary_and_large_files() {
        let d = tempfile::tempdir().unwrap();
        w(d.path(), "bin.dat", b"needle\nabc\0def\n".as_slice());
        w(d.path(), "bin2.dat", b"\0\0needle\n".as_slice());
        let mut big = vec![b'x'; (MAX_FILE_BYTES + 1) as usize];
        big.extend_from_slice(b"\nneedle\n");
        w(d.path(), "big.txt", big);
        w(d.path(), "ok.txt", "needle\n");
        let (hits, s) = run(d.path(), &q("needle"));
        assert_eq!(names(d.path(), &hits), vec!["ok.txt"]);
        assert_eq!(s.files, 1);
    }

    #[test]
    fn stops_at_limit_with_truncated() {
        let d = tempfile::tempdir().unwrap();
        let line = "a ".repeat(100) + "\n";
        for i in 0..6 {
            w(d.path(), &format!("f{i}.txt"), line.repeat(100)); // 10 000 corrispondenze per file
        }
        let mut chunks = 0;
        let s = search(&[d.path().to_path_buf()], &q("a"), &AtomicBool::new(false), |c| {
            assert!(c.len() <= CHUNK);
            chunks += 1;
        });
        assert_eq!(s.hits as usize, MAX_HITS);
        assert!(s.truncated);
        assert_eq!(chunks, MAX_HITS / CHUNK);
    }

    #[test]
    fn cancel_stops_early() {
        let d = tempfile::tempdir().unwrap();
        for i in 0..50 {
            w(d.path(), &format!("f{i}.txt"), "needle\n".repeat(100));
        }
        let cancel = AtomicBool::new(true);
        let mut n = 0;
        let s = search(&[d.path().to_path_buf()], &q("needle"), &cancel, |c| n += c.len());
        assert_eq!(n, 0);
        assert!(s.error.is_none());
        // annullata dopo il primo blocco: nessun altro blocco, riassunto annullato
        let cancel = AtomicBool::new(false);
        let mut chunks = 0;
        let s = search(&[d.path().to_path_buf()], &q("needle"), &cancel, |_| {
            chunks += 1;
            cancel.store(true, Ordering::Relaxed);
        });
        assert_eq!(chunks, 1);
        assert!(s.cancelled);
    }

    #[test]
    fn invalid_regex_reports_error() {
        let d = tempfile::tempdir().unwrap();
        w(d.path(), "a.txt", "x\n");
        let (hits, s) = run(d.path(), &SearchQuery { text: "(".into(), regex: true, ..Default::default() });
        assert!(hits.is_empty());
        assert!(s.error.is_some());
        let (_, s) = run(d.path(), &SearchQuery { text: "x".into(), include: vec!["[".into()], ..Default::default() });
        assert!(s.error.is_some());
    }

    #[test]
    fn non_utf8_file_does_not_panic() {
        let d = tempfile::tempdir().unwrap();
        w(d.path(), "latin1.txt", b"caff\xe8 needle \xff\xfe\n".as_slice());
        let (hits, s) = run(d.path(), &q("needle"));
        assert_eq!(hits.len(), 1);
        assert!(hits[0].preview.contains("needle"));
        assert!(s.error.is_none());
    }

    #[test]
    fn nfd_filename_reported_as_nfc() {
        use unicode_normalization::UnicodeNormalization;
        let d = tempfile::tempdir().unwrap();
        let nfd: String = "perche\u{301}.txt".into();
        w(d.path(), &nfd, "needle\n");
        let (hits, _) = run(d.path(), &q("needle"));
        assert_eq!(hits.len(), 1);
        assert!(hits[0].path.ends_with("perch\u{e9}.txt"), "{}", hits[0].path);
        assert_eq!(hits[0].path, hits[0].path.nfc().collect::<String>());
    }

    #[test]
    fn long_line_preview_is_bounded_and_contains_match() {
        let d = tempfile::tempdir().unwrap();
        w(d.path(), "a.txt", format!("{}needle{}\n", "é".repeat(1000), "è".repeat(1000)));
        let (hits, _) = run(d.path(), &q("needle"));
        assert!(hits[0].preview.chars().count() <= PREVIEW_CHARS);
        assert!(hits[0].preview.contains("needle"));
    }

    #[test]
    fn does_not_follow_symlinks() {
        let d = tempfile::tempdir().unwrap();
        let (proj, out) = (d.path().join("p"), d.path().join("out"));
        w(&out, "s.txt", "needle\n");
        w(&proj, "ok.txt", "needle\n");
        std::os::unix::fs::symlink(&out, proj.join("link")).unwrap();
        std::os::unix::fs::symlink(out.join("s.txt"), proj.join("s.txt")).unwrap();
        let (hits, _) = run(&proj, &q("needle"));
        assert_eq!(names(&proj, &hits), vec!["ok.txt"]);
    }

    /// Sottostringa UTF-16 `[col, col+len)` di `text`.
    fn utf16_slice(text: &str, col: u32, len: u32) -> String {
        let u: Vec<u16> = text.encode_utf16().collect();
        String::from_utf16(&u[col as usize..(col + len) as usize]).unwrap()
    }

    #[test]
    fn preview_offsets_point_at_the_match_for_short_and_long_lines() {
        let d = tempfile::tempdir().unwrap();
        w(d.path(), "short.txt", "è😀 needle 😀\n");
        w(d.path(), "long.txt", format!("{}needle{}\n", "😀é".repeat(300), "è".repeat(500)));
        w(d.path(), "start.txt", format!("needle{}\n", "x".repeat(500)));
        let (hits, _) = run(d.path(), &q("needle"));
        assert_eq!(hits.len(), 3);
        for h in &hits {
            assert!(h.preview.chars().count() <= PREVIEW_CHARS);
            assert_eq!(utf16_slice(&h.preview, h.preview_col, h.preview_len), "needle", "{}", h.path);
        }
        let short = hits.iter().find(|h| h.path.ends_with("short.txt")).unwrap();
        assert_eq!((short.preview_col, short.col), (4, 5));
        let long = hits.iter().find(|h| h.path.ends_with("long.txt")).unwrap();
        assert_eq!(long.col, 900 + 1); // 300 * (2 + 1) unità UTF-16
        assert!(long.preview_col < 100);
    }

    #[test]
    fn match_longer_than_window_is_clipped() {
        let d = tempfile::tempdir().unwrap();
        w(d.path(), "a.txt", format!("{}\n", "é".repeat(1000)));
        let (hits, _) = run(d.path(), &SearchQuery { text: "é+".into(), regex: true, ..Default::default() });
        assert_eq!(hits[0].len, 1000);
        assert_eq!(hits[0].preview_col, 0);
        assert_eq!(hits[0].preview_len as usize, PREVIEW_CHARS);
    }

    #[test]
    fn minified_line_is_fast_and_capped_per_line() {
        let d = tempfile::tempdir().unwrap();
        w(d.path(), "min.js", "é;".repeat(600_000) + "\n"); // circa 2 MB su una riga
        w(d.path(), "next.txt", "é\n");
        let t = std::time::Instant::now();
        let (hits, s) = run(d.path(), &q("é"));
        assert!(t.elapsed() < std::time::Duration::from_secs(5), "{:?}", t.elapsed());
        assert_eq!(hits.iter().filter(|h| h.path.ends_with("min.js")).count(), MAX_MATCHES_PER_LINE);
        assert_eq!(hits.iter().filter(|h| h.path.ends_with("next.txt")).count(), 1);
        assert!(!s.truncated);
    }

    #[test]
    fn truncated_is_set_when_the_cap_is_hit_exactly() {
        let d = tempfile::tempdir().unwrap();
        for i in 0..10 {
            w(d.path(), &format!("f{i}.txt"), "a\n".repeat(1000));
        }
        let (hits, s) = run(d.path(), &q("a"));
        assert_eq!((hits.len(), s.truncated), (MAX_HITS, true));
    }

    // ---- sostituzione ----

    fn rq(text: &str, regex: bool) -> SearchQuery {
        SearchQuery { text: text.into(), regex, case_sensitive: true, ..Default::default() }
    }

    fn yes(_: u32, _: u32) -> bool {
        true
    }

    #[test]
    fn replace_rewrites_only_matches() {
        let d = tempfile::tempdir().unwrap();
        w(d.path(), "a.txt", "uno needle\r\ndue\nneedle needle \u{e8}\u{1F600}\nNeedle\n");
        w(d.path(), "b.txt", "niente\n");
        let (a, b) = (d.path().join("a.txt"), d.path().join("b.txt"));
        let asked = std::cell::Cell::new((0, 0));
        let out = replace_in_files(&[a.clone(), b.clone()], &rq("needle", false), "$1 x", &|f, n| {
            asked.set((f, n));
            true
        })
        .unwrap();
        assert_eq!(asked.get(), (1, 3));
        assert_eq!((out.files, out.replacements, out.skipped.len(), out.cancelled), (1, 3, 0, false));
        // restano a capo CRLF, maiuscole non cercate e il resto del testo; in modo letterale `$1` non si espande
        assert_eq!(std::fs::read_to_string(a).unwrap(), "uno $1 x\r\ndue\n$1 x $1 x \u{e8}\u{1F600}\nNeedle\n");
        assert_eq!(std::fs::read_to_string(b).unwrap(), "niente\n");
    }

    #[test]
    fn replace_with_regex_groups() {
        let d = tempfile::tempdir().unwrap();
        w(d.path(), "a.txt", "fn foo(a)\nfn bar(b)\n");
        let a = d.path().join("a.txt");
        let out = replace_in_files(std::slice::from_ref(&a), &rq(r"fn (\w+)\((\w)\)", true), "$2 <- ${1}()", &yes).unwrap();
        assert_eq!(out.replacements, 2);
        assert_eq!(std::fs::read_to_string(a).unwrap(), "a <- foo()\nb <- bar()\n");
    }

    #[test]
    fn replace_ignores_empty_matches_and_keeps_lines_apart() {
        let d = tempfile::tempdir().unwrap();
        w(d.path(), "a.txt", "ab\nb\n");
        let a = d.path().join("a.txt");
        let out = replace_in_files(std::slice::from_ref(&a), &rq("x*", true), "Z", &yes).unwrap();
        assert_eq!((out.files, out.replacements), (0, 0));
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "ab\nb\n");
        // `\s+` non attraversa gli a capo, come nella ricerca
        w(d.path(), "c.txt", "a \n b\n");
        let c = d.path().join("c.txt");
        replace_in_files(std::slice::from_ref(&c), &rq(r"\s+", true), "_", &yes).unwrap();
        assert_eq!(std::fs::read_to_string(c).unwrap(), "a_\n_b\n");
    }

    #[test]
    fn replace_cancelled_touches_nothing() {
        let d = tempfile::tempdir().unwrap();
        w(d.path(), "a.txt", "needle\n");
        let a = d.path().join("a.txt");
        let out = replace_in_files(std::slice::from_ref(&a), &rq("needle", false), "x", &|_, _| false).unwrap();
        assert!(out.cancelled);
        assert_eq!(std::fs::read_to_string(a).unwrap(), "needle\n");
    }

    #[test]
    fn replace_skips_non_text_and_files_changed_since() {
        let d = tempfile::tempdir().unwrap();
        w(d.path(), "bin.dat", b"needle\0\xff");
        w(d.path(), "latin.txt", b"needle \xe8\n");
        w(d.path(), "a.txt", "needle\n");
        let paths: Vec<PathBuf> = ["bin.dat", "latin.txt", "a.txt"].iter().map(|n| d.path().join(n)).collect();
        // il file cambia mentre la conferma è aperta: la scrittura è rifiutata dal controllo dell'mtime
        let a = paths[2].clone();
        let out = replace_in_files(&paths, &rq("needle", false), "x", &|_, _| {
            std::thread::sleep(std::time::Duration::from_millis(20));
            std::fs::write(&a, "needle altro\n").unwrap();
            let t = std::fs::metadata(&a).unwrap().modified().unwrap() + std::time::Duration::from_secs(5);
            std::fs::File::options().write(true).open(&a).unwrap().set_modified(t).unwrap();
            true
        })
        .unwrap();
        assert_eq!((out.files, out.replacements), (0, 0));
        assert_eq!(out.skipped.len(), 3);
        assert_eq!(std::fs::read_to_string(a).unwrap(), "needle altro\n");
        assert_eq!(std::fs::read(d.path().join("latin.txt")).unwrap(), b"needle \xe8\n");
    }

    #[test]
    fn replace_reports_read_only_file_as_skipped() {
        use std::os::unix::fs::PermissionsExt;
        let d = tempfile::tempdir().unwrap();
        w(d.path(), "ro.txt", "needle\n");
        w(d.path(), "ok.txt", "needle\n");
        let ro = d.path().join("ro.txt");
        std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o444)).unwrap();
        let out = replace_in_files(&[ro.clone(), d.path().join("ok.txt")], &rq("needle", false), "x", &yes).unwrap();
        assert_eq!((out.files, out.replacements), (1, 1));
        assert_eq!(out.skipped, vec![nfc(&ro)]);
        assert_eq!(std::fs::read_to_string(ro).unwrap(), "needle\n");
    }

    #[test]
    fn replace_refused_when_untrusted_or_outside_project() {
        use crate::projects::PROJECT_PREFIX;
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().canonicalize().unwrap();
        let (proj, outside) = (base.join("progetto"), base.join("fuori"));
        std::fs::create_dir_all(&proj).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        w(&proj, "a.txt", "needle\n");
        w(&outside, "s.txt", "needle\n");
        let store = Store::open(&base.join("t.db")).unwrap();
        let folder = proj.to_string_lossy().into_owned();
        store.upsert_project(&crate::store::Project { id: "p1".into(), name: "p".into(), folders: vec![folder.clone()] }).unwrap();
        let label = format!("{PROJECT_PREFIX}p1");
        let inside = vec![proj.join("a.txt").to_string_lossy().into_owned()];
        assert_eq!(replace_targets(&store, &label, &inside).unwrap_err(), crate::trust::UNTRUSTED);
        crate::trust::mark_trusted(&store, &folder).unwrap();
        assert_eq!(replace_targets(&store, &label, &inside).unwrap().len(), 1);
        assert!(replace_targets(&store, &label, &[outside.join("s.txt").to_string_lossy().into_owned()]).is_err());
        assert!(replace_targets(&store, "main", &inside).is_err());
    }

    #[test]
    fn replace_message_is_singular_or_plural() {
        assert_eq!(replace_message(1, 1), "Sostituire 1 occorrenza in 1 file?");
        assert_eq!(replace_message(3, 12), "Sostituire 12 occorrenze in 3 file?");
    }
}
