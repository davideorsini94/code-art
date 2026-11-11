//! Lettura delle sessioni Claude Code già presenti su disco (`~/.claude/projects`, solo lettura)
//! e sincronizzazione nello store. Formato: docs/claude-protocol.md §11.
use super::{ChatItem, Conversation};
use crate::store::{Project, Result, Store};
use serde_json::Value;
use std::collections::HashMap;
use std::fs::File;
use std::io::{self, BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

fn err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

/// Righe più lunghe vengono lette solo in parte (un'immagine base64 inline può pesare megabyte).
const META_LINE_CAP: usize = 256 * 1024;
const ITEMS_LINE_CAP: usize = 4 * 1024 * 1024;
/// `conv_history` restituisce al massimo questi ultimi elementi.
pub const HISTORY_MAX_ITEMS: usize = 2000;
const TOOL_RESULT_CAP: usize = 20_000;
const TITLE_MAX_CHARS: usize = 60;
const ELSEWHERE_WINDOW_MS: i64 = 60_000;

// ---------- percorsi ----------

/// Codifica del cwd usata dalla CLI come nome di cartella (§11.1). Il chiamante passa il percorso reale.
pub fn encode_project_dir(path: &str) -> String {
    let units: Vec<u16> = path.encode_utf16().collect();
    let s: String = units
        .iter()
        .map(|&u| if u < 128 && (u as u8).is_ascii_alphanumeric() { u as u8 as char } else { '-' })
        .collect();
    if s.len() <= 200 {
        return s;
    }
    let mut h: i32 = 0;
    for &u in &units {
        h = h.wrapping_mul(31).wrapping_add(u as i32);
    }
    format!("{}-{}", &s[..200], to_base36((h as i64).unsigned_abs()))
}

fn to_base36(mut n: u64) -> String {
    if n == 0 {
        return "0".into();
    }
    let mut out = Vec::new();
    while n > 0 {
        out.push(b"0123456789abcdefghijklmnopqrstuvwxyz"[(n % 36) as usize]);
        n /= 36;
    }
    out.reverse();
    String::from_utf8(out).unwrap_or_default()
}

/// `~/.claude/projects`.
pub fn default_root() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default().join(".claude/projects")
}

/// Cartelle di sessione esistenti sotto `root` (default: `default_root()`) per le cartelle del progetto
/// (percorso dato e percorso reale, in NFC come richiede §11.1).
pub fn session_dirs_for(root: &Path, folders: &[String]) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for f in folders {
        let mut names = vec![encode_project_dir(&nfc(f))];
        if let Ok(real) = Path::new(f).canonicalize() {
            names.push(encode_project_dir(&nfc(&real.to_string_lossy())));
        }
        for n in names {
            let d = root.join(n);
            if d.is_dir() && !out.contains(&d) {
                out.push(d);
            }
        }
    }
    out
}

fn normalize(p: &str) -> String {
    let t = p.trim_end_matches('/');
    if t.is_empty() { "/".into() } else { t.into() }
}

fn nfc(s: &str) -> String {
    use unicode_normalization::UnicodeNormalization;
    s.nfc().collect()
}

/// Confronto in NFC su entrambi i lati (il cwd registrato dalla CLI è NFC, la cartella scelta può essere NFD).
fn cwd_in_project(cwd: &str, folders: &[String]) -> bool {
    let c = nfc(&normalize(cwd));
    let real = Path::new(cwd).canonicalize().ok().map(|p| nfc(&p.to_string_lossy()));
    folders.iter().any(|f| {
        let f = nfc(&normalize(f));
        f == c
            || real.as_deref().is_some_and(|r| {
                r == f || Path::new(&f).canonicalize().is_ok_and(|rf| nfc(&rf.to_string_lossy()) == r)
            })
    })
}

// ---------- lettura a righe con tetto ----------

/// Chiama `f(riga, troncata, terminata)` per ogni riga tenendo in memoria al massimo `cap` byte per riga
/// (`terminata` = finisce con `\n`: l'ultima riga può essere ancora in scrittura).
/// Restituisce i byte letti fino all'ultimo `\n` incluso.
fn for_each_line<R: BufRead>(r: R, cap: usize, mut f: impl FnMut(&[u8], bool, bool)) -> io::Result<u64> {
    for_each_line_until(r, cap, |l, t, e| {
        f(l, t, e);
        std::ops::ControlFlow::Continue(())
    })
}

/// Come `for_each_line`, ma `f` può fermare la lettura restituendo `Break`.
fn for_each_line_until<R: BufRead>(
    mut r: R,
    cap: usize,
    mut f: impl FnMut(&[u8], bool, bool) -> std::ops::ControlFlow<()>,
) -> io::Result<u64> {
    let mut buf: Vec<u8> = Vec::new();
    let mut consumed_terminated: u64 = 0;
    loop {
        let mut line_bytes: u64 = 0;
        buf.clear();
        let mut truncated = false;
        let mut any = false;
        let mut terminated = false;
        loop {
            let chunk = r.fill_buf()?;
            if chunk.is_empty() {
                break;
            }
            any = true;
            let (take, found) = match chunk.iter().position(|&b| b == b'\n') {
                Some(i) => (i, true),
                None => (chunk.len(), false),
            };
            let room = cap.saturating_sub(buf.len());
            buf.extend_from_slice(&chunk[..take.min(room)]);
            if take > room {
                truncated = true;
            }
            r.consume(take + usize::from(found));
            line_bytes += (take + usize::from(found)) as u64;
            terminated = found;
            if found {
                break;
            }
        }
        if !any {
            return Ok(consumed_terminated);
        }
        if terminated {
            consumed_terminated += line_bytes;
        }
        if f(&buf, truncated, terminated).is_break() {
            return Ok(consumed_terminated);
        }
    }
}

fn open_reader(file: &Path) -> io::Result<BufReader<File>> {
    Ok(BufReader::with_capacity(64 * 1024, File::open(file)?))
}

// ---------- classificazione righe ----------

fn has_tool_result(v: &Value) -> bool {
    v["message"]["content"]
        .as_array()
        .is_some_and(|a| a.iter().any(|b| b["type"] == "tool_result"))
}

/// Testo di un messaggio utente (stringa o blocchi `text`) e numero di immagini.
fn user_text(v: &Value) -> (String, usize) {
    match &v["message"]["content"] {
        Value::String(s) => (s.clone(), 0),
        Value::Array(blocks) => {
            let mut texts = Vec::new();
            let mut images = 0;
            for b in blocks {
                match b["type"].as_str() {
                    Some("text") => texts.push(b["text"].as_str().unwrap_or("")),
                    Some("image") => images += 1,
                    _ => {}
                }
            }
            (texts.join("\n"), images)
        }
        _ => (String::new(), 0),
    }
}

fn is_synthetic_text(t: &str) -> bool {
    let t = t.trim_start();
    t.starts_with("<local-command-")
        || t.starts_with("<command-name>")
        || t.starts_with("<task-notification>")
        || t.starts_with("[Request interrupted")
}

/// Prompt scritto dalla persona (§11.3). `promptSource` è una lista di esclusione (solo `system`) e non la
/// whitelist del documento (`sdk`/assente): i prompt di sessioni interattive potrebbero avere altri valori.
///: `Some((testo, immagini))`.
fn human_prompt(v: &Value) -> Option<(String, usize)> {
    if v["type"] != "user"
        || v["isSidechain"] == true
        || v["isMeta"] == true
        || v["isCompactSummary"] == true
        || v["promptSource"] == "system"
        || v["origin"]["kind"] == "task-notification"
        || has_tool_result(v)
    {
        return None;
    }
    let (text, images) = user_text(v);
    if is_synthetic_text(&text) || (text.trim().is_empty() && images == 0) {
        return None;
    }
    Some((text, images))
}

/// Per le righe troncate (solo prefisso): stima prudente "è un prompt umano con allegati".
fn sniff_truncated_human(prefix: &[u8]) -> bool {
    let s = String::from_utf8_lossy(prefix);
    s.contains("\"type\":\"user\"")
        && !s.contains("\"tool_result\"")
        && !s.contains("\"isMeta\":true")
        && !s.contains("\"isSidechain\":true")
        && !s.contains("\"toolUseResult\"")
        && !s.contains("\"isCompactSummary\":true")
}

fn extract_str<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let i = line.find(key)? + key.len();
    let rest = &line[i..];
    Some(&rest[..rest.find('"')?])
}

/// ISO-8601 UTC con millisecondi opzionali → ms da epoch.
pub fn parse_iso_ms(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() < 19 || b[4] != b'-' || b[7] != b'-' || (b[10] != b'T' && b[10] != b' ') {
        return None;
    }
    let num = |a: usize, z: usize| s.get(a..z)?.parse::<i64>().ok();
    let (y, m, d) = (num(0, 4)?, num(5, 7)?, num(8, 10)?);
    let (hh, mm, ss) = (num(11, 13)?, num(14, 16)?, num(17, 19)?);
    let mut ms = 0;
    if b.len() > 20 && b[19] == b'.' {
        let digits: String = s[20..].chars().take_while(|c| c.is_ascii_digit()).take(3).collect();
        ms = format!("{digits:0<3}").parse::<i64>().ok()?;
    }
    // giorni dall'epoch (algoritmo civil-from-days di H. Hinnant)
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(((days * 24 + hh) * 60 + mm) * 60_000 + ss * 1000 + ms)
}

fn mtime_ms(meta: &std::fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

// ---------- metadati ----------

#[derive(Debug, Clone, PartialEq)]
pub struct SessionMeta {
    pub session_id: String,
    pub cwd: String,
    pub first_prompt: String,
    pub last_activity_ms: i64,
    pub turn_count: i64,
    pub custom_title: Option<String>,
    pub mtime_ms: i64,
}

fn empty_meta(file: &Path) -> SessionMeta {
    SessionMeta {
        session_id: file.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default(),
        cwd: String::new(),
        first_prompt: String::new(),
        last_activity_ms: 0,
        turn_count: 0,
        custom_title: None,
        mtime_ms: 0,
    }
}

/// Legge `file` da `offset`, aggiornando `out`; consuma solo righe complete (terminate da `\n`).
/// Restituisce l'offset assoluto fino a cui ha letto. Righe malformate o di tipo ignoto: saltate.
fn scan_meta(file: &Path, offset: u64, out: &mut SessionMeta) -> Result<u64> {
    use std::io::Seek;
    let mut f = File::open(file).map_err(err)?;
    f.seek(io::SeekFrom::Start(offset)).map_err(err)?;
    let note_ts = |out: &mut SessionMeta, ts: Option<&str>| {
        if let Some(ms) = ts.and_then(parse_iso_ms) {
            out.last_activity_ms = out.last_activity_ms.max(ms);
        }
    };
    let n = for_each_line(BufReader::with_capacity(64 * 1024, f), META_LINE_CAP, |line, truncated, terminated| {
        if !terminated {
            return; // riga ancora in scrittura: non consumata, verrà riletta
        }
        if truncated {
            if sniff_truncated_human(line) {
                out.turn_count += 1;
            }
            return;
        }
        let Ok(text) = std::str::from_utf8(line) else { return };
        // le righe che non servono (assistant, allegati, snapshot…) non si parsano: solo timestamp e cwd
        let interesting = text.contains("\"type\":\"user\"") || text.contains("\"type\":\"custom-title\"");
        if !interesting {
            if text.contains("\"isSidechain\":true") {
                return;
            }
            note_ts(out, extract_str(text, "\"timestamp\":\""));
            if out.cwd.is_empty() {
                if let Some(c) = extract_str(text, "\"cwd\":\"") {
                    if !c.contains('\\') {
                        out.cwd = c.to_string();
                    }
                }
            }
            return;
        }
        let Ok(v) = serde_json::from_str::<Value>(text) else { return };
        if v["type"] == "custom-title" {
            if let Some(t) = v["customTitle"].as_str().filter(|t| !t.trim().is_empty()) {
                out.custom_title = Some(t.to_string());
            }
            return;
        }
        if v["isSidechain"] == true {
            return;
        }
        if out.cwd.is_empty() {
            if let Some(c) = v["cwd"].as_str() {
                out.cwd = c.to_string();
            }
        }
        note_ts(out, v["timestamp"].as_str());
        if let Some((t, _)) = human_prompt(&v) {
            out.turn_count += 1;
            if out.first_prompt.is_empty() {
                out.first_prompt = t.chars().take(200).collect();
            }
        }
    })
    .map_err(err)?;
    Ok(offset + n)
}

/// Lettura completa e senza cache (riferimento per i test della cache).
#[cfg(test)]
pub fn read_session_meta(file: &Path) -> Result<SessionMeta> {
    let md = std::fs::metadata(file).map_err(err)?;
    let mut out = empty_meta(file);
    scan_meta(file, 0, &mut out)?;
    out.mtime_ms = mtime_ms(&md);
    if out.last_activity_ms == 0 {
        out.last_activity_ms = out.mtime_ms;
    }
    Ok(out)
}

struct CacheEntry {
    size: u64,
    mtime_ms: i64,
    /// byte già consumati (fino all'ultimo `\n`)
    offset: u64,
    /// metadati grezzi (last_activity senza il ripiego sull'mtime)
    meta: SessionMeta,
}

/// Cache per file di sessione: un file invariato non viene riletto; uno cresciuto viene letto solo
/// dai byte aggiunti; uno rimpicciolito o con mtime indietro viene riletto per intero.
#[derive(Default)]
pub struct MetaCache {
    entries: Mutex<HashMap<PathBuf, CacheEntry>>,
    pub full_reads: AtomicUsize,
    pub incremental_reads: AtomicUsize,
}

impl MetaCache {
    /// Dimentica un file (conversazione eliminata): se ricomparisse sarebbe riletto per intero.
    pub fn forget(&self, file: &Path) {
        self.entries.lock().unwrap_or_else(|p| p.into_inner()).remove(file);
    }

    pub fn read(&self, file: &Path) -> Result<SessionMeta> {
        let md = std::fs::metadata(file).map_err(err)?;
        let (size, mtime) = (md.len(), mtime_ms(&md));
        let mut map = self.entries.lock().unwrap_or_else(|p| p.into_inner());
        let finish = |e: &CacheEntry| {
            let mut m = e.meta.clone();
            m.mtime_ms = e.mtime_ms;
            if m.last_activity_ms == 0 {
                m.last_activity_ms = m.mtime_ms;
            }
            m
        };
        if let Some(e) = map.get_mut(file) {
            if e.size == size && e.mtime_ms == mtime {
                return Ok(finish(e));
            }
            if size > e.size && mtime >= e.mtime_ms {
                let mut meta = e.meta.clone();
                let offset = scan_meta(file, e.offset, &mut meta)?;
                self.incremental_reads.fetch_add(1, Ordering::Relaxed);
                *e = CacheEntry { size, mtime_ms: mtime, offset, meta };
                return Ok(finish(e));
            }
        }
        let mut meta = empty_meta(file);
        let offset = scan_meta(file, 0, &mut meta)?;
        self.full_reads.fetch_add(1, Ordering::Relaxed);
        let e = CacheEntry { size, mtime_ms: mtime, offset, meta };
        let out = finish(&e);
        map.insert(file.to_path_buf(), e);
        Ok(out)
    }
}

// ---------- elementi della chat ----------

fn tool_file_path(input: &Value) -> Option<String> {
    ["file_path", "path", "notebook_path"].iter().find_map(|k| input[*k].as_str().map(String::from))
}

fn tool_result_text(content: &Value) -> String {
    let s = match content {
        Value::String(s) => s.clone(),
        Value::Array(a) => a.iter().filter_map(|b| b["text"].as_str()).collect::<Vec<_>>().join("\n"),
        _ => String::new(),
    };
    if s.len() > TOOL_RESULT_CAP {
        let mut end = TOOL_RESULT_CAP;
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}…", &s[..end])
    } else {
        s
    }
}

/// Ricostruisce la chat in ordine di file (§11.5). Tipi sconosciuti → nessun elemento. Sidechain escluse.
/// Restituisce al massimo gli ultimi `HISTORY_MAX_ITEMS` elementi.
pub fn read_session_items(file: &Path) -> Result<Vec<ChatItem>> {
    let mut items: Vec<ChatItem> = Vec::new();
    let mut tool_index: HashMap<String, usize> = HashMap::new();
    for_each_line(open_reader(file).map_err(err)?, ITEMS_LINE_CAP, |line, truncated, _terminated| {
        if truncated {
            return;
        }
        let Ok(v) = serde_json::from_slice::<Value>(line) else { return };
        if v["isSidechain"] == true {
            return;
        }
        let uuid = v["uuid"].as_str().unwrap_or("").to_string();
        match v["type"].as_str() {
            Some("user") => push_user(&v, &uuid, &mut items, &mut tool_index),
            Some("assistant") => {
                let Some(blocks) = v["message"]["content"].as_array() else { return };
                for (i, b) in blocks.iter().enumerate() {
                    let id = if blocks.len() == 1 { uuid.clone() } else { format!("{uuid}:{i}") };
                    match b["type"].as_str() {
                        Some("text") => {
                            let t = b["text"].as_str().unwrap_or("");
                            if !t.is_empty() {
                                items.push(ChatItem::Assistant { id, text: t.to_string() });
                            }
                        }
                        Some("thinking") => {
                            let t = b["thinking"].as_str().unwrap_or("");
                            if !t.is_empty() {
                                items.push(ChatItem::Thinking { id, text: t.to_string() });
                            }
                        }
                        Some("tool_use") => {
                            let tid = b["id"].as_str().unwrap_or("").to_string();
                            tool_index.insert(tid.clone(), items.len());
                            items.push(ChatItem::Tool {
                                id: tid,
                                name: b["name"].as_str().unwrap_or("").to_string(),
                                file_path: tool_file_path(&b["input"]),
                                input: b["input"].clone(),
                                result: None,
                                ok: None,
                                parent_id: None,
                            });
                        }
                        _ => {}
                    }
                }
            }
            Some("system") => match v["subtype"].as_str() {
                Some(sub @ ("compact_boundary" | "local_command")) => {
                    let label = if sub == "compact_boundary" { "compact" } else { "local-command" };
                    items.push(ChatItem::Raw {
                        id: uuid,
                        label: label.into(),
                        json: v["content"].as_str().unwrap_or("").to_string(),
                    });
                }
                _ => {}
            },
            _ => {}
        }
    })
    .map_err(err)?;
    if items.len() > HISTORY_MAX_ITEMS {
        items.drain(..items.len() - HISTORY_MAX_ITEMS);
    }
    Ok(items)
}

fn push_user(v: &Value, uuid: &str, items: &mut Vec<ChatItem>, tool_index: &mut HashMap<String, usize>) {
    if has_tool_result(v) {
        for b in v["message"]["content"].as_array().into_iter().flatten() {
            if b["type"] != "tool_result" {
                continue;
            }
            let Some(&idx) = b["tool_use_id"].as_str().and_then(|t| tool_index.get(t)) else { continue };
            if let ChatItem::Tool { result, ok, .. } = &mut items[idx] {
                *result = Some(tool_result_text(&b["content"]));
                *ok = Some(b["is_error"] != true);
            }
        }
        return;
    }
    if let Some((text, _images)) = human_prompt(v) {
        // le immagini non vengono incorporate (base64): `images` resta vuoto
        items.push(ChatItem::User { id: uuid.to_string(), text, images: vec![] });
        return;
    }
    let (text, _) = user_text(v);
    let t = text.trim_start();
    if v["isMeta"] != true && v["isCompactSummary"] != true {
        if let Some(rest) = t.strip_prefix("<command-name>") {
            if let Some(name) = rest.split("</command-name>").next() {
                items.push(ChatItem::User { id: uuid.to_string(), text: name.trim().to_string(), images: vec![] });
            }
        } else if t.starts_with("<local-command-stdout>") {
            items.push(ChatItem::Raw { id: uuid.to_string(), label: "local-command".into(), json: text });
        }
    }
}

// ---------- sincronizzazione ----------

fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

fn provisional_title(meta: &SessionMeta) -> String {
    let src = meta.custom_title.as_deref().unwrap_or(&meta.first_prompt);
    src.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(TITLE_MAX_CHARS).collect()
}

/// Sincronizza un file di sessione; `None` se non appartiene al progetto (cwd estraneo) o non è leggibile.
pub fn sync_session_file(store: &Store, cache: &MetaCache, project: &Project, file: &Path) -> Result<Option<SessionMeta>> {
    let meta = cache.read(file)?;
    if meta.cwd.is_empty() || !cwd_in_project(&meta.cwd, &project.folders) {
        return Ok(None);
    }
    store.upsert_synced_conversation(
        &project.id,
        &meta.session_id,
        &meta.cwd,
        &provisional_title(&meta),
        meta.last_activity_ms,
        meta.turn_count,
    )?;
    Ok(Some(meta))
}

fn is_session_file(p: &Path) -> bool {
    p.extension().is_some_and(|e| e == "jsonl") && p.file_stem().is_some_and(|s| valid_session_id(&s.to_string_lossy()))
}

/// Un id di sessione finisce in un percorso: solo caratteri sicuri.
pub fn valid_session_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// `is_running(session_id)`: la conversazione è pilotata da un processo del motore (vivo, o uscito da meno
/// di 60 s: le scritture recenti sul file sono nostre). Vedi `Engine::is_ours`.
pub type IsRunning<'a> = &'a dyn Fn(&str) -> bool;

/// Nomi dei file di una cartella; `None` se anche una sola voce non si è potuta leggere (mai "quasi tutto").
fn list_names(dir: &Path) -> Option<HashSet<String>> {
    let mut names = HashSet::new();
    for entry in std::fs::read_dir(dir).ok()? {
        names.insert(entry.ok()?.file_name().to_string_lossy().into_owned());
    }
    Some(names)
}

/// Elenchi riusciti in questo passaggio (cartella → nomi); le cartelle non leggibili non compaiono.
type Listings = HashMap<PathBuf, HashSet<String>>;

pub fn sync_project_sessions(
    root: &Path,
    store: &Store,
    cache: &MetaCache,
    project: &Project,
    now: i64,
    is_running: IsRunning,
) -> Result<Vec<Conversation>> {
    let mut listings = Listings::new();
    for dir in session_dirs_for(root, &project.folders) {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        let mut names = Some(HashSet::new());
        for entry in rd {
            let Ok(entry) = entry else {
                names = None; // elenco incompleto: non prova nulla
                continue;
            };
            if let Some(n) = names.as_mut() {
                n.insert(entry.file_name().to_string_lossy().into_owned());
            }
            let p = entry.path();
            if is_session_file(&p) && p.is_file() {
                let _ = sync_session_file(store, cache, project, &p); // un file illeggibile non blocca gli altri
            }
        }
        if let Some(n) = names {
            listings.insert(dir, n);
        }
    }
    prune_vanished(root, store, project, is_running, &mut listings);
    conversations_for(root, store, project, now, is_running)
}

/// Toglie dallo store le conversazioni il cui file di sessione è PROVATO sparito (eliminate altrove o dopo un
/// fallimento di `conv_delete`). Una riga si scarta solo se: non è pilotata dal motore e ha già dei turni; ogni
/// cartella di sessione in cui potrebbe stare (quelle del progetto e quella del suo cwd) è stata elencata con
/// successo (in questo passaggio) e non contiene il file; `try_exists` sul percorso atteso dà `Ok(false)`. Qualsiasi
/// errore lascia la riga. Se un passaggio scartasse TUTTE le righe di un progetto con più di 3 righe è sospetto
/// (cartella sparita, permessi, volume smontato): non si scarta nulla.
fn prune_vanished(root: &Path, store: &Store, project: &Project, is_running: IsRunning, listings: &mut Listings) {
    if !root.is_dir() {
        return;
    }
    let project_dirs = session_dirs_for(root, &project.folders);
    let Ok(list) = store.list_conversations(&project.id) else { return };
    let mut gone = Vec::new();
    for c in &list {
        if c.turn_count == 0 || !valid_session_id(&c.session_id) || is_running(&c.session_id) {
            continue;
        }
        let name = format!("{}.jsonl", c.session_id);
        let mut dirs = project_dirs.clone();
        for d in session_dirs_for(root, std::slice::from_ref(&c.cwd)) {
            if !dirs.contains(&d) {
                dirs.push(d);
            }
        }
        let proven = !dirs.is_empty()
            && dirs.iter().all(|d| {
                if !listings.contains_key(d) {
                    if let Some(n) = list_names(d) {
                        listings.insert(d.clone(), n);
                    }
                }
                listings.get(d).is_some_and(|n| !n.contains(&name)) && matches!(d.join(&name).try_exists(), Ok(false))
            });
        if proven {
            gone.push(c.session_id.clone());
        }
    }
    if list.len() > 3 && gone.len() == list.len() {
        return;
    }
    for id in gone {
        let _ = store.delete_conversation(&id);
    }
}

/// Conversazioni salvate del progetto con `maybe_open_elsewhere` calcolato dall'mtime del file.
pub fn conversations_for(root: &Path, store: &Store, project: &Project, now: i64, is_running: IsRunning) -> Result<Vec<Conversation>> {
    let dirs = session_dirs_for(root, &project.folders);
    let mut list = store.list_conversations(&project.id)?;
    for c in &mut list {
        fill_elsewhere(c, &dirs, now, is_running(&c.session_id));
    }
    Ok(list)
}

/// Avviso "forse aperta altrove": file scritto negli ultimi 60 s e conversazione non pilotata adesso dal
/// motore. Una conversazione `owned` ma senza processo (dopo un riavvio dell'app) può essere proseguita
/// altrove, quindi conta il processo vivo e non solo `owned`.
pub fn fill_elsewhere(c: &mut Conversation, dirs: &[PathBuf], now: i64, running: bool) {
    c.maybe_open_elsewhere = !running
        && dirs
            .iter()
            .filter_map(|d| std::fs::metadata(d.join(format!("{}.jsonl", c.session_id))).ok())
            .any(|m| now - mtime_ms(&m) < ELSEWHERE_WINDOW_MS);
}

/// Storia di una sessione del progetto: rifiuta ids sconosciuti, di altri progetti o con caratteri strani.
pub fn history_in(root: &Path, store: &Store, project: &Project, session_id: &str) -> Result<Vec<ChatItem>> {
    if !valid_session_id(session_id) {
        return Err("sessione non valida".into());
    }
    match store.get_conversation(session_id)? {
        Some(c) if c.project_id == project.id => {}
        _ => return Err("sessione non appartiene a questo progetto".into()),
    }
    for dir in session_dirs_for(root, &project.folders) {
        let f = dir.join(format!("{session_id}.jsonl"));
        if f.is_file() {
            return read_session_items(&f);
        }
    }
    Err("file della sessione non trovato".into())
}

// ---------- ricerca nel testo delle conversazioni ----------

pub const SEARCH_MAX_RESULTS: usize = 30;
const SEARCH_MIN_CHARS: usize = 2;
const SEARCH_MAX_CHARS: usize = 200;
const SEARCH_FILE_BYTES: u64 = 20 * 1024 * 1024;
const SEARCH_BUDGET: Duration = Duration::from_secs(3);
const SNIPPET_CHARS: usize = 80;
const SNIPPET_BEFORE: usize = 25;

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    pub session_id: String,
    pub snippet: String,
}

/// Minuscola carattere per carattere (1:1, così gli indici restano allineati al testo originale).
fn fold(s: &str) -> Vec<char> {
    s.chars().map(|c| c.to_lowercase().next().unwrap_or(c)).collect()
}

/// Primo `needle` (già in `fold`) dentro `text`; ritorna lo snippet attorno alla corrispondenza.
fn snippet_of(text: &str, needle: &[char]) -> Option<String> {
    let chars: Vec<char> = text.chars().collect();
    let hay = fold(text);
    if needle.is_empty() || hay.len() < needle.len() {
        return None;
    }
    let at = (0..=hay.len() - needle.len()).find(|&i| hay[i] == needle[0] && hay[i..i + needle.len()] == *needle)?;
    let start = at.saturating_sub(SNIPPET_BEFORE);
    let end = chars.len().min((start + SNIPPET_CHARS).max(at + needle.len()));
    let body: String = chars[start..end].iter().map(|&c| if c.is_control() { ' ' } else { c }).collect();
    Some(format!("{}{}{}", if start > 0 { "…" } else { "" }, body.trim(), if end < chars.len() { "…" } else { "" }))
}

/// Testi scritti da persona e assistente in una riga del file di sessione (sidechain escluse).
fn message_texts(v: &Value) -> Vec<String> {
    if v["isSidechain"] == true {
        return vec![];
    }
    match v["type"].as_str() {
        Some("user") => human_prompt(v).map(|(t, _)| t).into_iter().collect(),
        Some("assistant") => v["message"]["content"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|b| b["type"] == "text")
            .filter_map(|b| b["text"].as_str().map(String::from))
            .collect(),
        _ => vec![],
    }
}

/// Prima corrispondenza nel file, letto a righe; si ferma dopo 20 MB o alla scadenza.
fn search_file(file: &Path, needle: &[char], deadline: Instant) -> Option<String> {
    use std::io::Read;
    let reader = BufReader::with_capacity(64 * 1024, File::open(file).ok()?.take(SEARCH_FILE_BYTES));
    let mut found: Option<String> = None;
    let _ = for_each_line_until(reader, ITEMS_LINE_CAP, |line, truncated, _| {
        if Instant::now() >= deadline {
            return std::ops::ControlFlow::Break(());
        }
        if truncated {
            return std::ops::ControlFlow::Continue(());
        }
        let Ok(v) = serde_json::from_slice::<Value>(line) else { return std::ops::ControlFlow::Continue(()) };
        found = message_texts(&v).iter().find_map(|t| snippet_of(t, needle));
        if found.is_some() { std::ops::ControlFlow::Break(()) } else { std::ops::ControlFlow::Continue(()) }
    });
    found
}

/// Ricerca testuale (senza regex, senza distinzione di maiuscole) nei messaggi delle sessioni del progetto,
/// comprese le archiviate. Al massimo una corrispondenza per sessione, `SEARCH_MAX_RESULTS` in tutto.
pub fn search_in(root: &Path, store: &Store, project: &Project, query: &str, budget: Duration) -> Result<Vec<SearchHit>> {
    let query = query.trim();
    let n = query.chars().count();
    if !(SEARCH_MIN_CHARS..=SEARCH_MAX_CHARS).contains(&n) {
        return Ok(vec![]);
    }
    let needle = fold(query);
    let deadline = Instant::now() + budget;
    let dirs = session_dirs_for(root, &project.folders);
    let mut hits = Vec::new();
    for c in store.list_conversations(&project.id)? {
        if hits.len() >= SEARCH_MAX_RESULTS || Instant::now() >= deadline {
            break;
        }
        if !valid_session_id(&c.session_id) {
            continue;
        }
        let Some(file) = dirs.iter().map(|d| d.join(format!("{}.jsonl", c.session_id))).find(|f| f.is_file()) else { continue };
        if let Some(snippet) = search_file(&file, &needle, deadline) {
            hits.push(SearchHit { session_id: c.session_id, snippet });
        }
    }
    Ok(hits)
}

// ---------- watcher ----------

use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::HashSet;
use std::sync::mpsc::{channel, RecvTimeoutError};
use super::engine::Engine;
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};

const DEBOUNCE: Duration = Duration::from_millis(150);

/// Un watcher per progetto aperto (chiave: id progetto).
#[derive(Default)]
pub struct SessionWatchers(Mutex<HashMap<String, RecommendedWatcher>>);

/// Osserva `~/.claude/projects` (la cartella del progetto può nascere dopo) e, per ogni file di
/// sessione toccato nelle cartelle del progetto, risincronizza ed emette `conv:upsert` alla sua finestra.
pub fn watch_project(app: &AppHandle, project: &Project) {
    let root = default_root();
    if !root.is_dir() {
        return;
    }
    let root = root.canonicalize().unwrap_or(root);
    let (tx, rx) = channel::<PathBuf>();
    let Ok(mut watcher) = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if let Ok(ev) = res {
            for p in ev.paths {
                let _ = tx.send(p);
            }
        }
    }) else {
        return;
    };
    if watcher.watch(&root, RecursiveMode::Recursive).is_err() {
        return;
    }
    let label = crate::projects::window_label(&project.id);
    let (handle, project_id) = (app.clone(), project.id.clone());
    std::thread::spawn(move || {
        while let Ok(first) = rx.recv() {
            let mut paths = vec![first];
            let deadline = std::time::Instant::now() + DEBOUNCE;
            loop {
                match rx.recv_timeout(deadline.saturating_duration_since(std::time::Instant::now())) {
                    Ok(p) => paths.push(p),
                    Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => break,
                }
            }
            // il progetto può essere cambiato (cartelle aggiunte): rileggerlo ogni volta
            let store = handle.state::<Store>();
            let Ok(Some(project)) = store.get_project(&project_id) else { continue };
            let dirs: Vec<PathBuf> = session_dirs_for(&root, &project.folders)
                .into_iter()
                .map(|d| d.canonicalize().unwrap_or(d))
                .collect();
            let mut seen = HashSet::new();
            for p in paths {
                if !is_session_file(&p) || !p.parent().is_some_and(|d| dirs.iter().any(|x| x == d)) || !seen.insert(p.clone()) {
                    continue;
                }
                if !p.is_file() {
                    continue;
                }
                let before = p
                    .file_stem()
                    .and_then(|s| store.get_conversation(&s.to_string_lossy()).ok().flatten())
                    .map(|c| c.turn_count);
                if let Ok(Some(meta)) = sync_session_file(&store, &handle.state::<MetaCache>(), &project, &p) {
                    if let Ok(Some(mut conv)) = store.get_conversation(&meta.session_id) {
                        if let Some(q) = handle.try_state::<super::titles::TitleQueue>() {
                            q.after_file_sync(&conv, before);
                        }
                        let running = handle.try_state::<Engine>().is_some_and(|e| e.is_ours(&conv.session_id));
                        fill_elsewhere(&mut conv, &dirs, now_ms(), running);
                        let _ = handle.emit_to(label.as_str(), "conv:upsert", conv);
                    }
                }
            }
        }
    });
    if let Ok(mut map) = app.state::<SessionWatchers>().0.lock() {
        map.insert(project.id.clone(), watcher);
    }
}

pub fn unwatch_project(app: &AppHandle, project_id: &str) {
    if let Ok(mut map) = app.state::<SessionWatchers>().0.lock() {
        map.remove(project_id);
    }
}

// ---------- comandi ----------

fn window_project(window: &WebviewWindow, store: &Store) -> Result<Project> {
    let id = window
        .label()
        .strip_prefix(crate::projects::PROJECT_PREFIX)
        .ok_or("questa finestra non è un progetto")?;
    store.get_project(id)?.ok_or_else(|| format!("progetto {id} inesistente"))
}

/// Lavoro su file e SQLite fuori dal thread principale (i comandi sincroni di Tauri girano lì).
#[tauri::command]
pub async fn conv_list(app: AppHandle, window: WebviewWindow, project_id: String) -> Result<Vec<Conversation>> {
    let project = window_project(&window, &app.state::<Store>())?;
    if project.id != project_id {
        return Err("progetto non della finestra chiamante".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let engine = app.state::<Engine>();
        let running = |id: &str| engine.is_ours(id);
        super::titles::sync_and_enqueue(
            &app.state::<super::titles::TitleQueue>(),
            &default_root(),
            &app.state::<Store>(),
            &app.state::<MetaCache>(),
            &project,
            now_ms(),
            &running,
        )
    })
    .await
    .map_err(err)?
}

#[tauri::command]
pub async fn conv_history(app: AppHandle, window: WebviewWindow, session_id: String) -> Result<Vec<ChatItem>> {
    let project = window_project(&window, &app.state::<Store>())?;
    tauri::async_runtime::spawn_blocking(move || {
        let items = history_in(&default_root(), &app.state::<Store>(), &project, &session_id)?;
        // la scheda aperta di una sessione mai titolata (oltre le 20 importate) la mette in coda
        app.state::<super::titles::TitleQueue>().enqueue_if_due(&session_id);
        Ok(items)
    })
    .await
    .map_err(err)?
}

/// Cerca `query` nel testo delle conversazioni del progetto della finestra chiamante.
#[tauri::command]
pub async fn conv_search(app: AppHandle, window: WebviewWindow, query: String) -> Result<Vec<SearchHit>> {
    let project = window_project(&window, &app.state::<Store>())?;
    tauri::async_runtime::spawn_blocking(move || search_in(&default_root(), &app.state::<Store>(), &project, &query, SEARCH_BUDGET))
        .await
        .map_err(err)?
}

// ---------- test ----------

#[cfg(test)]
mod tests {
    use super::*;
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::cell::Cell;

    /// Allocatore contatore per thread: misura il picco di memoria allocata dal lettore.
    struct Counting;
    thread_local! {
        static CUR: Cell<isize> = const { Cell::new(0) };
        static PEAK: Cell<isize> = const { Cell::new(0) };
    }
    unsafe impl GlobalAlloc for Counting {
        unsafe fn alloc(&self, l: Layout) -> *mut u8 {
            let _ = CUR.try_with(|c| {
                c.set(c.get() + l.size() as isize);
                let _ = PEAK.try_with(|p| p.set(p.get().max(c.get())));
            });
            unsafe { System.alloc(l) }
        }
        unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
            let _ = CUR.try_with(|c| c.set(c.get() - l.size() as isize));
            unsafe { System.dealloc(p, l) }
        }
    }
    #[global_allocator]
    static A: Counting = Counting;

    const U1: &str = "11111111-1111-4111-8111-111111111111";
    const U2: &str = "22222222-2222-4222-8222-222222222222";
    const U3: &str = "33333333-3333-4333-8333-333333333333";

    fn fixtures_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sessions")
    }
    fn fx(name: &str) -> PathBuf {
        fixtures_root().join("-Users-dev-proj").join(format!("{name}.jsonl"))
    }
    fn project(folder: &str) -> Project {
        Project { id: "p1".into(), name: "proj".into(), folders: vec![folder.into()] }
    }
    fn store() -> (tempfile::TempDir, Store) {
        let d = tempfile::tempdir().unwrap();
        let s = Store::open(&d.path().join("t.db")).unwrap();
        (d, s)
    }

    #[test]
    fn encode_matches_cli_rule() {
        assert_eq!(encode_project_dir("/Users/a/workspace/code-art"), "-Users-a-workspace-code-art");
        assert_eq!(encode_project_dir("/Users/a/v2.3.6"), "-Users-a-v2-3-6");
        assert_eq!(encode_project_dir("/x/a b"), "-x-a-b");
        assert_eq!(encode_project_dir("/x/a_b"), "-x-a-b");
        assert_eq!(encode_project_dir("/x/città"), "-x-citt-");
        assert_eq!(encode_project_dir("/x/emoji😀x"), "-x-emoji--x");
        let long = format!("/{}", "L".repeat(229));
        let enc = encode_project_dir(&long);
        assert_eq!(enc.len(), 207);
        assert!(enc.starts_with(&format!("-{}", "L".repeat(199))));
        assert!(enc[200..].starts_with('-') && enc[201..].chars().all(|c| c.is_ascii_alphanumeric()));
    }

    #[test]
    fn meta_counts_only_human_turns() {
        let m = read_session_meta(&fx(U1)).unwrap();
        assert_eq!(m.turn_count, 2);
        assert_eq!(m.session_id, U1);
        assert_eq!(m.cwd, "/Users/dev/proj");
        assert_eq!(m.first_prompt, "Leggi a.txt e cambia red in blue");
        assert_eq!(m.custom_title.as_deref(), Some("Titolo finale"));
        assert_eq!(m.last_activity_ms, parse_iso_ms("2026-11-11T09:00:12.000Z").unwrap());
    }

    #[test]
    fn iso_parsing() {
        assert_eq!(parse_iso_ms("1970-01-01T00:00:00.000Z"), Some(0));
        assert_eq!(parse_iso_ms("2026-11-11T09:13:27.491Z"), Some(1_794_388_407_491));
        assert_eq!(parse_iso_ms("boh"), None);
    }

    #[test]
    fn meta_skips_malformed_and_unknown_lines() {
        let m = read_session_meta(&fx(U2)).unwrap();
        assert_eq!(m.turn_count, 2);
        assert_eq!(m.first_prompt, "Primo prompt");
        let m = read_session_meta(&fx(U1)).unwrap();
        assert_eq!(m.turn_count, 2);
    }

    #[test]
    fn items_follow_documented_shapes() {
        let items = read_session_items(&fx(U1)).unwrap();
        let kinds: Vec<&str> = items
            .iter()
            .map(|i| match i {
                ChatItem::User { .. } => "user",
                ChatItem::Assistant { .. } => "assistant",
                ChatItem::Thinking { .. } => "thinking",
                ChatItem::Tool { .. } => "tool",
                ChatItem::Ask { .. } => "ask",
                ChatItem::Raw { .. } => "raw",
            })
            .collect();
        assert_eq!(kinds, ["user", "tool", "tool", "assistant", "user", "assistant", "raw"]);
        match &items[1] {
            ChatItem::Tool { name, result, ok, file_path, .. } => {
                assert_eq!(name, "Read");
                assert_eq!(result.as_deref(), Some("1\tcolor = red"));
                assert_eq!(*ok, Some(true));
                assert_eq!(file_path.as_deref(), Some("/Users/dev/proj/a.txt"));
            }
            _ => panic!(),
        }
        // il secondo risultato (errore) è seguito da uno di successo: vince l'ultimo
        match &items[2] {
            ChatItem::Tool { result, ok, .. } => {
                assert_eq!(result.as_deref(), Some("ok di nuovo"));
                assert_eq!(*ok, Some(true));
            }
            _ => panic!(),
        }
        assert!(matches!(&items[4], ChatItem::User { text, .. } if text == "Grazie, ora riassumi"));
        let json = serde_json::to_value(&items[0]).unwrap();
        assert_eq!(json["kind"], "user");
        assert_eq!(json["images"], serde_json::json!([]));
    }

    #[test]
    fn items_are_capped_to_the_tail() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("s.jsonl");
        let mut s = String::new();
        for i in 0..2500 {
            s += &format!(
                "{{\"type\":\"user\",\"uuid\":\"u{i}\",\"cwd\":\"/Users/dev/proj\",\"message\":{{\"role\":\"user\",\"content\":\"m{i}\"}}}}\n"
            );
        }
        std::fs::write(&f, s).unwrap();
        let items = read_session_items(&f).unwrap();
        assert_eq!(items.len(), HISTORY_MAX_ITEMS);
        assert!(matches!(&items[0], ChatItem::User { text, .. } if text == "m500"));
    }

    #[test]
    fn meta_streams_large_file() {
        use std::io::Write;
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("big.jsonl");
        let mut i = 0;
        {
            let mut w = std::io::BufWriter::new(File::create(&f).unwrap());
            let pad = "x".repeat(100_000);
            let mut written = 0usize;
            while written < 120 * 1024 * 1024 {
                let user = format!(
                    "{{\"type\":\"user\",\"isSidechain\":false,\"cwd\":\"/Users/dev/proj\",\"timestamp\":\"2026-11-11T09:00:00.000Z\",\"message\":{{\"role\":\"user\",\"content\":\"prompt {i}\"}}}}\n"
                );
                let asst = format!(
                    "{{\"type\":\"assistant\",\"isSidechain\":false,\"cwd\":\"/Users/dev/proj\",\"timestamp\":\"2026-11-11T09:00:01.000Z\",\"message\":{{\"content\":[{{\"type\":\"text\",\"text\":\"{pad}\"}}]}}}}\n"
                );
                w.write_all(user.as_bytes()).unwrap();
                w.write_all(asst.as_bytes()).unwrap();
                written += user.len() + asst.len();
                i += 1;
            }
            // una riga gigante (8 MB) di tipo utente con allegato: viene letta solo in parte ma contata
            let huge = format!(
                "{{\"type\":\"user\",\"isSidechain\":false,\"message\":{{\"role\":\"user\",\"content\":[{{\"type\":\"image\",\"source\":{{\"data\":\"{}\"}}}}]}}}}\n",
                "A".repeat(8 * 1024 * 1024)
            );
            w.write_all(huge.as_bytes()).unwrap();
            w.flush().unwrap();
        }
        assert!(std::fs::metadata(&f).unwrap().len() > 120 * 1024 * 1024);
        CUR.with(|c| c.set(0));
        PEAK.with(|p| p.set(0));
        let m = read_session_meta(&f).unwrap();
        let peak = PEAK.with(|p| p.get());
        assert_eq!(m.turn_count, i + 1); // i prompt normali + la riga gigante troncata ma contata
        assert_eq!(m.cwd, "/Users/dev/proj");
        assert!(peak < 8 * 1024 * 1024, "picco {peak} byte");
    }

    #[test]
    fn session_with_foreign_cwd_is_excluded() {
        let (_d, s) = store();
        let p = project("/Users/dev/proj");
        let list = sync_project_sessions(&fixtures_root(), &s, &MetaCache::default(), &p, now_ms(), &|_| false).unwrap();
        let ids: Vec<&str> = list.iter().map(|c| c.session_id.as_str()).collect();
        assert!(ids.contains(&U1) && ids.contains(&U2));
        assert!(!ids.contains(&U3));
        let c1 = list.iter().find(|c| c.session_id == U1).unwrap();
        assert_eq!(c1.title, "Titolo finale");
        assert_eq!(c1.turn_count, 2);
        assert!(!c1.owned && c1.status == "idle" && c1.permission_mode == "bypassPermissions");
        let c2 = list.iter().find(|c| c.session_id == U2).unwrap();
        assert_eq!(c2.title, "Primo prompt");
        // l'altro progetto vede la sua sessione, non le nostre
        let other = Project { id: "p2".into(), name: "o".into(), folders: vec!["/Users/dev/other".into()] };
        // la cartella codificata di "other" non esiste: nessuna sessione trovata
        assert!(sync_project_sessions(&fixtures_root(), &s, &MetaCache::default(), &other, now_ms(), &|_| false).unwrap().is_empty());
    }

    #[test]
    fn resync_keeps_user_fields_and_updates_counts() {
        let (_d, s) = store();
        let p = project("/Users/dev/proj");
        sync_project_sessions(&fixtures_root(), &s, &MetaCache::default(), &p, now_ms(), &|_| false).unwrap();
        {
            let conn_title = "UPDATE conversations SET title = 'Mio', archived = 1, owned = 1, seen_turn = 5, turn_count = 99 WHERE session_id = ?1";
            s.exec_for_test(conn_title, U1);
        }
        let list = sync_project_sessions(&fixtures_root(), &s, &MetaCache::default(), &p, now_ms(), &|_| false).unwrap();
        let c = list.iter().find(|c| c.session_id == U1).unwrap();
        assert_eq!((c.title.as_str(), c.archived, c.owned, c.turn_count), ("Mio", true, true, 2));
    }

    #[test]
    fn recent_foreign_write_sets_maybe_open_elsewhere() {
        let (_d, s) = store();
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("-Users-dev-proj");
        std::fs::create_dir_all(&dir).unwrap();
        for u in [U1, U2] {
            std::fs::copy(fx(u), dir.join(format!("{u}.jsonl"))).unwrap();
        }
        File::options().write(true).open(dir.join(format!("{U1}.jsonl"))).unwrap().set_modified(SystemTime::now()).unwrap();
        let old = SystemTime::now() - Duration::from_secs(3600);
        File::options().write(true).open(dir.join(format!("{U2}.jsonl"))).unwrap().set_modified(old).unwrap();
        let p = project("/Users/dev/proj");
        let list = sync_project_sessions(root.path(), &s, &MetaCache::default(), &p, now_ms(), &|_| false).unwrap();
        let by = |id: &str| list.iter().find(|c| c.session_id == id).unwrap();
        assert!(by(U1).maybe_open_elsewhere); // appena copiato: mtime = adesso
        assert!(!by(U2).maybe_open_elsewhere);
        // pilotata adesso da un processo del motore: nessun avviso
        s.exec_for_test("UPDATE conversations SET owned = 1 WHERE session_id = ?1", U1);
        let list = conversations_for(root.path(), &s, &p, now_ms(), &|id| id == U1).unwrap();
        assert!(!list.iter().find(|c| c.session_id == U1).unwrap().maybe_open_elsewhere);
        // owned ma senza processo (es. dopo un riavvio dell'app): la scrittura recente può venire da altrove
        let list = conversations_for(root.path(), &s, &p, now_ms(), &|_| false).unwrap();
        assert!(list.iter().find(|c| c.session_id == U1).unwrap().maybe_open_elsewhere);
    }

    #[test]
    fn history_refuses_other_projects_and_bad_ids() {
        let (_d, s) = store();
        let p = project("/Users/dev/proj");
        sync_project_sessions(&fixtures_root(), &s, &MetaCache::default(), &p, now_ms(), &|_| false).unwrap();
        assert!(history_in(&fixtures_root(), &s, &p, U1).is_ok());
        let other = Project { id: "p2".into(), name: "o".into(), folders: vec!["/Users/dev/proj".into()] };
        assert!(history_in(&fixtures_root(), &s, &other, U1).is_err());
        assert!(history_in(&fixtures_root(), &s, &p, U3).is_err()); // non nello store
        assert!(history_in(&fixtures_root(), &s, &p, "../../etc/passwd").is_err());
    }

    fn write_lines(f: &Path, lines: &[&str]) {
        std::fs::write(f, lines.concat()).unwrap();
    }
    fn append(f: &Path, text: &str) {
        use std::io::Write;
        File::options().append(true).open(f).unwrap().write_all(text.as_bytes()).unwrap();
    }
    fn user_line(n: u32, ts: &str) -> String {
        format!("{{\"type\":\"user\",\"cwd\":\"/Users/dev/proj\",\"timestamp\":\"{ts}\",\"message\":{{\"role\":\"user\",\"content\":\"p{n}\"}}}}\n")
    }

    #[test]
    fn cache_skips_unchanged_and_reads_appended_bytes_only() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("s.jsonl");
        write_lines(&f, &[&user_line(1, "2026-11-11T09:00:01.000Z")]);
        let c = MetaCache::default();
        assert_eq!(c.read(&f).unwrap().turn_count, 1);
        assert_eq!(c.read(&f).unwrap().turn_count, 1);
        assert_eq!((c.full_reads.load(Ordering::Relaxed), c.incremental_reads.load(Ordering::Relaxed)), (1, 0));

        append(&f, &user_line(2, "2026-11-11T09:00:05.000Z"));
        let m = c.read(&f).unwrap();
        assert_eq!(m.turn_count, 2);
        assert_eq!(m.first_prompt, "p1");
        assert_eq!(m.last_activity_ms, parse_iso_ms("2026-11-11T09:00:05.000Z").unwrap());
        assert_eq!((c.full_reads.load(Ordering::Relaxed), c.incremental_reads.load(Ordering::Relaxed)), (1, 1));
        assert_eq!(m, read_session_meta(&f).unwrap());
    }

    #[test]
    fn cache_does_not_consume_partial_trailing_line() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("s.jsonl");
        let l2 = user_line(2, "2026-11-11T09:00:05.000Z");
        let (head, tail) = l2.split_at(30);
        write_lines(&f, &[&user_line(1, "2026-11-11T09:00:01.000Z"), head]);
        let c = MetaCache::default();
        assert_eq!(c.read(&f).unwrap().turn_count, 1);
        append(&f, tail); // completa la riga (con il suo \n)
        assert_eq!(c.read(&f).unwrap().turn_count, 2);
        assert_eq!(c.incremental_reads.load(Ordering::Relaxed), 1);
        assert_eq!(c.full_reads.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn cache_rereads_fully_after_truncation() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("s.jsonl");
        write_lines(&f, &[&user_line(1, "2026-11-11T09:00:01.000Z"), &user_line(2, "2026-11-11T09:00:02.000Z")]);
        let c = MetaCache::default();
        assert_eq!(c.read(&f).unwrap().turn_count, 2);
        write_lines(&f, &[&user_line(9, "2026-11-11T09:00:03.000Z")]);
        let m = c.read(&f).unwrap();
        assert_eq!((m.turn_count, m.first_prompt.as_str()), (1, "p9"));
        assert_eq!(c.full_reads.load(Ordering::Relaxed), 2);
        assert_eq!(c.incremental_reads.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn cwd_comparison_is_nfc_on_both_sides() {
        let nfd = "/Users/dev/caffe\u{301}";
        let nfc_form = "/Users/dev/caff\u{e9}";
        assert!(cwd_in_project(nfc_form, &[nfd.to_string()]));
        assert!(cwd_in_project(nfd, &[nfc_form.to_string()]));
        assert!(!cwd_in_project("/Users/dev/caffe", &[nfc_form.to_string()]));
        // anche la cartella di sessione si cerca con il nome NFC
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join(encode_project_dir(nfc_form))).unwrap();
        assert_eq!(session_dirs_for(root.path(), &[nfd.to_string()]).len(), 1);
    }

    #[test]
    fn conv_search_finds_text_in_archived() {
        let (_d, s) = store();
        let p = project("/Users/dev/proj");
        sync_project_sessions(&fixtures_root(), &s, &MetaCache::default(), &p, now_ms(), &|_| false).unwrap();
        s.exec_for_test("UPDATE conversations SET archived = 1 WHERE session_id = ?1", U1);
        let hits = search_in(&fixtures_root(), &s, &p, "  CAMBIA red ", SEARCH_BUDGET).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].session_id, U1);
        assert!(hits[0].snippet.contains("cambia red in blue"));
        // solo il testo di persona/assistente: i risultati degli strumenti non contano
        assert!(search_in(&fixtures_root(), &s, &p, "color = red", SEARCH_BUDGET).unwrap().is_empty());
        // lunghezza fuori limiti: nessun risultato
        assert!(search_in(&fixtures_root(), &s, &p, "c", SEARCH_BUDGET).unwrap().is_empty());
        assert!(search_in(&fixtures_root(), &s, &p, &"a".repeat(201), SEARCH_BUDGET).unwrap().is_empty());
        // altro progetto: niente
        let other = Project { id: "p2".into(), name: "o".into(), folders: vec!["/Users/dev/other".into()] };
        assert!(search_in(&fixtures_root(), &s, &other, "cambia", SEARCH_BUDGET).unwrap().is_empty());
    }

    #[test]
    fn snippet_is_short_single_line_and_case_insensitive() {
        let long = format!("{}Ciao\nMONDO{}", "x".repeat(200), "y".repeat(200));
        let sn = snippet_of(&long, &fold("ciao mondo")).or_else(|| snippet_of(&long, &fold("mondo"))).unwrap();
        assert!(sn.chars().count() <= SNIPPET_CHARS + 2 && !sn.contains('\n') && sn.contains("MONDO"));
        assert!(sn.starts_with('…') && sn.ends_with('…'));
        assert!(snippet_of("abc", &fold("zz")).is_none());
    }

    #[test]
    fn search_with_zero_budget_returns_nothing() {
        let (_d, s) = store();
        let p = project("/Users/dev/proj");
        sync_project_sessions(&fixtures_root(), &s, &MetaCache::default(), &p, now_ms(), &|_| false).unwrap();
        assert!(search_in(&fixtures_root(), &s, &p, "cambia", Duration::ZERO).unwrap().is_empty());
    }

    #[test]
    fn search_caps_results_at_30_one_per_session() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().join("projects");
        let dir = root.join(encode_project_dir("/Users/dev/big"));
        std::fs::create_dir_all(&dir).unwrap();
        let (_td, s) = store();
        let p = Project { id: "pb".into(), name: "big".into(), folders: vec!["/Users/dev/big".into()] };
        for n in 0..35u32 {
            let id = format!("{n:08}-0000-4000-8000-000000000000");
            let line = format!(
                r#"{{"type":"user","uuid":"u{n}","isSidechain":false,"cwd":"/Users/dev/big","sessionId":"{id}","timestamp":"2026-11-11T09:00:01.000Z","message":{{"role":"user","content":"trova ago nel pagliaio, ago due"}}}}"#
            );
            write_lines(&dir.join(format!("{id}.jsonl")), &[&line, "\n"]);
        }
        sync_project_sessions(&root, &s, &MetaCache::default(), &p, now_ms(), &|_| false).unwrap();
        assert_eq!(s.list_conversations("pb").unwrap().len(), 35);
        let hits = search_in(&root, &s, &p, "ago", SEARCH_BUDGET).unwrap();
        assert_eq!(hits.len(), SEARCH_MAX_RESULTS);
        let ids: std::collections::HashSet<_> = hits.iter().map(|h| &h.session_id).collect();
        assert_eq!(ids.len(), 30);
    }

    // ---------- potatura delle righe con file sparito ----------

    const PR_FOLDER: &str = "/Users/dev/pr";

    /// `n` sessioni sul disco, già sincronizzate nello store; restituisce radice, cartella di sessione, store, progetto, ids.
    fn pruning_setup(n: u32) -> (tempfile::TempDir, PathBuf, PathBuf, Store, Project, Vec<String>) {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().join("projects");
        let dir = root.join(encode_project_dir(PR_FOLDER));
        std::fs::create_dir_all(&dir).unwrap();
        let (_td, s) = store();
        std::mem::forget(_td); // lo store deve sopravvivere al helper (cartella temporanea del test, ripulita dal sistema)
        let p = Project { id: "pr".into(), name: "pr".into(), folders: vec![PR_FOLDER.into()] };
        let ids: Vec<String> = (0..n).map(|k| format!("{k:08}-0000-4000-8000-000000000000")).collect();
        for id in &ids {
            let line = format!(
                r#"{{"type":"user","uuid":"u","isSidechain":false,"cwd":"{PR_FOLDER}","sessionId":"{id}","timestamp":"2026-11-11T09:00:01.000Z","message":{{"role":"user","content":"ciao"}}}}"#
            );
            write_lines(&dir.join(format!("{id}.jsonl")), &[&line, "\n"]);
        }
        sync_project_sessions(&root, &s, &MetaCache::default(), &p, now_ms(), &|_| false).unwrap();
        assert_eq!(s.list_conversations("pr").unwrap().len(), n as usize);
        (d, root, dir, s, p, ids)
    }

    fn count(s: &Store) -> usize {
        s.list_conversations("pr").unwrap().len()
    }

    #[test]
    fn prune_drops_only_the_one_vanished_file() {
        let (_d, root, dir, s, p, ids) = pruning_setup(4);
        std::fs::remove_file(dir.join(format!("{}.jsonl", ids[1]))).unwrap();
        sync_project_sessions(&root, &s, &MetaCache::default(), &p, now_ms(), &|_| false).unwrap();
        assert_eq!(count(&s), 3);
        assert!(s.get_conversation(&ids[1]).unwrap().is_none());
        // una conversazione pilotata dal motore resta anche senza file
        std::fs::remove_file(dir.join(format!("{}.jsonl", ids[2]))).unwrap();
        sync_project_sessions(&root, &s, &MetaCache::default(), &p, now_ms(), &|id| id == ids[2]).unwrap();
        assert!(s.get_conversation(&ids[2]).unwrap().is_some());
    }

    #[test]
    fn prune_keeps_everything_when_the_session_dir_cannot_be_listed() {
        use std::os::unix::fs::PermissionsExt;
        for mode in [0o000, 0o100] {
            let (_d, root, dir, s, p, ids) = pruning_setup(5);
            std::fs::remove_file(dir.join(format!("{}.jsonl", ids[0]))).unwrap();
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(mode)).unwrap();
            let r = sync_project_sessions(&root, &s, &MetaCache::default(), &p, now_ms(), &|_| false);
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap(); // ripristino prima delle asserzioni
            r.unwrap();
            assert_eq!(count(&s), 5, "modo {mode:o}: nessuna riga va persa se non si riesce a guardare");
        }
    }

    #[test]
    fn prune_refuses_to_drop_every_row_of_a_larger_project() {
        let (_d, root, dir, s, p, ids) = pruning_setup(4);
        for id in &ids {
            std::fs::remove_file(dir.join(format!("{id}.jsonl"))).unwrap();
        }
        sync_project_sessions(&root, &s, &MetaCache::default(), &p, now_ms(), &|_| false).unwrap();
        assert_eq!(count(&s), 4, "tutte sparite insieme è sospetto");
    }

    #[test]
    fn prune_may_drop_all_rows_of_a_small_project() {
        let (_d, root, dir, s, p, ids) = pruning_setup(2);
        for id in &ids {
            std::fs::remove_file(dir.join(format!("{id}.jsonl"))).unwrap();
        }
        sync_project_sessions(&root, &s, &MetaCache::default(), &p, now_ms(), &|_| false).unwrap();
        assert_eq!(count(&s), 0);
    }
}
