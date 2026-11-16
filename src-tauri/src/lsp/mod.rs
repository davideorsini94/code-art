//! Ponte verso i language server: un processo per progetto e linguaggio, avviato al primo file di quel linguaggio
//! nella cartella principale del progetto, solo in progetti fidati (i server eseguono codice del progetto: build.rs,
//! `go list`, Gradle). Il core non interpreta il protocollo: inoltra messaggi JSON-RPC in entrambe le direzioni
//! (cornice `Content-Length`) e controlla soltanto che ogni URI `file://` in uscita stia dentro il progetto.
//! Processi in una sessione propria (`stop_group` raggiunge i discendenti), riavviati al massimo 3 volte in 5 minuti.
pub mod framing;
pub mod servers;

use crate::claude::locate::{in_new_session, stop_group, TERM_GRACE};
use crate::lock;
use crate::store::{Result, Store};
use serde::Serialize;
use serde_json::Value;
use std::collections::{HashMap, VecDeque};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{ChildStdin, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex, Weak};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager, WebviewWindow};

/// Riavvii automatici concessi in `RESTART_WINDOW`; poi il server resta fermo fino alla riapertura del progetto.
const RESTART_MAX: usize = 3;
const RESTART_WINDOW: Duration = Duration::from_secs(5 * 60);
/// Quanto stderr si conserva (le ultime righe, per l'errore di avvio); il resto viene scartato.
const STDERR_TAIL: usize = 4 * 1024;
/// Un server che esce entro questo tempo dall'avvio è un errore di avvio (restituito da `lsp_start` con lo stderr).
const STARTUP_GRACE: Duration = Duration::from_millis(300);
/// Dopo l'uscita del processo, quanto aspettare ancora la fine di stderr.
const DRAIN_GRACE: Duration = Duration::from_millis(300);
const POLL: Duration = Duration::from_millis(20);
/// Spazio di lavoro di jdtls, per progetto, sotto la cartella dati dell'app (relativo alla home).
const JDTLS_DIR: &str = "Library/Application Support/CodeArt/jdtls";

pub const HALTED: &str = "Language server fermo dopo troppi riavvii: riapri il progetto per riprovare";
const NOT_RUNNING: &str = "Language server non attivo";
const OUTSIDE: &str = "URI fuori dalle cartelle del progetto";

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LspStart {
    pub id: u64,
    pub root_uri: String,
}

/// Eventi per la finestra del progetto: `lsp:message` `{ id, message }` e `lsp:exit` `{ id, code }` (uscita
/// inattesa: il server è già stato riavviato, se c'era budget, e la pagina deve ripetere `initialize`).
#[derive(Debug, Clone, PartialEq)]
pub enum LspEvent {
    Message { id: u64, message: String },
    Exit { id: u64, code: i32 },
}

pub type Emit = Arc<dyn Fn(LspEvent) + Send + Sync>;

/// Come avviare un server: binario, argomenti, ambiente completo e cartella di lavoro.
#[derive(Clone)]
pub struct Launch {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
    pub cwd: PathBuf,
}

/// Avvii recenti: al massimo `RESTART_MAX` in `RESTART_WINDOW`.
#[derive(Default)]
pub struct RestartBudget {
    recent: VecDeque<Instant>,
}

impl RestartBudget {
    /// `true` (e lo conta) se un riavvio a `now` sta nel budget.
    pub fn allow(&mut self, now: Instant) -> bool {
        while self.recent.front().is_some_and(|t| now.saturating_duration_since(*t) >= RESTART_WINDOW) {
            self.recent.pop_front();
        }
        if self.recent.len() >= RESTART_MAX {
            return false;
        }
        self.recent.push_back(now);
        true
    }
}

// ---------- controllo dei messaggi in uscita ----------

/// Accetta solo un oggetto JSON-RPC 2.0 (richiesta, notifica o risposta) in cui ogni URI `file:` (valori e chiavi,
/// a qualunque profondità) e ogni `rootPath` stanno dentro `roots`. Il campo `text` (contenuto di un documento) non
/// è un URI e non si controlla.
pub fn check_message(message: &str, roots: &[PathBuf]) -> Result<()> {
    if message.len() > framing::MAX_CONTENT {
        return Err("messaggio LSP oltre 64 MB".into());
    }
    let v: Value = serde_json::from_str(message).map_err(|_| "messaggio LSP non valido".to_string())?;
    let o = v.as_object().ok_or_else(|| "messaggio LSP non valido".to_string())?;
    if o.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Err("messaggio LSP non valido: serve \"jsonrpc\": \"2.0\"".into());
    }
    let call = o.get("method").is_some_and(Value::is_string);
    let reply = !o.contains_key("method") && o.contains_key("id") && (o.contains_key("result") || o.contains_key("error"));
    if !call && !reply {
        return Err("messaggio LSP non valido: né richiesta né risposta".into());
    }
    check_value(&v, None, roots)
}

fn is_file_uri(s: &str) -> bool {
    s.get(..5).is_some_and(|p| p.eq_ignore_ascii_case("file:"))
}

fn check_value(v: &Value, key: Option<&str>, roots: &[PathBuf]) -> Result<()> {
    match v {
        Value::String(s) => match key {
            Some("text") => Ok(()),
            Some("rootPath") => check_path(Path::new(s), roots),
            _ if is_file_uri(s) => check_uri(s, roots),
            _ => Ok(()),
        },
        Value::Array(items) => items.iter().try_for_each(|x| check_value(x, None, roots)),
        Value::Object(map) => map.iter().try_for_each(|(k, x)| {
            if is_file_uri(k) {
                check_uri(k, roots)?;
            }
            check_value(x, Some(k), roots)
        }),
        _ => Ok(()),
    }
}

/// URI `file:` locale (host vuoto o `localhost`), decodificato, poi confinato come i percorsi dei comandi su file.
fn check_uri(uri: &str, roots: &[PathBuf]) -> Result<()> {
    let url = tauri::Url::parse(uri).map_err(|_| OUTSIDE.to_string())?;
    if url.scheme() != "file" {
        return Err(OUTSIDE.into());
    }
    check_path(&url.to_file_path().map_err(|_| OUTSIDE.to_string())?, roots)
}

/// Niente `..` né NUL (anche da `%2F..`/`%00` decodificati), poi `ensure_inside` (link simbolici risolti).
fn check_path(path: &Path, roots: &[PathBuf]) -> Result<()> {
    use std::os::unix::ffi::OsStrExt;
    if path.as_os_str().as_bytes().contains(&0) || path.components().any(|c| c == Component::ParentDir) {
        return Err(OUTSIDE.into());
    }
    crate::fs_tree::ensure_inside(roots, path).map(|_| ()).map_err(|_| OUTSIDE.to_string())
}

// ---------- processi ----------

/// Processo in corso di un server. `generation` distingue i riavvii (un'uscita vecchia non tocca il processo nuovo).
struct Running {
    generation: u64,
    stdin: Arc<Mutex<ChildStdin>>,
    stop: Arc<AtomicBool>,
    stderr: Arc<Mutex<Vec<u8>>>,
    supervisor: Option<JoinHandle<()>>,
}

impl Running {
    fn halt(&mut self) -> Option<JoinHandle<()>> {
        self.stop.store(true, Ordering::SeqCst);
        self.supervisor.take()
    }
}

/// Server di un progetto per un linguaggio. `run: None` e `starting: false` = fermo dopo troppi riavvii.
struct Slot {
    project: String,
    lang: String,
    root_uri: String,
    launch: Launch,
    emit: Emit,
    budget: RestartBudget,
    /// Durante `STARTUP_GRACE` un'uscita non si riavvia: la segnala `start`.
    starting: bool,
    run: Option<Running>,
    /// Ultima uscita: codice e stderr finale.
    last_exit: Option<(i32, String)>,
}

/// Language server attivi di tutte le finestre, per id.
#[derive(Default)]
pub struct Lsp {
    next: AtomicU64,
    slots: Mutex<HashMap<u64, Slot>>,
}

/// Avvia `launch` in una sessione propria con tre thread: stdout (messaggi completi → `emit`), stderr (ultime 4 KB)
/// e un supervisore che lo ferma su richiesta (`stop`) o su un flusso malformato, e segnala le uscite a `on_exit`.
fn spawn(lsp: Weak<Lsp>, id: u64, generation: u64, launch: &Launch, emit: Emit) -> Result<Running> {
    let mut cmd = crate::tools::command_in(&launch.program, &launch.env);
    cmd.args(&launch.args).current_dir(&launch.cwd);
    in_new_session(&mut cmd);
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("avvio del language server non riuscito: {e}"))?;
    let (Some(stdin), Some(mut out), Some(mut err)) = (child.stdin.take(), child.stdout.take(), child.stderr.take()) else {
        stop_group(&mut child, TERM_GRACE);
        return Err("avvio del language server non riuscito".into());
    };
    let stop = Arc::new(AtomicBool::new(false));
    let broken = Arc::new(AtomicBool::new(false));
    let tail = Arc::new(Mutex::new(Vec::new()));

    let flag = broken.clone();
    std::thread::spawn(move || {
        let mut decoder = framing::Decoder::default();
        let mut chunk = vec![0u8; 64 * 1024];
        while let Ok(n) = out.read(&mut chunk) {
            if n == 0 {
                break;
            }
            match decoder.push(&chunk[..n]) {
                Ok(messages) => {
                    for m in messages {
                        emit(LspEvent::Message { id, message: String::from_utf8_lossy(&m).into_owned() });
                    }
                }
                // flusso non più allineato: il supervisore chiude il server (e lo riavvia, se c'è budget)
                Err(_) => {
                    flag.store(true, Ordering::SeqCst);
                    break;
                }
            }
        }
    });

    let (eof_tx, eof_rx) = mpsc::channel::<()>();
    let buf = tail.clone();
    std::thread::spawn(move || {
        let mut chunk = [0u8; 4096];
        while let Ok(n) = err.read(&mut chunk) {
            if n == 0 {
                break;
            }
            let mut b = lock(&buf);
            b.extend_from_slice(&chunk[..n]);
            let excess = b.len().saturating_sub(STDERR_TAIL);
            b.drain(..excess);
        }
        let _ = eof_tx.send(());
    });

    let halt = stop.clone();
    let supervisor = std::thread::spawn(move || {
        let code = loop {
            if halt.load(Ordering::SeqCst) {
                stop_group(&mut child, TERM_GRACE);
                return;
            }
            if broken.load(Ordering::SeqCst) {
                stop_group(&mut child, TERM_GRACE);
                break -1;
            }
            match child.try_wait() {
                Ok(Some(st)) => break st.code().unwrap_or(-1),
                Ok(None) => std::thread::sleep(POLL),
                Err(_) => {
                    stop_group(&mut child, TERM_GRACE);
                    break -1;
                }
            }
        };
        // i discendenti rimasti nel gruppo non sopravvivono al server; poi lo stderr finale, se arriva
        stop_group(&mut child, TERM_GRACE);
        let _ = eof_rx.recv_timeout(DRAIN_GRACE);
        if let Some(lsp) = lsp.upgrade() {
            lsp.on_exit(id, generation, code);
        }
    });
    Ok(Running { generation, stdin: Arc::new(Mutex::new(stdin)), stop, stderr: tail, supervisor: Some(supervisor) })
}

impl Lsp {
    /// Server di `lang` per il progetto (riusato se c'è già), nella sua cartella principale. `resolve` riceve la
    /// cartella e trova il binario; `emit` riceve messaggi e uscite.
    pub fn start(self: &Arc<Self>, store: &Store, project_id: &str, lang: &str, resolve: impl FnOnce(&Path) -> Result<Launch>, emit: Emit) -> Result<LspStart> {
        crate::trust::ensure_project_trusted(store, project_id)?;
        let project = store.get_project(project_id)?.ok_or_else(|| crate::trust::UNTRUSTED.to_string())?;
        servers::server(lang).ok_or_else(|| format!("linguaggio non supportato: {lang}"))?;
        if let Some(found) = self.existing(project_id, lang) {
            return found;
        }
        let root = project.folders.first().map(PathBuf::from).ok_or_else(|| "progetto senza cartelle".to_string())?;
        let root = root.canonicalize().map_err(|_| "cartella principale del progetto non trovata".to_string())?;
        let root_uri = tauri::Url::from_file_path(&root).map_err(|_| "cartella principale non valida".to_string())?.to_string();
        // fuori dal lucchetto: la prima ricerca del binario può aspettare l'ambiente di login
        let launch = resolve(&root)?;
        let id = {
            let mut slots = lock(&self.slots);
            // un avvio concorrente può essere arrivato prima
            if let Some(found) = existing_in(&slots, project_id, lang) {
                return found;
            }
            let id = self.next.fetch_add(1, Ordering::Relaxed) + 1;
            let run = spawn(Arc::downgrade(self), id, 0, &launch, emit.clone())?;
            slots.insert(
                id,
                Slot {
                    project: project_id.to_string(),
                    lang: lang.to_string(),
                    root_uri: root_uri.clone(),
                    launch,
                    emit,
                    budget: RestartBudget::default(),
                    starting: true,
                    run: Some(run),
                    last_exit: None,
                },
            );
            id
        };
        let t0 = Instant::now();
        while t0.elapsed() < STARTUP_GRACE && lock(&self.slots).get(&id).is_some_and(|s| s.run.is_some()) {
            std::thread::sleep(POLL);
        }
        let mut slots = lock(&self.slots);
        let Some(slot) = slots.get_mut(&id) else { return Err(NOT_RUNNING.into()) };
        if slot.run.is_none() {
            let (code, tail) = slot.last_exit.take().unwrap_or((-1, String::new()));
            slots.remove(&id);
            let why = if tail.is_empty() { String::new() } else { format!(": {tail}") };
            return Err(format!("il language server per {lang} si è chiuso all'avvio (uscita {code}){why}"));
        }
        slot.starting = false;
        Ok(LspStart { id, root_uri })
    }

    fn existing(&self, project_id: &str, lang: &str) -> Option<Result<LspStart>> {
        existing_in(&lock(&self.slots), project_id, lang)
    }

    /// Inoltra `message` al server `id` del progetto, dopo `check_message` sulle cartelle attuali del progetto.
    pub fn send(&self, store: &Store, project_id: &str, id: u64, message: &str) -> Result<()> {
        crate::trust::ensure_project_trusted(store, project_id)?;
        let project = store.get_project(project_id)?.ok_or_else(|| crate::trust::UNTRUSTED.to_string())?;
        let stdin = {
            let slots = lock(&self.slots);
            let slot = slots.get(&id).filter(|s| s.project == project_id).ok_or_else(|| NOT_RUNNING.to_string())?;
            slot.run.as_ref().ok_or_else(|| HALTED.to_string())?.stdin.clone()
        };
        let roots: Vec<PathBuf> = project.folders.iter().map(PathBuf::from).collect();
        check_message(message, &roots)?;
        let mut w = lock(&stdin);
        w.write_all(&framing::encode(message.as_bytes())).and_then(|()| w.flush()).map_err(|_| NOT_RUNNING.to_string())
    }

    /// Ferma il server `id` se appartiene al progetto e aspetta la fine del suo gruppo di processi. Un server
    /// fermo dopo troppi riavvii resta tale fino alla chiusura della finestra.
    pub fn stop(&self, project_id: &str, id: u64) {
        let run = {
            let mut slots = lock(&self.slots);
            match slots.get(&id) {
                Some(s) if s.project == project_id && s.run.is_some() => slots.remove(&id).and_then(|s| s.run),
                _ => None,
            }
        };
        if let Some(h) = run.and_then(|mut r| r.halt()) {
            let _ = h.join();
        }
    }

    /// Ferma (senza aspettare) e dimentica tutti i server del progetto: chiusura della sua finestra.
    pub fn stop_project(&self, project_id: &str) {
        let mut slots = lock(&self.slots);
        slots.retain(|_, s| {
            if s.project != project_id {
                return true;
            }
            if let Some(r) = s.run.as_mut() {
                r.halt();
            }
            false
        });
    }

    /// Ferma tutti i server in parallelo e aspetta (uscita dell'app: i gruppi propri sopravviverebbero).
    pub fn stop_all(&self) {
        let slots: Vec<Slot> = lock(&self.slots).drain().map(|(_, s)| s).collect();
        let handles: Vec<JoinHandle<()>> = slots.into_iter().filter_map(|s| s.run.and_then(|mut r| r.halt())).collect();
        for h in handles {
            let _ = h.join();
        }
    }

    /// Uscita non richiesta del processo `generation` del server `id`: riavvio se c'è budget, altrimenti fermo; poi `lsp:exit`.
    fn on_exit(self: &Arc<Self>, id: u64, generation: u64, code: i32) {
        let emit = {
            let mut slots = lock(&self.slots);
            let Some(slot) = slots.get_mut(&id) else { return };
            if slot.run.as_ref().map(|r| r.generation) != Some(generation) {
                return;
            }
            let Some(old) = slot.run.take() else { return };
            let tail = String::from_utf8_lossy(&lock(&old.stderr)).trim().to_string();
            slot.last_exit = Some((code, tail));
            if slot.starting {
                return;
            }
            if slot.budget.allow(Instant::now()) {
                slot.run = spawn(Arc::downgrade(self), id, generation + 1, &slot.launch, slot.emit.clone()).ok();
            }
            slot.emit.clone()
        };
        emit(LspEvent::Exit { id, code });
    }
}

fn existing_in(slots: &HashMap<u64, Slot>, project_id: &str, lang: &str) -> Option<Result<LspStart>> {
    let (id, slot) = slots.iter().find(|(_, s)| s.project == project_id && s.lang == lang)?;
    Some(match (&slot.run, slot.starting) {
        (Some(_), _) => Ok(LspStart { id: *id, root_uri: slot.root_uri.clone() }),
        (None, false) => Err(HALTED.into()),
        (None, true) => Err(NOT_RUNNING.into()),
    })
}

// ---------- comandi ----------

#[derive(Serialize, Clone)]
struct MessagePayload {
    id: u64,
    message: String,
}

#[derive(Serialize, Clone)]
struct ExitPayload {
    id: u64,
    code: i32,
}

/// Eventi del server solo alla finestra del progetto.
fn project_emit(app: &AppHandle, project_id: &str) -> Emit {
    let (app, pid) = (app.clone(), project_id.to_string());
    Arc::new(move |e| match e {
        LspEvent::Message { id, message } => crate::projects::emit_to_project(&app, &pid, "lsp:message", MessagePayload { id, message }),
        LspEvent::Exit { id, code } => crate::projects::emit_to_project(&app, &pid, "lsp:exit", ExitPayload { id, code }),
    })
}

/// Server installato per `lang` con l'ambiente di login; per jdtls crea lo spazio di lavoro del progetto.
fn launch_for(lang: &str, root: &Path, data_dir: &Path) -> Result<Launch> {
    let env = crate::tools::env();
    let r = servers::resolve(env, crate::tools::SYSTEM_DIRS, lang, data_dir)?;
    if let Some(d) = &r.data_dir {
        std::fs::create_dir_all(d).map_err(|e| format!("cartella di lavoro di {lang} non creata: {e}"))?;
    }
    Ok(Launch { program: r.program, args: r.args, env: env.clone(), cwd: root.to_path_buf() })
}

/// Avvia (o riusa) il server di `lang` per il progetto della finestra chiamante.
#[tauri::command]
pub async fn lsp_start(app: AppHandle, window: WebviewWindow, lang: String) -> Result<LspStart> {
    let id = crate::projects::window_project_id(&window)?;
    crate::projects::off_main(move || {
        if !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') {
            return Err("progetto non valido".into());
        }
        let data_dir = app.path().home_dir().map_err(|e| e.to_string())?.join(JDTLS_DIR).join(&id);
        let lsp = app.state::<Arc<Lsp>>().inner().clone();
        lsp.start(&app.state::<Store>(), &id, &lang, |root| launch_for(&lang, root, &data_dir), project_emit(&app, &id))
    })
    .await
}

#[tauri::command]
pub async fn lsp_send(app: AppHandle, window: WebviewWindow, id: u64, message: String) -> Result<()> {
    let project_id = crate::projects::window_project_id(&window)?;
    crate::projects::off_main(move || app.state::<Arc<Lsp>>().send(&app.state::<Store>(), &project_id, id, &message)).await
}

#[tauri::command]
pub async fn lsp_stop(app: AppHandle, window: WebviewWindow, id: u64) -> Result<()> {
    let project_id = crate::projects::window_project_id(&window)?;
    crate::projects::off_main(move || {
        app.state::<Arc<Lsp>>().stop(&project_id, id);
        Ok(())
    })
    .await
}

#[cfg(test)]
mod tests;
