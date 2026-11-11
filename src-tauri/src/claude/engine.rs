//! Motore delle conversazioni: un processo `claude` per conversazione, stream-json su stdin/stdout
//! (docs/claude-protocol.md), stato salvato nello store ed eventi inoltrati alla finestra del progetto.
use super::protocol::{parse_line, AskType, ClaudeEvent};
use super::state::{next_status, ConvState, StateInput};
use super::{sessions, ConvStatus, Conversation};
use crate::store::{Project, Result, Store};
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard, OnceLock};

/// Tetto per `Unknown.json` negli eventi (le righe utente riemesse possono contenere immagini base64).
const UNKNOWN_JSON_CAP: usize = 4096;
/// Coda di stderr conservata per l'evento "Processo terminato".
const STDERR_TAIL: usize = 4096;
const IMAGE_MAX_BYTES: u64 = 20 * 1024 * 1024;
pub const PERMISSION_MODES: &[&str] = &["default", "acceptEdits", "auto", "bypassPermissions", "manual", "dontAsk", "plan"];
const FALLBACK_PERMISSION_MODE: &str = "bypassPermissions";
const DENY_MESSAGE: &str = "The user denied this action.";
const PLAN_REJECT_MESSAGE: &str = "The user rejected the plan.";
const EXIT_LABEL: &str = "Processo terminato";

/// Destinazione degli eventi (`conv:event`, `conv:upsert`); nei test un finto che li registra.
pub trait EventSink: Send + Sync + 'static {
    fn emit(&self, project_id: &str, event: &str, payload: Value);
}

/// Eseguibile `claude` e ambiente con cui avviarlo.
pub struct Launch {
    pub claude: Option<PathBuf>,
    pub env: HashMap<String, String>,
}

/// Calcolato una volta sola (la shell di login può impiegare qualche secondo).
pub type LaunchCell = LazyLock<Launch, Box<dyn FnOnce() -> Launch + Send>>;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeCheck {
    pub found: bool,
    pub version: Option<String>,
    pub path: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartOpts {
    pub model: Option<String>,
    pub permission_mode: Option<String>,
}

/// Cartella temporanea dell'app per le immagini incollate (unica fuori dal progetto accettata da `conv_send`).
pub fn image_dir() -> PathBuf {
    std::env::temp_dir().join("CodeArt/images")
}

/// Alias o id di modello: niente spazi, niente trattino iniziale (non deve sembrare un'opzione).
pub fn valid_model(m: &str) -> bool {
    let mut chars = m.chars();
    m.len() <= 80
        && chars.next().is_some_and(|c| c.is_ascii_alphanumeric())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | ':' | '[' | ']'))
}

pub fn valid_permission_mode(m: &str) -> bool {
    PERMISSION_MODES.contains(&m)
}

fn truncate_utf8(s: &mut String, max: usize) {
    if s.len() <= max {
        return;
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s.truncate(end);
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

fn parse_status(s: &str) -> ConvStatus {
    match s {
        "working" => ConvStatus::Working,
        "waiting" => ConvStatus::Waiting,
        _ => ConvStatus::Idle,
    }
}

/// Processo vivo di una conversazione.
struct Conv {
    project_id: String,
    stdin: Mutex<Option<ChildStdin>>,
    rt: Mutex<Runtime>,
}

struct Runtime {
    state: ConvState,
    /// Richieste aperte: request id → evento `Ask` (serve `tool_name`/`input` per rispondere).
    pending: HashMap<String, ClaudeEvent>,
    next_req: u64,
}

impl Runtime {
    fn request_id(&mut self) -> String {
        self.next_req += 1;
        format!("host_req_{}", self.next_req)
    }
}

struct Inner {
    launch: Arc<LaunchCell>,
    store: Store,
    sink: Arc<dyn EventSink>,
    sessions_root: PathBuf,
    image_dir: PathBuf,
    convs: Mutex<HashMap<String, Arc<Conv>>>,
    titles: OnceLock<super::titles::TitleQueue>,
}

/// Stato condiviso dell'app (non della finestra): i processi sopravvivono alla chiusura della finestra.
#[derive(Clone)]
pub struct Engine(Arc<Inner>);

impl Engine {
    pub fn new(launch: Arc<LaunchCell>, store: Store, sink: Arc<dyn EventSink>, sessions_root: PathBuf, image_dir: PathBuf) -> Engine {
        // nessun processo è vivo alla creazione del motore (= avvio dell'app)
        let _ = store.reset_working_conversations();
        Engine(Arc::new(Inner { launch, store, sink, sessions_root, image_dir, convs: Mutex::new(HashMap::new()), titles: OnceLock::new() }))
    }

    /// Coda di titolazione (impostata una volta all'avvio dell'app): a fine turno decide se ritolare.
    pub fn set_titles(&self, q: super::titles::TitleQueue) {
        let _ = self.0.titles.set(q);
    }

    fn claude_path(&self) -> Option<PathBuf> {
        let l = &**self.0.launch;
        l.claude.clone().or_else(|| super::locate::find_claude(&l.env))
    }

    pub fn claude_check(&self) -> ClaudeCheck {
        let path = self.claude_path();
        let version = path.as_deref().and_then(|p| super::locate::claude_version(p, &self.0.launch.env));
        ClaudeCheck { found: path.is_some(), version, path: path.map(|p| p.to_string_lossy().into_owned()) }
    }

    pub fn is_running(&self, session_id: &str) -> bool {
        lock(&self.0.convs).contains_key(session_id)
    }

    pub fn running(&self, project_id: &str) -> Vec<String> {
        lock(&self.0.convs).iter().filter(|(_, c)| c.project_id == project_id).map(|(id, _)| id.clone()).collect()
    }

    fn project(&self, project_id: &str) -> Result<Project> {
        self.0.store.get_project(project_id)?.ok_or_else(|| format!("progetto {project_id} inesistente"))
    }

    /// La conversazione esiste e appartiene al progetto della finestra chiamante.
    fn owned_conv(&self, project_id: &str, session_id: &str) -> Result<Conversation> {
        if !sessions::valid_session_id(session_id) {
            return Err("sessione non valida".into());
        }
        match self.0.store.get_conversation(session_id)? {
            Some(c) if c.project_id == project_id => Ok(c),
            _ => Err("sessione non appartiene a questo progetto".into()),
        }
    }

    fn live(&self, session_id: &str) -> Option<Arc<Conv>> {
        lock(&self.0.convs).get(session_id).cloned()
    }

    fn default_mode(&self) -> String {
        self.0
            .store
            .setting_get("default_permission_mode")
            .ok()
            .flatten()
            .filter(|m| valid_permission_mode(m))
            .unwrap_or_else(|| FALLBACK_PERMISSION_MODE.into())
    }

    // ---------- comandi ----------

    pub fn start(&self, project_id: &str, opts: StartOpts) -> Result<String> {
        let project = self.project(project_id)?;
        if let Some(m) = opts.model.as_deref().filter(|m| !valid_model(m)) {
            return Err(format!("modello non valido: {m}"));
        }
        let mode = match opts.permission_mode {
            Some(m) if valid_permission_mode(&m) => m,
            Some(m) => return Err(format!("modalità permessi non valida: {m}")),
            None => self.default_mode(),
        };
        let cwd = project.folders.first().ok_or("il progetto non ha cartelle")?;
        let session_id = uuid::Uuid::new_v4().to_string();
        self.0.store.insert_owned_conversation(project_id, &session_id, cwd, opts.model.as_deref(), &mode)?;
        self.ensure_running(project_id, &session_id)?;
        Ok(session_id)
    }

    pub fn resume(&self, project_id: &str, session_id: &str) -> Result<()> {
        self.owned_conv(project_id, session_id)?;
        self.ensure_running(project_id, session_id).map(|_| ())
    }

    pub fn send(&self, project_id: &str, session_id: &str, text: &str, image_paths: &[String]) -> Result<()> {
        self.owned_conv(project_id, session_id)?;
        let project = self.project(project_id)?;
        let images = image_paths.iter().map(|p| self.load_image(&project, p)).collect::<Result<Vec<Value>>>()?;
        if text.trim().is_empty() && images.is_empty() {
            return Err("messaggio vuoto".into());
        }
        let content = if images.is_empty() {
            Value::String(text.to_string())
        } else {
            let mut blocks = vec![json!({"type": "text", "text": text})];
            blocks.extend(images);
            Value::Array(blocks)
        };
        let conv = self.ensure_running(project_id, session_id)?;
        // lo stato passa a `working` prima della scrittura: un `result` velocissimo non deve arrivare prima
        self.apply(session_id, Some(&conv), StateInput::UserSent, true);
        let msg = json!({"type": "user", "session_id": "", "parent_tool_use_id": null,
                         "message": {"role": "user", "content": content}});
        if let Err(e) = write_line(&conv, &msg) {
            self.apply(session_id, Some(&conv), StateInput::ProcessExited, false);
            return Err(e);
        }
        Ok(())
    }

    pub fn answer(&self, project_id: &str, session_id: &str, request_id: &str, choice: &str) -> Result<()> {
        self.owned_conv(project_id, session_id)?;
        let conv = self.live(session_id).ok_or("la conversazione non è attiva")?;
        let response = {
            let mut rt = lock(&conv.rt);
            let ask = rt.pending.get(request_id).ok_or("richiesta non più aperta")?;
            let response = answer_payload(ask, choice)?;
            rt.pending.remove(request_id);
            rt.state = next_status(rt.state, StateInput::AskAnswered);
            self.save(session_id, &rt.state, false);
            response
        };
        self.emit_upsert(session_id);
        write_line(
            &conv,
            &json!({"type": "control_response", "response": {"subtype": "success", "request_id": request_id, "response": response}}),
        )
    }

    /// Interrompe il turno. Le richieste aperte si chiudono subito (§7, §5.5): la CLI manderà
    /// `control_cancel_request`, che a quel punto viene ignorato perché la richiesta non è più pendente.
    pub fn interrupt(&self, project_id: &str, session_id: &str) -> Result<()> {
        self.owned_conv(project_id, session_id)?;
        let Some(conv) = self.live(session_id) else { return Ok(()) };
        let (req_id, cancelled) = {
            let mut rt = lock(&conv.rt);
            let ids: Vec<String> = rt.pending.drain().map(|(id, _)| id).collect();
            let mut cancelled = Vec::new();
            for id in ids {
                let ev = ClaudeEvent::AskCancelled { request_id: id };
                rt.state = next_status(rt.state, StateInput::Event(&ev));
                cancelled.push(ev);
            }
            self.save(session_id, &rt.state, false);
            (rt.request_id(), cancelled)
        };
        for ev in &cancelled {
            self.emit_event(project_id, session_id, ev);
        }
        if !cancelled.is_empty() {
            self.emit_upsert(session_id);
        }
        write_line(&conv, &json!({"type": "control_request", "request_id": req_id, "request": {"subtype": "interrupt"}}))
    }

    pub fn set_model(&self, project_id: &str, session_id: &str, model: &str) -> Result<()> {
        self.owned_conv(project_id, session_id)?;
        if !valid_model(model) {
            return Err(format!("modello non valido: {model}"));
        }
        self.0.store.set_conversation_model(session_id, model)?;
        self.emit_upsert(session_id);
        self.control(session_id, json!({"subtype": "set_model", "model": model}))
    }

    pub fn set_permission_mode(&self, project_id: &str, session_id: &str, mode: &str) -> Result<()> {
        self.owned_conv(project_id, session_id)?;
        if !valid_permission_mode(mode) {
            return Err(format!("modalità permessi non valida: {mode}"));
        }
        self.0.store.set_conversation_permission_mode(session_id, mode)?;
        self.emit_upsert(session_id);
        self.control(session_id, json!({"subtype": "set_permission_mode", "mode": mode}))
    }

    pub fn mark_viewed(&self, project_id: &str, session_id: &str) -> Result<()> {
        self.owned_conv(project_id, session_id)?;
        let conv = self.live(session_id);
        self.apply(session_id, conv.as_ref(), StateInput::Viewed, false);
        Ok(())
    }

    pub fn archive(&self, project_id: &str, session_id: &str, archived: bool) -> Result<()> {
        self.owned_conv(project_id, session_id)?;
        self.0.store.set_conversation_archived(session_id, archived)?;
        self.emit_upsert(session_id);
        Ok(())
    }

    /// Messaggio di controllo a processo vivo (§8); senza processo il valore salvato vale alla ripresa.
    fn control(&self, session_id: &str, request: Value) -> Result<()> {
        let Some(conv) = self.live(session_id) else { return Ok(()) };
        let req_id = lock(&conv.rt).request_id();
        write_line(&conv, &json!({"type": "control_request", "request_id": req_id, "request": request}))
    }

    // ---------- stato ----------

    fn load_state(&self, session_id: &str) -> ConvState {
        match self.0.store.conversation_state(session_id).ok().flatten() {
            Some((status, turns, seen)) => ConvState {
                status: parse_status(&status),
                open_asks: 0,
                turn_count: turns.max(0) as u32,
                seen_turn: seen.unwrap_or(turns).max(0) as u32,
            },
            None => ConvState::default(),
        }
    }

    fn save(&self, session_id: &str, s: &ConvState, touch: bool) {
        let _ = self.0.store.save_conversation_state(session_id, s.status.as_str(), s.turn_count.into(), s.seen_turn.into(), touch);
    }

    /// Applica un ingresso alla macchina degli stati (in memoria se il processo è vivo, altrimenti dallo store).
    fn apply(&self, session_id: &str, conv: Option<&Arc<Conv>>, input: StateInput, touch: bool) {
        let (before, after) = match conv {
            Some(c) => {
                let mut rt = lock(&c.rt);
                let before = rt.state;
                rt.state = next_status(before, input);
                self.save(session_id, &rt.state, touch);
                (before, rt.state)
            }
            None => {
                let before = self.load_state(session_id);
                let after = next_status(before, input);
                self.save(session_id, &after, touch);
                (before, after)
            }
        };
        if before != after || touch {
            if let Ok(Some(c)) = self.0.store.get_conversation(session_id) {
                self.emit_conversation(c);
            }
        }
    }

    // ---------- eventi ----------

    fn emit_event(&self, project_id: &str, session_id: &str, ev: &ClaudeEvent) {
        let event = serde_json::to_value(ev).unwrap_or(Value::Null);
        self.0.sink.emit(project_id, "conv:event", json!({"sessionId": session_id, "event": event}));
    }

    fn emit_conversation(&self, mut c: Conversation) {
        // le conversazioni emesse dal motore sono pilotate da noi
        c.maybe_open_elsewhere = false;
        let project_id = c.project_id.clone();
        if let Ok(v) = serde_json::to_value(&c) {
            self.0.sink.emit(&project_id, "conv:upsert", v);
        }
    }

    fn emit_upsert(&self, session_id: &str) {
        if let Ok(Some(c)) = self.0.store.get_conversation(session_id) {
            self.emit_conversation(c);
        }
    }

    // ---------- processi ----------

    fn session_file_exists(&self, cwd: &str, session_id: &str) -> bool {
        sessions::session_dirs_for(&self.0.sessions_root, &[cwd.to_string()])
            .iter()
            .any(|d| d.join(format!("{session_id}.jsonl")).is_file())
    }

    /// Avvia il processo della conversazione se non è già vivo (il controllo e l'avvio avvengono sotto lo
    /// stesso lock: due invii simultanei non avviano due processi).
    fn ensure_running(&self, project_id: &str, session_id: &str) -> Result<Arc<Conv>> {
        // prima del lock: la prima risoluzione aspetta l'ambiente della shell di login
        let claude = self.claude_path();
        let mut convs = lock(&self.0.convs);
        if let Some(c) = convs.get(session_id) {
            return Ok(c.clone());
        }
        let claude = claude.ok_or("claude non trovato")?;
        let project = self.project(project_id)?;
        self.0.store.mark_conversation_owned(session_id)?;
        let row = self.owned_conv(project_id, session_id)?;
        if !Path::new(&row.cwd).is_dir() {
            return Err(format!("cartella della conversazione non trovata: {}", row.cwd));
        }
        let mode = if valid_permission_mode(&row.permission_mode) { row.permission_mode.clone() } else { self.default_mode() };
        let mut args: Vec<String> = [
            "-p",
            "--input-format",
            "stream-json",
            "--output-format",
            "stream-json",
            "--verbose",
            "--include-partial-messages",
            "--permission-prompts",
            "host",
            "--permission-prompt-tool",
            "stdio",
            "--permission-mode",
        ]
        .map(String::from)
        .into();
        args.push(mode);
        if let Some(m) = row.model.as_deref().filter(|m| valid_model(m)) {
            args.extend(["--model".into(), m.to_string()]);
        }
        // una sessione mai usata non ha ancora il file: `--resume` fallirebbe
        let flag = if self.session_file_exists(&row.cwd, session_id) { "--resume" } else { "--session-id" };
        args.extend([flag.to_string(), session_id.to_string()]);
        for f in project.folders.iter().filter(|f| **f != row.cwd) {
            args.extend(["--add-dir".into(), f.clone()]);
        }
        let mut child = Command::new(&claude)
            .args(&args)
            .current_dir(&row.cwd)
            .env_clear()
            .envs(&self.0.launch.env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("avvio di claude fallito: {e}"))?;
        let (stdout, stderr) = (child.stdout.take(), child.stderr.take());
        let conv = Arc::new(Conv {
            project_id: project_id.to_string(),
            stdin: Mutex::new(child.stdin.take()),
            rt: Mutex::new(Runtime { state: self.load_state(session_id), pending: HashMap::new(), next_req: 0 }),
        });
        convs.insert(session_id.to_string(), conv.clone());
        drop(convs);

        let stderr_tail = std::thread::spawn(move || read_tail(stderr));
        let (engine, c, sid) = (self.clone(), conv.clone(), session_id.to_string());
        std::thread::spawn(move || engine.pump(&sid, &c, stdout, child, stderr_tail));

        // handshake facoltativo (§2.1): comandi slash e modelli arrivano subito come `ControlResponse`
        let req_id = lock(&conv.rt).request_id();
        let _ = write_line(&conv, &json!({"type": "control_request", "request_id": req_id, "request": {"subtype": "initialize", "hooks": null}}));
        self.emit_upsert(session_id);
        Ok(conv)
    }

    /// Legge stdout fino alla chiusura, poi gestisce l'uscita del processo.
    fn pump(
        &self,
        session_id: &str,
        conv: &Arc<Conv>,
        stdout: Option<std::process::ChildStdout>,
        mut child: Child,
        stderr_tail: std::thread::JoinHandle<String>,
    ) {
        if let Some(out) = stdout {
            let mut reader = BufReader::new(out);
            let mut buf = Vec::new();
            loop {
                buf.clear();
                match reader.read_until(b'\n', &mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => self.handle_line(session_id, conv, &String::from_utf8_lossy(&buf)),
                }
            }
        }
        *lock(&conv.stdin) = None;
        let success = child.wait().is_ok_and(|s| s.success());
        let tail = stderr_tail.join().unwrap_or_default();
        // stato salvato prima di togliere il processo dalla mappa: una ripresa immediata lo rilegge aggiornato
        {
            let mut rt = lock(&conv.rt);
            rt.pending.clear();
            rt.state = next_status(rt.state, StateInput::ProcessExited);
            self.save(session_id, &rt.state, false);
        }
        {
            let mut convs = lock(&self.0.convs);
            if convs.get(session_id).is_some_and(|c| Arc::ptr_eq(c, conv)) {
                convs.remove(session_id);
            }
        }
        if !success {
            let mut json = tail;
            truncate_utf8(&mut json, UNKNOWN_JSON_CAP);
            self.emit_event(&conv.project_id, session_id, &ClaudeEvent::Unknown { label: EXIT_LABEL.into(), json });
        }
        self.emit_upsert(session_id);
    }

    fn handle_line(&self, session_id: &str, conv: &Arc<Conv>, line: &str) {
        for mut ev in parse_line(line) {
            if let ClaudeEvent::Unknown { json, .. } = &mut ev {
                truncate_utf8(json, UNKNOWN_JSON_CAP);
            }
            let mut changed = self.persist_metadata(session_id, &ev);
            let forward = {
                let mut rt = lock(&conv.rt);
                let forward = match &ev {
                    ClaudeEvent::Ask { request_id, .. } => {
                        rt.pending.insert(request_id.clone(), ev.clone());
                        true
                    }
                    // annullamento di una richiesta già chiusa da noi (risposta o interruzione): ignorato
                    ClaudeEvent::AskCancelled { request_id } => rt.pending.remove(request_id).is_some(),
                    ClaudeEvent::TurnEnd { .. } => {
                        if rt.state.status == ConvStatus::Working {
                            rt.pending.clear(); // la macchina degli stati azzera `open_asks`
                        }
                        true
                    }
                    _ => true,
                };
                if forward {
                    let before = rt.state;
                    rt.state = next_status(before, StateInput::Event(&ev));
                    let turn_end = matches!(ev, ClaudeEvent::TurnEnd { .. });
                    if rt.state != before || turn_end {
                        self.save(session_id, &rt.state, turn_end);
                        changed = true;
                    }
                }
                forward
            };
            if forward {
                self.emit_event(&conv.project_id, session_id, &ev);
            }
            if changed {
                self.emit_upsert(session_id);
            }
            if matches!(ev, ClaudeEvent::TurnEnd { .. }) {
                self.0.titles.get().inspect(|q| q.enqueue_if_due(session_id));
            }
        }
    }

    /// Metadati della conversazione portati dagli eventi: modello e modalità (`Init`, `system/status`),
    /// contesto usato (`TurnEnd`). Restituisce true se qualcosa è stato salvato.
    fn persist_metadata(&self, session_id: &str, ev: &ClaudeEvent) -> bool {
        let store = &self.0.store;
        match ev {
            ClaudeEvent::Init { model, permission_mode, .. } => {
                let mut saved = false;
                if valid_model(model) {
                    saved |= store.set_conversation_model(session_id, model).is_ok();
                }
                if valid_permission_mode(permission_mode) {
                    saved |= store.set_conversation_permission_mode(session_id, permission_mode).is_ok();
                }
                saved
            }
            ClaudeEvent::TurnEnd { context_used_pct: Some(p), .. } => store.set_conversation_context(session_id, f64::from(*p)).is_ok(),
            ClaudeEvent::Unknown { label, json } if label == "system/status" => {
                let mode = serde_json::from_str::<Value>(json).ok().and_then(|v| v["permissionMode"].as_str().map(String::from));
                mode.filter(|m| valid_permission_mode(m))
                    .is_some_and(|m| store.set_conversation_permission_mode(session_id, &m).is_ok())
            }
            _ => false,
        }
    }

    /// Immagine da allegare: file esistente dentro la cartella immagini dell'app o dentro il progetto.
    fn load_image(&self, project: &Project, path: &str) -> Result<Value> {
        let real = Path::new(path).canonicalize().map_err(|_| format!("immagine non trovata: {path}"))?;
        let roots = std::iter::once(self.0.image_dir.clone()).chain(project.folders.iter().map(PathBuf::from));
        let allowed = roots.filter_map(|r| r.canonicalize().ok()).any(|r| real.starts_with(&r));
        if !allowed {
            return Err(format!("immagine fuori dalle cartelle consentite: {path}"));
        }
        let ext = real.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
        let media_type = match ext.as_str() {
            "png" => "image/png",
            "jpg" | "jpeg" => "image/jpeg",
            "gif" => "image/gif",
            "webp" => "image/webp",
            _ => return Err(format!("formato immagine non supportato: {path}")),
        };
        let meta = std::fs::metadata(&real).map_err(|e| e.to_string())?;
        if !meta.is_file() || meta.len() > IMAGE_MAX_BYTES {
            return Err(format!("immagine non valida o troppo grande: {path}"));
        }
        let bytes = std::fs::read(&real).map_err(|e| e.to_string())?;
        let data = base64::engine::general_purpose::STANDARD.encode(bytes);
        Ok(json!({"type": "image", "source": {"type": "base64", "media_type": media_type, "data": data}}))
    }
}

#[cfg(test)]
impl Engine {
    /// Chiude lo stdin di tutti i processi (che quindi terminano).
    fn close_all(&self) {
        for c in lock(&self.0.convs).values() {
            *lock(&c.stdin) = None;
        }
    }
}

/// Risposta a una richiesta `can_use_tool` (§5.2–5.4).
fn answer_payload(ask: &ClaudeEvent, choice: &str) -> Result<Value> {
    let ClaudeEvent::Ask { ask_type, input, .. } = ask else { return Err("richiesta non valida".into()) };
    let allow = |input: Value| json!({"behavior": "allow", "updatedInput": input});
    let deny = |message: &str| json!({"behavior": "deny", "message": message});
    match (ask_type, choice) {
        (AskType::Permission, "allow") | (AskType::Plan, "approve") => Ok(allow(input.clone())),
        (AskType::Permission, "deny") => Ok(deny(DENY_MESSAGE)),
        (AskType::Plan, "reject") => Ok(deny(PLAN_REJECT_MESSAGE)),
        (AskType::Question, c) if !c.trim().is_empty() => {
            // la risposta vale per la domanda mostrata (la prima); testo libero accettato com'è
            let question = input.pointer("/questions/0/question").and_then(Value::as_str).ok_or("domanda senza testo")?;
            let mut updated = input.clone();
            let obj = updated.as_object_mut().ok_or("input della domanda non valido")?;
            obj.insert("answers".into(), json!({ question: c }));
            Ok(allow(updated))
        }
        _ => Err(format!("risposta non valida: {choice}")),
    }
}

fn write_line(conv: &Conv, v: &Value) -> Result<()> {
    let mut line = serde_json::to_string(v).map_err(|e| e.to_string())?;
    line.push('\n');
    let mut stdin = lock(&conv.stdin);
    let pipe = stdin.as_mut().ok_or("il processo della conversazione non è attivo")?;
    pipe.write_all(line.as_bytes()).and_then(|_| pipe.flush()).map_err(|e| format!("scrittura verso claude fallita: {e}"))
}

/// Ultimi `STDERR_TAIL` byte di stderr.
fn read_tail(stderr: Option<std::process::ChildStderr>) -> String {
    let Some(mut err) = stderr else { return String::new() };
    let mut tail: Vec<u8> = Vec::new();
    let mut buf = [0u8; 8192];
    while let Ok(n) = err.read(&mut buf) {
        if n == 0 {
            break;
        }
        tail.extend_from_slice(&buf[..n]);
        if tail.len() > STDERR_TAIL {
            tail.drain(..tail.len() - STDERR_TAIL);
        }
    }
    String::from_utf8_lossy(&tail).into_owned()
}

// ---------- integrazione con Tauri ----------

use tauri::{AppHandle, Emitter, Manager, WebviewWindow};

/// Emette verso la finestra del progetto; se è chiusa l'evento si perde (lo stato resta salvato).
pub struct TauriSink(pub AppHandle);

impl EventSink for TauriSink {
    fn emit(&self, project_id: &str, event: &str, payload: Value) {
        let _ = self.0.emit_to(crate::projects::window_label(project_id).as_str(), event, payload);
    }
}

/// Ambiente di lancio dell'app: shell di login, calcolata in sottofondo all'avvio.
pub fn app_launch() -> Arc<LaunchCell> {
    let cell: Arc<LaunchCell> = Arc::new(LazyLock::new(Box::new(|| {
        let env = super::locate::login_env();
        Launch { claude: super::locate::find_claude(&env), env }
    })));
    let warm = cell.clone();
    std::thread::spawn(move || {
        LazyLock::force(&warm);
    });
    cell
}

fn window_project_id(window: &WebviewWindow) -> Result<String> {
    window
        .label()
        .strip_prefix(crate::projects::PROJECT_PREFIX)
        .map(String::from)
        .ok_or_else(|| "questa finestra non è un progetto".into())
}

/// Esegue `f` fuori dal thread principale con il motore e il progetto della finestra chiamante.
async fn with_engine<T: Send + 'static>(
    app: AppHandle,
    window: WebviewWindow,
    f: impl FnOnce(&Engine, &str) -> Result<T> + Send + 'static,
) -> Result<T> {
    let project_id = window_project_id(&window)?;
    let engine = app.state::<Engine>().inner().clone();
    tauri::async_runtime::spawn_blocking(move || f(&engine, &project_id)).await.map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn claude_check(app: AppHandle) -> Result<ClaudeCheck> {
    let engine = app.state::<Engine>().inner().clone();
    tauri::async_runtime::spawn_blocking(move || engine.claude_check()).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn conv_start(app: AppHandle, window: WebviewWindow, project_id: String, opts: Option<StartOpts>) -> Result<String> {
    with_engine(app, window, move |e, pid| {
        if pid != project_id {
            return Err("progetto non della finestra chiamante".into());
        }
        e.start(pid, opts.unwrap_or_default())
    })
    .await
}

#[tauri::command]
pub async fn conv_resume(app: AppHandle, window: WebviewWindow, session_id: String) -> Result<()> {
    with_engine(app, window, move |e, pid| e.resume(pid, &session_id)).await
}

#[tauri::command]
pub async fn conv_send(app: AppHandle, window: WebviewWindow, session_id: String, text: String, image_paths: Vec<String>) -> Result<()> {
    with_engine(app, window, move |e, pid| e.send(pid, &session_id, &text, &image_paths)).await
}

#[tauri::command]
pub async fn conv_answer(app: AppHandle, window: WebviewWindow, session_id: String, request_id: String, choice: String) -> Result<()> {
    with_engine(app, window, move |e, pid| e.answer(pid, &session_id, &request_id, &choice)).await
}

#[tauri::command]
pub async fn conv_interrupt(app: AppHandle, window: WebviewWindow, session_id: String) -> Result<()> {
    with_engine(app, window, move |e, pid| e.interrupt(pid, &session_id)).await
}

#[tauri::command]
pub async fn conv_set_model(app: AppHandle, window: WebviewWindow, session_id: String, model: String) -> Result<()> {
    with_engine(app, window, move |e, pid| e.set_model(pid, &session_id, &model)).await
}

#[tauri::command]
pub async fn conv_set_permission_mode(app: AppHandle, window: WebviewWindow, session_id: String, mode: String) -> Result<()> {
    with_engine(app, window, move |e, pid| e.set_permission_mode(pid, &session_id, &mode)).await
}

#[tauri::command]
pub async fn conv_mark_viewed(app: AppHandle, window: WebviewWindow, session_id: String) -> Result<()> {
    with_engine(app, window, move |e, pid| e.mark_viewed(pid, &session_id)).await
}

#[tauri::command]
pub async fn conv_archive(app: AppHandle, window: WebviewWindow, session_id: String, archived: bool) -> Result<()> {
    with_engine(app, window, move |e, pid| e.archive(pid, &session_id, archived)).await
}

#[tauri::command]
pub async fn conv_running(app: AppHandle, window: WebviewWindow, project_id: String) -> Result<Vec<String>> {
    with_engine(app, window, move |e, pid| {
        if pid != project_id {
            return Err("progetto non della finestra chiamante".into());
        }
        Ok(e.running(pid))
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Project;
    use serde_json::json;
    use std::path::Path;
    use std::time::{Duration, Instant};

    #[derive(Default)]
    struct Capture(Mutex<Vec<(String, String, Value)>>);
    impl EventSink for Capture {
        fn emit(&self, project_id: &str, event: &str, payload: Value) {
            self.0.lock().unwrap().push((project_id.into(), event.into(), payload));
        }
    }
    impl Capture {
        fn events(&self, session_id: &str) -> Vec<Value> {
            self.0
                .lock()
                .unwrap()
                .iter()
                .filter(|(_, e, p)| e == "conv:event" && p["sessionId"] == session_id)
                .map(|(_, _, p)| p["event"].clone())
                .collect()
        }
        fn has_kind(&self, session_id: &str, kind: &str) -> bool {
            self.events(session_id).iter().any(|e| e["kind"] == kind)
        }
    }

    struct Env {
        _dir: tempfile::TempDir,
        out: PathBuf,
        folders: Vec<PathBuf>,
        sessions_root: PathBuf,
        images: PathBuf,
        store: Store,
        sink: Arc<Capture>,
        engine: Engine,
    }

    /// I finti terminano a fine test: su macOS una pipe può finire ereditata da un processo avviato in
    /// parallelo da un altro test, e resterebbe aperta finché quel processo vive.
    impl Drop for Env {
        fn drop(&mut self) {
            self.engine.close_all();
        }
    }

    fn fixture(name: &str) -> String {
        format!("{}/tests/fixtures/stream/{name}.jsonl", env!("CARGO_MANIFEST_DIR"))
    }

    fn setup(vars: &[(&str, &str)]) -> Env {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let (out, sessions_root, images) = (root.join("out"), root.join("sessions"), root.join("images"));
        let folders = vec![root.join("principale"), root.join("seconda")];
        for d in [&out, &sessions_root, &images, &folders[0], &folders[1]] {
            std::fs::create_dir_all(d).unwrap();
        }
        let store = Store::open(&root.join("t.db")).unwrap();
        store
            .upsert_project(&Project {
                id: "p1".into(),
                name: "proj".into(),
                folders: folders.iter().map(|f| f.to_string_lossy().into_owned()).collect(),
            })
            .unwrap();
        store
            .upsert_project(&Project { id: "p2".into(), name: "altro".into(), folders: vec![folders[1].to_string_lossy().into_owned()] })
            .unwrap();
        let mut env: HashMap<String, String> =
            [("PATH".to_string(), "/usr/bin:/bin".to_string()), ("FAKE_CLAUDE_OUT".to_string(), out.to_string_lossy().into_owned())].into();
        for (k, v) in vars {
            env.insert(k.to_string(), v.to_string());
        }
        let fake = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fake_claude.sh");
        let launch: Arc<LaunchCell> = Arc::new(LazyLock::new(Box::new(move || Launch { claude: Some(fake), env })));
        let sink = Arc::new(Capture::default());
        let engine = Engine::new(launch, store.clone(), sink.clone(), sessions_root.clone(), images.clone());
        Env { _dir: dir, out, folders, sessions_root, images, store, sink, engine }
    }

    fn wait_until(what: &str, mut cond: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !cond() {
            assert!(Instant::now() < deadline, "tempo scaduto aspettando: {what}");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn read(p: &Path) -> String {
        std::fs::read_to_string(p).unwrap_or_default()
    }

    fn stdin_lines(env: &Env, sid: &str) -> Vec<Value> {
        read(&env.out.join(format!("{sid}.stdin"))).lines().filter_map(|l| serde_json::from_str(l).ok()).collect()
    }

    fn status(env: &Env, sid: &str) -> String {
        env.store.get_conversation(sid).unwrap().unwrap().status
    }

    #[test]
    fn start_passes_cwd_add_dirs_and_permission_mode() {
        let env = setup(&[]);
        let sid = env.engine.start("p1", StartOpts::default()).unwrap();
        let args_file = env.out.join(format!("{sid}.args"));
        wait_until("argomenti scritti", || read(&env.out.join(format!("{sid}.cwd"))).ends_with('\n'));
        let args: Vec<String> = read(&args_file).lines().map(String::from).collect();
        let pair = |flag: &str, val: &str| args.windows(2).any(|w| w[0] == flag && w[1] == val);
        assert!(pair("--add-dir", &env.folders[1].to_string_lossy()), "{args:?}");
        assert!(pair("--permission-mode", "bypassPermissions"), "{args:?}");
        assert!(pair("--session-id", &sid), "{args:?}");
        assert!(pair("--permission-prompt-tool", "stdio"), "{args:?}");
        for f in ["-p", "--verbose", "--include-partial-messages"] {
            assert!(args.iter().any(|a| a == f), "{f} manca in {args:?}");
        }
        assert!(pair("--input-format", "stream-json") && pair("--output-format", "stream-json"));
        assert!(!args.iter().any(|a| a == "--safe-mode" || a == "--resume"));
        assert_eq!(read(&env.out.join(format!("{sid}.cwd"))).trim(), env.folders[0].to_string_lossy());
        let conv = env.store.get_conversation(&sid).unwrap().unwrap();
        assert!(conv.owned && conv.project_id == "p1" && conv.permission_mode == "bypassPermissions");
        assert_eq!(env.engine.running("p1"), vec![sid.clone()]);
        assert!(env.engine.running("p2").is_empty());
        // handshake `initialize` inviato all'avvio
        wait_until("initialize", || stdin_lines(&env, &sid).iter().any(|l| l["request"]["subtype"] == "initialize"));
    }

    #[test]
    fn start_uses_requested_model_and_mode_and_validates_them() {
        let env = setup(&[]);
        let opts = StartOpts { model: Some("sonnet".into()), permission_mode: Some("plan".into()) };
        let sid = env.engine.start("p1", opts).unwrap();
        wait_until("argomenti", || read(&env.out.join(format!("{sid}.cwd"))).ends_with('\n'));
        let args = read(&env.out.join(format!("{sid}.args")));
        assert!(args.contains("--model\nsonnet\n") && args.contains("--permission-mode\nplan\n"), "{args}");
        let bad_mode = StartOpts { permission_mode: Some("tutto; rm -rf".into()), ..Default::default() };
        assert!(env.engine.start("p1", bad_mode).is_err());
        let bad_model = StartOpts { model: Some("--dangerously".into()), ..Default::default() };
        assert!(env.engine.start("p1", bad_model).is_err());
        assert!(env.engine.start("nessuno", StartOpts::default()).is_err());
    }

    #[test]
    fn send_emits_events_and_ends_waiting_until_viewed() {
        let fx = fixture("simple-turn");
        let env = setup(&[("FAKE_CLAUDE_FIXTURE", &fx)]);
        let sid = env.engine.start("p1", StartOpts::default()).unwrap();
        env.engine.send("p1", &sid, "ciao", &[]).unwrap();
        wait_until("fine turno", || env.sink.has_kind(&sid, "turnEnd"));
        wait_until("stato waiting", || status(&env, &sid) == "waiting");
        assert!(env.sink.has_kind(&sid, "init") && env.sink.has_kind(&sid, "textDelta"));
        let user = stdin_lines(&env, &sid).into_iter().find(|l| l["type"] == "user").unwrap();
        assert_eq!(user["message"], json!({"role": "user", "content": "ciao"}));
        let conv = env.store.get_conversation(&sid).unwrap().unwrap();
        assert_eq!(conv.model.as_deref(), Some("claude-haiku-5-5")); // da `Init`
        assert!(conv.context_used_pct.is_some());
        let upserts = env.sink.0.lock().unwrap().iter().filter(|(p, e, v)| p == "p1" && e == "conv:upsert" && v["sessionId"] == sid.as_str()).count();
        assert!(upserts >= 2);
        env.engine.mark_viewed("p1", &sid).unwrap();
        assert_eq!(status(&env, &sid), "idle");
        assert!(env.engine.mark_viewed("p2", &sid).is_err());
    }

    #[test]
    fn turn_end_hands_the_conversation_to_the_title_queue() {
        struct Spy;
        impl super::super::titles::TitleRunner for Spy {
            fn run(&self, _prompt: &str) -> Option<String> {
                Some(r#"{"title":"Saluto iniziale","summary":"Si e' salutato."}"#.into())
            }
        }
        let fx = fixture("simple-turn");
        let env = setup(&[("FAKE_CLAUDE_FIXTURE", &fx)]);
        env.engine.set_titles(super::super::titles::TitleQueue::new(
            env.store.clone(),
            env.sink.clone(),
            env.sessions_root.clone(),
            Arc::new(Spy),
        ));
        let sid = env.engine.start("p1", StartOpts::default()).unwrap();
        // il finto `claude` non scrive il file di sessione: ne serve uno con il testo da riassumere
        let cwd = env.store.get_conversation(&sid).unwrap().unwrap().cwd;
        let dir = env.sessions_root.join(sessions::encode_project_dir(&cwd));
        std::fs::create_dir_all(&dir).unwrap();
        let line = json!({"type":"user","uuid":"u1","isSidechain":false,"cwd":cwd,"sessionId":sid,"timestamp":"2026-11-11T09:00:01.000Z","promptSource":"sdk","message":{"role":"user","content":"ciao"}});
        std::fs::write(dir.join(format!("{sid}.jsonl")), format!("{line}\n")).unwrap();
        env.engine.send("p1", &sid, "ciao", &[]).unwrap();
        wait_until("titolo generato", || env.store.get_conversation(&sid).unwrap().unwrap().title == "Saluto iniziale");
        assert_eq!(env.store.title_turn(&sid).unwrap(), Some(1));
    }

    #[test]
    fn permission_fixture_sets_waiting_then_answer_writes_to_stdin() {
        let fx = fixture("permission-request");
        let env = setup(&[("FAKE_CLAUDE_FIXTURE", &fx)]);
        let sid = env.engine.start("p1", StartOpts::default()).unwrap();
        env.engine.send("p1", &sid, "scrivi note.txt", &[]).unwrap();
        wait_until("richiesta", || env.sink.has_kind(&sid, "ask"));
        wait_until("stato waiting", || status(&env, &sid) == "waiting");
        let rid = "c6b1b64f-8800-4372-b30a-5b4c4e3c465b";
        assert!(env.engine.answer("p1", &sid, rid, "forse").is_err()); // scelta non valida: resta pendente
        assert!(env.engine.answer("p1", &sid, "sconosciuta", "allow").is_err());
        env.engine.answer("p1", &sid, rid, "allow").unwrap();
        let allow = json!({"type": "control_response", "response": {"subtype": "success", "request_id": rid,
            "response": {"behavior": "allow", "updatedInput": {"file_path": "/Users/dev/proj/note.txt", "content": "hi"}}}});
        wait_until("risposta allow su stdin", || stdin_lines(&env, &sid).contains(&allow));
        // seconda richiesta (Bash): negata
        let rid2 = "0477865f-c919-47c1-8f1e-8c069e660596";
        wait_until("seconda richiesta", || env.sink.events(&sid).iter().any(|e| e["requestId"] == rid2));
        env.engine.answer("p1", &sid, rid2, "deny").unwrap();
        let deny = json!({"type": "control_response", "response": {"subtype": "success", "request_id": rid2,
            "response": {"behavior": "deny", "message": "The user denied this action."}}});
        wait_until("risposta deny su stdin", || stdin_lines(&env, &sid).contains(&deny));
        assert!(env.engine.answer("p1", &sid, rid2, "deny").is_err()); // già risposta
    }

    #[test]
    fn question_and_plan_answers_follow_the_protocol() {
        let fx = fixture("ask-user-question");
        let env = setup(&[("FAKE_CLAUDE_FIXTURE", &fx)]);
        let sid = env.engine.start("p1", StartOpts::default()).unwrap();
        env.engine.send("p1", &sid, "chiedimi un colore", &[]).unwrap();
        wait_until("domanda", || env.sink.has_kind(&sid, "ask"));
        let ask = env.sink.events(&sid).into_iter().find(|e| e["kind"] == "ask").unwrap();
        let rid = ask["requestId"].as_str().unwrap().to_string();
        env.engine.answer("p1", &sid, &rid, "Blue").unwrap();
        wait_until("risposta domanda", || {
            stdin_lines(&env, &sid).iter().any(|l| {
                l["response"]["request_id"] == rid.as_str()
                    && l["response"]["response"]["behavior"] == "allow"
                    && l["response"]["response"]["updatedInput"]["answers"] == json!({"Which color do you prefer?": "Blue"})
                    && l["response"]["response"]["updatedInput"]["questions"] == ask["input"]["questions"]
            })
        });

        let fx = fixture("plan-mode");
        let env = setup(&[("FAKE_CLAUDE_FIXTURE", &fx)]);
        let sid = env.engine.start("p1", StartOpts::default()).unwrap();
        env.engine.send("p1", &sid, "fai un piano", &[]).unwrap();
        wait_until("piano", || env.sink.events(&sid).iter().any(|e| e["askType"] == "plan"));
        let ask = env.sink.events(&sid).into_iter().find(|e| e["askType"] == "plan").unwrap();
        let rid = ask["requestId"].as_str().unwrap().to_string();
        assert!(env.engine.answer("p1", &sid, &rid, "allow").is_err()); // per il piano: approve/reject
        env.engine.answer("p1", &sid, &rid, "approve").unwrap();
        wait_until("approvazione", || {
            stdin_lines(&env, &sid).iter().any(|l| {
                l["response"]["request_id"] == rid.as_str()
                    && l["response"]["response"] == json!({"behavior": "allow", "updatedInput": ask["input"]})
            })
        });
    }

    #[test]
    fn interrupt_with_pending_ask_closes_it() {
        let fx = fixture("permission-request");
        let env = setup(&[("FAKE_CLAUDE_FIXTURE", &fx)]);
        let sid = env.engine.start("p1", StartOpts::default()).unwrap();
        env.engine.send("p1", &sid, "scrivi", &[]).unwrap();
        wait_until("richiesta", || env.sink.has_kind(&sid, "ask"));
        env.engine.interrupt("p1", &sid).unwrap();
        wait_until("interrupt su stdin", || stdin_lines(&env, &sid).iter().any(|l| l["request"]["subtype"] == "interrupt"));
        assert!(env.sink.events(&sid).iter().any(|e| e["kind"] == "askCancelled" && e["requestId"] == "c6b1b64f-8800-4372-b30a-5b4c4e3c465b"));
        assert_eq!(status(&env, &sid), "working"); // nessuna richiesta aperta: il prossimo `result` chiude il turno
        assert!(env.engine.answer("p1", &sid, "c6b1b64f-8800-4372-b30a-5b4c4e3c465b", "allow").is_err());
    }

    #[test]
    fn crash_mid_turn_sets_waiting_and_keeps_resumable() {
        let fx = fixture("simple-turn");
        let env = setup(&[("FAKE_CLAUDE_FIXTURE", &fx), ("FAKE_CLAUDE_CRASH_AFTER", "4")]);
        let sid = env.engine.start("p1", StartOpts::default()).unwrap();
        env.engine.send("p1", &sid, "ciao", &[]).unwrap();
        wait_until("processo terminato", || env.engine.running("p1").is_empty());
        wait_until("stato waiting", || status(&env, &sid) == "waiting");
        wait_until("evento di uscita", || env.sink.events(&sid).iter().any(|e| e["label"] == "Processo terminato"));
        let ev = env.sink.events(&sid).into_iter().find(|e| e["label"] == "Processo terminato").unwrap();
        assert_eq!(ev["kind"], "unknown");
        assert!(ev["json"].as_str().unwrap().contains("errore finto"));
        assert!(!env.sink.has_kind(&sid, "turnEnd"));
        // la CLI ha scritto la sessione: la ripresa usa --resume
        let cwd = env.folders[0].to_string_lossy().into_owned();
        let dir = env.sessions_root.join(crate::claude::sessions::encode_project_dir(&cwd));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(format!("{sid}.jsonl")), "{}\n").unwrap();
        std::fs::remove_file(env.out.join(format!("{sid}.cwd"))).unwrap();
        env.engine.resume("p1", &sid).unwrap();
        wait_until("ripreso", || read(&env.out.join(format!("{sid}.cwd"))).ends_with('\n'));
        let args = read(&env.out.join(format!("{sid}.args")));
        assert!(args.contains(&format!("--resume\n{sid}\n")) && !args.contains("--session-id"), "{args}");
        assert_eq!(env.engine.running("p1"), vec![sid.clone()]);
        assert_eq!(status(&env, &sid), "waiting");
    }

    #[test]
    fn two_conversations_run_in_parallel() {
        let fx = fixture("simple-turn");
        let env = setup(&[("FAKE_CLAUDE_FIXTURE", &fx), ("FAKE_CLAUDE_BARRIER", "2")]);
        let a = env.engine.start("p1", StartOpts::default()).unwrap();
        let b = env.engine.start("p1", StartOpts::default()).unwrap();
        assert_ne!(a, b);
        let mut running = env.engine.running("p1");
        running.sort();
        let mut want = vec![a.clone(), b.clone()];
        want.sort();
        assert_eq!(running, want);
        env.engine.send("p1", &a, "uno", &[]).unwrap();
        env.engine.send("p1", &b, "due", &[]).unwrap();
        for sid in [&a, &b] {
            wait_until("stato waiting", || status(&env, sid) == "waiting");
            assert_eq!(read(&env.out.join(format!("{sid}.barrier"))).trim(), "ok");
            assert!(env.sink.has_kind(sid, "turnEnd"));
        }
    }

    #[test]
    fn send_resumes_a_stopped_conversation_and_checks_images() {
        let fx = fixture("simple-turn");
        let env = setup(&[("FAKE_CLAUDE_FIXTURE", &fx)]);
        // sessione importata (non pilotata): conv_send la riprende
        let sid = "44444444-4444-4444-8444-444444444444";
        let cwd = env.folders[0].to_string_lossy().into_owned();
        env.store.upsert_synced_conversation("p1", sid, &cwd, "vecchia", 0, 3).unwrap();
        let outside = env._dir.path().join("fuori.png");
        std::fs::write(&outside, [0x89, b'P', b'N', b'G']).unwrap();
        assert!(env.engine.send("p1", sid, "guarda", &[outside.to_string_lossy().into_owned()]).is_err());
        assert!(env.engine.send("p1", sid, "guarda", &["/etc/hosts".into()]).is_err());
        assert!(env.engine.running("p1").is_empty());
        let img = env.images.join("a.png");
        std::fs::write(&img, [0x89, b'P', b'N', b'G']).unwrap();
        env.engine.send("p1", sid, "guarda", &[img.to_string_lossy().into_owned()]).unwrap();
        wait_until("stato waiting", || status(&env, sid) == "waiting");
        assert!(env.store.get_conversation(sid).unwrap().unwrap().owned);
        let user = stdin_lines(&env, sid).into_iter().find(|l| l["type"] == "user").unwrap();
        let content = &user["message"]["content"];
        assert_eq!(content[0], json!({"type": "text", "text": "guarda"}));
        assert_eq!(content[1]["source"]["media_type"], "image/png");
        assert_eq!(content[1]["source"]["data"], "iVBORw==");
        assert!(env.engine.send("p2", sid, "x", &[]).is_err());
    }

    #[test]
    fn model_and_mode_are_validated_persisted_and_sent_live() {
        let env = setup(&[]);
        let sid = env.engine.start("p1", StartOpts::default()).unwrap();
        assert!(env.engine.set_model("p1", &sid, "a b").is_err());
        assert!(env.engine.set_permission_mode("p1", &sid, "yolo").is_err());
        env.engine.set_model("p1", &sid, "opus").unwrap();
        env.engine.set_permission_mode("p1", &sid, "acceptEdits").unwrap();
        let conv = env.store.get_conversation(&sid).unwrap().unwrap();
        assert_eq!((conv.model.as_deref(), conv.permission_mode.as_str()), (Some("opus"), "acceptEdits"));
        wait_until("controlli su stdin", || {
            let l = stdin_lines(&env, &sid);
            l.iter().any(|l| l["request"] == json!({"subtype": "set_model", "model": "opus"}))
                && l.iter().any(|l| l["request"] == json!({"subtype": "set_permission_mode", "mode": "acceptEdits"}))
        });
        assert!(env.engine.set_model("p2", &sid, "opus").is_err());
    }

    #[test]
    fn unknown_json_is_truncated_on_char_boundary() {
        let mut s = format!("{}è{}", "a".repeat(4095), "b".repeat(100));
        truncate_utf8(&mut s, 4096);
        assert_eq!(s, "a".repeat(4095));
        let mut short = "ok".to_string();
        truncate_utf8(&mut short, 4096);
        assert_eq!(short, "ok");
    }
}
