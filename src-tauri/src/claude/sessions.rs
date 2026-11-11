//! Lettura delle sessioni Claude Code già presenti su disco (`~/.claude/projects`, solo lettura)
//! e sincronizzazione nello store. Formato: docs/claude-protocol.md §11.
use super::{ChatItem, Conversation};
use crate::store::{Project, Result, Store};
use serde_json::Value;
use std::collections::HashMap;
use std::fs::File;
use std::io::{self, BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

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

/// Cartelle di sessione esistenti per le cartelle del progetto (percorso dato e percorso reale).
pub fn session_dirs_in(root: &Path, folders: &[String]) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for f in folders {
        let mut names = vec![encode_project_dir(f)];
        if let Ok(real) = Path::new(f).canonicalize() {
            names.push(encode_project_dir(&real.to_string_lossy()));
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

#[allow(dead_code)] // API dichiarata dal brief; i chiamanti usano `session_dirs_in` con la radice iniettata
pub fn session_dirs_for(folders: &[String]) -> Vec<PathBuf> {
    session_dirs_in(&default_root(), folders)
}

fn normalize(p: &str) -> String {
    let t = p.trim_end_matches('/');
    if t.is_empty() { "/".into() } else { t.into() }
}

fn cwd_in_project(cwd: &str, folders: &[String]) -> bool {
    let c = normalize(cwd);
    let real = Path::new(cwd).canonicalize().ok().map(|p| p.to_string_lossy().to_string());
    folders.iter().any(|f| {
        let f = normalize(f);
        f == c
            || real.as_deref().is_some_and(|r| {
                r == f || Path::new(&f).canonicalize().is_ok_and(|rf| rf.to_string_lossy() == r)
            })
    })
}

// ---------- lettura a righe con tetto ----------

/// Chiama `f(riga, troncata)` per ogni riga tenendo in memoria al massimo `cap` byte per riga.
fn for_each_line<R: BufRead>(mut r: R, cap: usize, mut f: impl FnMut(&[u8], bool)) -> io::Result<()> {
    let mut buf: Vec<u8> = Vec::new();
    loop {
        buf.clear();
        let mut truncated = false;
        let mut any = false;
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
            if found {
                break;
            }
        }
        if !any {
            return Ok(());
        }
        f(&buf, truncated);
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

/// Prompt scritto dalla persona (§11.3): `Some((testo, immagini))`.
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

/// Legge in streaming (memoria limitata dal tetto per riga). Righe malformate o di tipo ignoto: saltate.
pub fn read_session_meta(file: &Path) -> Result<SessionMeta> {
    let meta = std::fs::metadata(file).map_err(err)?;
    let mtime = mtime_ms(&meta);
    let session_id = file.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let mut out = SessionMeta {
        session_id,
        cwd: String::new(),
        first_prompt: String::new(),
        last_activity_ms: 0,
        turn_count: 0,
        custom_title: None,
        mtime_ms: mtime,
    };
    let note_ts = |out: &mut SessionMeta, ts: Option<&str>| {
        if let Some(ms) = ts.and_then(parse_iso_ms) {
            out.last_activity_ms = out.last_activity_ms.max(ms);
        }
    };
    for_each_line(open_reader(file).map_err(err)?, META_LINE_CAP, |line, truncated| {
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
            note_ts(&mut out, extract_str(text, "\"timestamp\":\""));
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
        note_ts(&mut out, v["timestamp"].as_str());
        if let Some((t, _)) = human_prompt(&v) {
            out.turn_count += 1;
            if out.first_prompt.is_empty() {
                out.first_prompt = t.chars().take(200).collect();
            }
        }
    })
    .map_err(err)?;
    if out.last_activity_ms == 0 {
        out.last_activity_ms = mtime;
    }
    Ok(out)
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
    for_each_line(open_reader(file).map_err(err)?, ITEMS_LINE_CAP, |line, truncated| {
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
pub fn sync_session_file(store: &Store, project: &Project, file: &Path) -> Result<Option<SessionMeta>> {
    let meta = read_session_meta(file)?;
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

pub fn sync_project_sessions_in(root: &Path, store: &Store, project: &Project, now: i64) -> Result<Vec<Conversation>> {
    for dir in session_dirs_in(root, &project.folders) {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for entry in rd.flatten() {
            let p = entry.path();
            if is_session_file(&p) && p.is_file() {
                let _ = sync_session_file(store, project, &p); // un file illeggibile non blocca gli altri
            }
        }
    }
    conversations_for(root, store, project, now)
}

pub fn sync_project_sessions(store: &Store, project: &Project) -> Result<Vec<Conversation>> {
    sync_project_sessions_in(&default_root(), store, project, now_ms())
}

/// Conversazioni salvate del progetto con `maybe_open_elsewhere` calcolato dall'mtime del file.
pub fn conversations_for(root: &Path, store: &Store, project: &Project, now: i64) -> Result<Vec<Conversation>> {
    let dirs = session_dirs_in(root, &project.folders);
    let mut list = store.list_conversations(&project.id)?;
    for c in &mut list {
        fill_elsewhere(c, &dirs, now);
    }
    Ok(list)
}

pub fn fill_elsewhere(c: &mut Conversation, dirs: &[PathBuf], now: i64) {
    c.maybe_open_elsewhere = !c.owned
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
    for dir in session_dirs_in(root, &project.folders) {
        let f = dir.join(format!("{session_id}.jsonl"));
        if f.is_file() {
            return read_session_items(&f);
        }
    }
    Err("file della sessione non trovato".into())
}

// ---------- watcher ----------

use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::HashSet;
use std::sync::mpsc::{channel, RecvTimeoutError};
use tauri::{AppHandle, Emitter, Manager, State, WebviewWindow};

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
            let dirs: Vec<PathBuf> = session_dirs_in(&root, &project.folders)
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
                if let Ok(Some(meta)) = sync_session_file(&store, &project, &p) {
                    if let Ok(Some(mut conv)) = store.get_conversation(&meta.session_id) {
                        fill_elsewhere(&mut conv, &dirs, now_ms());
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

#[tauri::command]
pub fn conv_list(window: WebviewWindow, store: State<Store>, project_id: String) -> Result<Vec<Conversation>> {
    let project = window_project(&window, &store)?;
    if project.id != project_id {
        return Err("progetto non della finestra chiamante".into());
    }
    sync_project_sessions(&store, &project)
}

#[tauri::command]
pub fn conv_history(window: WebviewWindow, store: State<Store>, session_id: String) -> Result<Vec<ChatItem>> {
    let project = window_project(&window, &store)?;
    history_in(&default_root(), &store, &project, &session_id)
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
        {
            let mut w = std::io::BufWriter::new(File::create(&f).unwrap());
            let pad = "x".repeat(100_000);
            let mut written = 0usize;
            let mut i = 0;
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
        assert!(m.turn_count > 100);
        assert_eq!(m.cwd, "/Users/dev/proj");
        assert!(peak < 8 * 1024 * 1024, "picco {peak} byte");
    }

    #[test]
    fn session_with_foreign_cwd_is_excluded() {
        let (_d, s) = store();
        let p = project("/Users/dev/proj");
        let list = sync_project_sessions_in(&fixtures_root(), &s, &p, now_ms()).unwrap();
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
        assert!(sync_project_sessions_in(&fixtures_root(), &s, &other, now_ms()).unwrap().is_empty());
    }

    #[test]
    fn resync_keeps_user_fields_and_updates_counts() {
        let (_d, s) = store();
        let p = project("/Users/dev/proj");
        sync_project_sessions_in(&fixtures_root(), &s, &p, now_ms()).unwrap();
        {
            let conn_title = "UPDATE conversations SET title = 'Mio', archived = 1, owned = 1, seen_turn = 5, turn_count = 99 WHERE session_id = ?1";
            s.exec_for_test(conn_title, U1);
        }
        let list = sync_project_sessions_in(&fixtures_root(), &s, &p, now_ms()).unwrap();
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
        let list = sync_project_sessions_in(root.path(), &s, &p, now_ms()).unwrap();
        let by = |id: &str| list.iter().find(|c| c.session_id == id).unwrap();
        assert!(by(U1).maybe_open_elsewhere); // appena copiato: mtime = adesso
        assert!(!by(U2).maybe_open_elsewhere);
        s.exec_for_test("UPDATE conversations SET owned = 1 WHERE session_id = ?1", U1);
        let list = conversations_for(root.path(), &s, &p, now_ms()).unwrap();
        assert!(!list.iter().find(|c| c.session_id == U1).unwrap().maybe_open_elsewhere);
    }

    #[test]
    fn history_refuses_other_projects_and_bad_ids() {
        let (_d, s) = store();
        let p = project("/Users/dev/proj");
        sync_project_sessions_in(&fixtures_root(), &s, &p, now_ms()).unwrap();
        assert!(history_in(&fixtures_root(), &s, &p, U1).is_ok());
        let other = Project { id: "p2".into(), name: "o".into(), folders: vec!["/Users/dev/proj".into()] };
        assert!(history_in(&fixtures_root(), &s, &other, U1).is_err());
        assert!(history_in(&fixtures_root(), &s, &p, U3).is_err()); // non nello store
        assert!(history_in(&fixtures_root(), &s, &p, "../../etc/passwd").is_err());
    }
}
