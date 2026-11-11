//! Motore delle conversazioni: un processo `claude` per conversazione, stream-json su stdin/stdout
//! (docs/claude-protocol.md), stato salvato nello store ed eventi inoltrati alla finestra del progetto.
use super::protocol::{parse_line, AskType, ClaudeEvent};
use super::state::{next_status, ConvState, StateInput};
use super::{sessions, ConvStatus, Conversation};
use crate::store::{Project, Result, Store};
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{mpsc, Arc, Condvar, LazyLock, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

/// Tetto per `Unknown.json` negli eventi (le righe utente riemesse possono contenere immagini base64).
const UNKNOWN_JSON_CAP: usize = 4096;
/// Coda di stderr conservata per l'evento "Processo terminato".
const STDERR_TAIL: usize = 4096;
pub(super) const IMAGE_MAX_BYTES: u64 = 20 * 1024 * 1024;
const IMAGES_MAX_PER_MESSAGE: usize = 10;
const IMAGES_MAX_TOTAL_BYTES: u64 = 30 * 1024 * 1024;
/// `stop`: attesa dopo la richiesta di chiusura prima di uccidere il gruppo del processo.
const STOP_GRACE: Duration = Duration::from_secs(2);
/// `stop`: attesa dopo l'uccisione del gruppo perché l'uscita sia gestita.
const KILL_WAIT: Duration = Duration::from_secs(2);
/// `ensure_running`: attesa massima perché un processo in chiusura sia del tutto terminato prima di sostituirlo.
const REPLACE_WAIT: Duration = Duration::from_secs(5);
/// Dopo l'uscita del processo, quanto aspettare ancora la fine dello stdout (un discendente può tenerlo aperto).
const STDOUT_GRACE: Duration = Duration::from_millis(500);
/// Dopo l'uscita di un nostro processo il file di sessione è stato scritto da noi: niente avviso "altrove".
const OWN_WRITE_WINDOW: Duration = Duration::from_secs(60);
pub const PERMISSION_MODES: &[&str] = &["default", "acceptEdits", "auto", "bypassPermissions", "manual", "dontAsk", "plan"];
const FALLBACK_PERMISSION_MODE: &str = "bypassPermissions";
const DENY_MESSAGE: &str = "The user denied this action.";
const PLAN_REJECT_MESSAGE: &str = "The user rejected the plan.";
const EXIT_LABEL: &str = "Processo terminato";
pub const INTERRUPTED_LABEL: &str = "Turno interrotto — Riprendi";

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
    /// pid = pgid (il processo parte in una sessione propria).
    pid: u32,
    stdin: Mutex<Option<ChildStdin>>,
    rt: Mutex<Runtime>,
    /// Segnala `finished` (insieme a `rt`).
    ended: Condvar,
}

impl Conv {
    /// Processo terminato o stdin chiuso: non più utilizzabile, `ensure_running` lo sostituisce.
    /// Guarda solo `rt`, mai il mutex di stdin (che uno scrittore bloccato può tenere a lungo).
    fn is_dead(&self) -> bool {
        let rt = lock(&self.rt);
        rt.closed || rt.reaped || rt.stopping
    }
}

struct Runtime {
    state: ConvState,
    /// Richieste aperte: request id → evento `Ask` (serve `tool_name`/`input` per rispondere).
    /// Il numero d'ordine di arrivo serve a restituirle dalla più vecchia (`pending_asks`).
    pending: HashMap<String, (u64, ClaudeEvent)>,
    ask_seq: u64,
    next_req: u64,
    /// Stdin dato per chiuso (scrittura fallita o uscita): il processo non accetta più messaggi.
    closed: bool,
    /// `stop` in corso: chi vuole riprendere la sessione aspetta che il processo sia del tutto terminato.
    stopping: bool,
    /// Il processo è uscito ed è stato raccolto (`wait`): il suo pid non va più usato.
    reaped: bool,
    /// Uscita gestita (stato salvato, eventi emessi): le righe stdout tardive vengono ignorate.
    finished: bool,
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
    /// Ultima uscita di un nostro processo per sessione (solo in memoria).
    exits: Mutex<HashMap<String, Instant>>,
    /// Sessioni con un turno rimasto a metà al riavvio dell'app; finché non si invia un nuovo messaggio la
    /// conversazione mostra "Turno interrotto — Riprendi" (solo in memoria: dopo un'altra sessione dell'app il
    /// turno non è più "appena interrotto").
    interrupted: Mutex<HashSet<String>>,
}

/// Stato condiviso dell'app (non della finestra): i processi sopravvivono alla chiusura della finestra.
#[derive(Clone)]
pub struct Engine(Arc<Inner>);

impl Engine {
    pub fn new(launch: Arc<LaunchCell>, store: Store, sink: Arc<dyn EventSink>, sessions_root: PathBuf, image_dir: PathBuf) -> Engine {
        let interrupted = Self::recover(&store);
        let inner = Arc::new(Inner {
            launch,
            store,
            sink,
            sessions_root,
            image_dir,
            convs: Mutex::new(HashMap::new()),
            titles: OnceLock::new(),
            exits: Mutex::new(HashMap::new()),
            interrupted: Mutex::new(interrupted),
        });
        Engine(inner)
    }

    /// Avvio dell'app: nessun processo è vivo e nessun turno riparte da solo. Le conversazioni rimaste
    /// `working` diventano `waiting`; restituisce i loro id (marcatore "Turno interrotto").
    fn recover(store: &Store) -> HashSet<String> {
        store.reset_working_conversations().unwrap_or_default().into_iter().collect()
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

    /// Il file della sessione può essere stato scritto da noi: processo vivo, o uscito da meno di 60 s.
    /// Serve a non mostrare "forse aperta altrove" per le nostre stesse scritture.
    pub fn is_ours(&self, session_id: &str) -> bool {
        self.is_running(session_id)
            || lock(&self.0.exits).get(session_id).is_some_and(|t| t.elapsed() < OWN_WRITE_WINDOW)
    }

    /// Richieste ancora aperte della conversazione (eventi `Ask`, dalla più vecchia): servono a chi apre la
    /// scheda dopo che la richiesta è già stata emessa.
    pub fn pending_asks(&self, project_id: &str, session_id: &str) -> Result<Vec<ClaudeEvent>> {
        self.owned_conv(project_id, session_id)?;
        let Some(conv) = self.live(session_id) else { return Ok(self.interrupted_marker(session_id)) };
        let rt = lock(&conv.rt);
        let mut asks: Vec<&(u64, ClaudeEvent)> = rt.pending.values().collect();
        asks.sort_by_key(|(seq, _)| *seq);
        Ok(asks.into_iter().map(|(_, ev)| ev.clone()).collect())
    }

    /// La conversazione ha un processo vivo con almeno una richiesta aperta (permesso, domanda, piano).
    pub fn has_open_ask(&self, session_id: &str) -> bool {
        self.live(session_id).is_some_and(|c| !lock(&c.rt).pending.is_empty())
    }

    /// Elemento "Turno interrotto — Riprendi" per una conversazione il cui turno era a metà all'avvio.
    fn interrupted_marker(&self, session_id: &str) -> Vec<ClaudeEvent> {
        if lock(&self.0.interrupted).contains(session_id) {
            vec![ClaudeEvent::Unknown { label: INTERRUPTED_LABEL.into(), json: String::new() }]
        } else {
            vec![]
        }
    }

    /// Sessioni con un processo vivo, di tutti i progetti.
    pub fn live_ids(&self) -> Vec<String> {
        lock(&self.0.convs).keys().cloned().collect()
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
        self.claude_path().ok_or("claude non trovato")?;
        let session_id = uuid::Uuid::new_v4().to_string();
        self.0.store.insert_owned_conversation(project_id, &session_id, cwd, opts.model.as_deref(), &mode)?;
        if let Err(e) = self.ensure_running(project_id, &session_id) {
            // nessuna conversazione vuota lasciata indietro
            let _ = self.0.store.delete_conversation(&session_id);
            return Err(e);
        }
        Ok(session_id)
    }

    pub fn resume(&self, project_id: &str, session_id: &str) -> Result<()> {
        self.owned_conv(project_id, session_id)?;
        self.ensure_running(project_id, session_id).map(|_| ())
    }

    pub fn send(&self, project_id: &str, session_id: &str, text: &str, image_paths: &[String]) -> Result<()> {
        self.owned_conv(project_id, session_id)?;
        let project = self.project(project_id)?;
        if image_paths.len() > IMAGES_MAX_PER_MESSAGE {
            return Err(format!("al massimo {IMAGES_MAX_PER_MESSAGE} immagini per messaggio"));
        }
        let mut total = 0u64;
        let mut images = Vec::new();
        for p in image_paths {
            let (img, size) = self.load_image(&project, p)?;
            total += size;
            if total > IMAGES_MAX_TOTAL_BYTES {
                return Err(format!("immagini troppo grandi: al massimo {} MB per messaggio", IMAGES_MAX_TOTAL_BYTES / (1024 * 1024)));
            }
            images.push(img);
        }
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
        let msg = json!({"type": "user", "session_id": "", "parent_tool_use_id": null,
                         "message": {"role": "user", "content": content}});
        let conv = self.ensure_running(project_id, session_id)?;
        // lo stato passa a `working` prima della scrittura: un `result` velocissimo non deve arrivare prima
        self.apply(session_id, Some(&conv), StateInput::UserSent, true);
        if write_line(&conv, &msg).is_ok() {
            lock(&self.0.interrupted).remove(session_id);
            return Ok(());
        }
        // processo appena uscito ma non ancora ripulito: lo si dà per morto e si riprende la sessione
        // (non se lo si sta fermando di proposito)
        {
            let mut rt = lock(&conv.rt);
            if rt.stopping {
                return Err("la conversazione è stata fermata".into());
            }
            rt.closed = true;
        }
        let conv = self.ensure_running(project_id, session_id)?;
        self.apply(session_id, Some(&conv), StateInput::UserSent, true);
        write_line(&conv, &msg)
            .inspect_err(|_| self.apply(session_id, Some(&conv), StateInput::ProcessExited, false))
            .map(|()| {
                lock(&self.0.interrupted).remove(session_id);
            })
    }

    pub fn answer(&self, project_id: &str, session_id: &str, request_id: &str, choice: &str) -> Result<()> {
        self.owned_conv(project_id, session_id)?;
        let conv = self.live(session_id).ok_or("la conversazione non è attiva")?;
        // la richiesta si chiude prima della scrittura (la CLI può proseguire subito), ma senza tenere il lock
        // durante la scrittura: se fallisce torna aperta
        let (ask, response) = {
            let mut rt = lock(&conv.rt);
            let (_, ask) = rt.pending.get(request_id).ok_or("richiesta non più aperta")?;
            let response = answer_payload(ask, choice)?;
            let (_, ask) = rt.pending.remove(request_id).ok_or("richiesta non più aperta")?;
            rt.state = next_status(rt.state, StateInput::AskAnswered);
            self.save(session_id, &rt.state, false);
            (ask, response)
        };
        let written = write_line(
            &conv,
            &json!({"type": "control_response", "response": {"subtype": "success", "request_id": request_id, "response": response}}),
        );
        if written.is_err() {
            let mut rt = lock(&conv.rt);
            if !rt.finished {
                rt.state = next_status(rt.state, StateInput::Event(&ask));
                rt.ask_seq += 1;
                let seq = rt.ask_seq;
                rt.pending.insert(request_id.to_string(), (seq, ask));
                self.save(session_id, &rt.state, false);
            }
        }
        self.emit_upsert(session_id);
        written
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
        if archived {
            self.stop(session_id);
        }
        Ok(())
    }

    /// Messaggio di controllo a processo vivo (§8); senza processo il valore salvato vale alla ripresa.
    fn control(&self, session_id: &str, request: Value) -> Result<()> {
        let Some(conv) = self.live(session_id) else { return Ok(()) };
        let req_id = lock(&conv.rt).request_id();
        write_line(&conv, &json!({"type": "control_request", "request_id": req_id, "request": request}))
    }

    /// `conv_stop`: come `stop`, con il controllo del progetto.
    pub fn stop_owned(&self, project_id: &str, session_id: &str) -> Result<()> {
        self.owned_conv(project_id, session_id)?;
        self.stop(session_id);
        Ok(())
    }

    /// Ferma il processo della conversazione: interrompe il turno se ce n'è uno, chiude stdin, aspetta fino a
    /// 2 s e poi uccide il gruppo del processo. Lo stato diventa `waiting` se un turno è stato tagliato,
    /// altrimenti non cambia. Senza processo non fa nulla.
    ///
    /// Sempre limitato nel tempo (`STOP_GRACE + KILL_WAIT`): interruzione e chiusura di stdin passano da un
    /// thread a parte, perché il mutex di stdin può essere tenuto da uno scrittore bloccato su una pipe piena
    /// (processo che non legge più). Ucciso il gruppo, quello scrittore riceve EPIPE e si sblocca.
    pub fn stop(&self, session_id: &str) {
        let Some(conv) = self.live(session_id) else { return };
        let working = {
            let mut rt = lock(&conv.rt);
            rt.stopping = true;
            rt.state.status == ConvStatus::Working
        };
        {
            let conv = conv.clone();
            std::thread::spawn(move || {
                if working {
                    let req_id = lock(&conv.rt).request_id();
                    let _ = write_line(&conv, &json!({"type": "control_request", "request_id": req_id, "request": {"subtype": "interrupt"}}));
                }
                *lock(&conv.stdin) = None;
                lock(&conv.rt).closed = true;
            });
        }
        if !wait_finished(&conv, STOP_GRACE) {
            {
                let rt = lock(&conv.rt);
                if !rt.reaped {
                    super::locate::kill_group(conv.pid);
                }
            }
            wait_finished(&conv, KILL_WAIT);
        }
    }

    /// Ferma tutti i processi in parallelo (all'uscita dell'app): limitato come un singolo `stop`.
    pub fn stop_all(&self) {
        let ids: Vec<String> = lock(&self.0.convs).keys().cloned().collect();
        std::thread::scope(|s| {
            for id in &ids {
                s.spawn(move || self.stop(id));
            }
        });
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
        self.owned_conv(project_id, session_id)?;
        let deadline = Instant::now() + REPLACE_WAIT;
        let mut convs = loop {
            let convs = lock(&self.0.convs);
            let Some(c) = convs.get(session_id).cloned() else { break convs };
            if !c.is_dead() {
                return Ok(c);
            }
            // processo in chiusura (fermato, uscito o con stdin rotto): prima di avviarne un altro sulla stessa
            // sessione si aspetta che sia del tutto terminato (la sua pulizia lo toglie dalla mappa)
            drop(convs);
            let left = deadline.saturating_duration_since(Instant::now());
            if !wait_finished(&c, left) {
                return Err("la conversazione si sta ancora chiudendo, riprova tra poco".into());
            }
            let mut convs = lock(&self.0.convs);
            if convs.get(session_id).is_some_and(|x| Arc::ptr_eq(x, &c)) {
                convs.remove(session_id); // terminato ma non più "corrente" per la sua pulizia: non serve oltre
            }
        };
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
        let mut cmd = Command::new(&claude);
        // sessione propria: `stop` può uccidere anche i discendenti (server MCP, strumenti)
        super::locate::in_new_session(&mut cmd);
        let mut child = cmd
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
            pid: child.id(),
            stdin: Mutex::new(child.stdin.take()),
            rt: Mutex::new(Runtime {
                state: self.load_state(session_id),
                pending: HashMap::new(),
                ask_seq: 0,
                next_req: 0,
                closed: false,
                stopping: false,
                reaped: false,
                finished: false,
            }),
            ended: Condvar::new(),
        });
        convs.insert(session_id.to_string(), conv.clone());
        drop(convs);

        // tre thread: stderr in un buffer circolare, stdout riga per riga, attesa dell'uscita del processo.
        // Nessuno aspetta la chiusura di una pipe per gestire l'uscita (un discendente può tenerla aperta).
        let tail = Arc::new(Mutex::new(Vec::new()));
        {
            let tail = tail.clone();
            std::thread::spawn(move || collect_tail(stderr, &tail));
        }
        let (done_tx, done_rx) = mpsc::channel::<()>();
        let (engine, c, sid) = (self.clone(), conv.clone(), session_id.to_string());
        std::thread::spawn(move || {
            engine.read_stdout(&sid, &c, stdout);
            let _ = done_tx.send(());
        });
        let (engine, c, sid) = (self.clone(), conv.clone(), session_id.to_string());
        std::thread::spawn(move || engine.wait_exit(&sid, &c, child, &done_rx, &tail));

        // handshake facoltativo (§2.1): comandi slash e modelli arrivano subito come `ControlResponse`
        let req_id = lock(&conv.rt).request_id();
        let _ = write_line(&conv, &json!({"type": "control_request", "request_id": req_id, "request": {"subtype": "initialize", "hooks": null}}));
        self.emit_upsert(session_id);
        Ok(conv)
    }

    fn read_stdout(&self, session_id: &str, conv: &Arc<Conv>, stdout: Option<std::process::ChildStdout>) {
        let Some(out) = stdout else { return };
        let mut reader = BufReader::new(out);
        let mut buf = Vec::new();
        loop {
            buf.clear();
            match reader.read_until(b'\n', &mut buf) {
                Ok(0) | Err(_) => break,
                Ok(_) if lock(&conv.rt).finished => break,
                Ok(_) => self.handle_line(session_id, conv, &String::from_utf8_lossy(&buf)),
            }
        }
    }

    /// Aspetta l'uscita del processo (non la chiusura delle sue pipe), poi salva lo stato ed emette gli eventi.
    fn wait_exit(&self, session_id: &str, conv: &Arc<Conv>, mut child: Child, stdout_done: &mpsc::Receiver<()>, tail: &Mutex<Vec<u8>>) {
        // attesa senza raccogliere il processo: finché è zombie il suo pgid non può essere riusato e si può
        // uccidere il gruppo (discendenti rimasti orfani: server MCP, strumenti in background)
        wait_exit_no_reap(conv.pid);
        super::locate::kill_group(conv.pid);
        // raccolta e `reaped` sotto lo stesso lock: `stop` non può uccidere un pid già raccolto
        let success = {
            let mut rt = lock(&conv.rt);
            let ok = child.wait().is_ok_and(|s| s.success()); // immediato: il processo è già zombie
            rt.reaped = true;
            rt.closed = true;
            ok
        };
        // chiusura di stdin senza aspettare uno scrittore bloccato (riceverà EPIPE)
        if let Ok(mut stdin) = conv.stdin.try_lock() {
            *stdin = None;
        }
        // le ultime righe già scritte arrivano ancora; poi si va avanti anche se la pipe resta aperta
        let _ = stdout_done.recv_timeout(STDOUT_GRACE);
        // stato salvato sotto il lock della mappa: una ripresa concorrente lo rilegge aggiornato, e una
        // conversazione già sostituita da un nuovo processo non viene toccata
        let current = {
            let mut convs = lock(&self.0.convs);
            let current = convs.get(session_id).is_some_and(|c| Arc::ptr_eq(c, conv));
            let mut rt = lock(&conv.rt);
            rt.finished = true;
            rt.pending.clear();
            if current {
                rt.state = next_status(rt.state, StateInput::ProcessExited);
                self.save(session_id, &rt.state, false);
                convs.remove(session_id);
            }
            conv.ended.notify_all();
            current
        };
        lock(&self.0.exits).insert(session_id.to_string(), Instant::now());
        if !success {
            let mut json = String::from_utf8_lossy(&lock(tail)).into_owned();
            truncate_utf8(&mut json, UNKNOWN_JSON_CAP);
            self.emit_event(&conv.project_id, session_id, &ClaudeEvent::Unknown { label: EXIT_LABEL.into(), json });
        }
        if current {
            self.emit_upsert(session_id);
        }
    }

    fn handle_line(&self, session_id: &str, conv: &Arc<Conv>, line: &str) {
        for mut ev in parse_line(line) {
            if let ClaudeEvent::Unknown { json, .. } = &mut ev {
                truncate_utf8(json, UNKNOWN_JSON_CAP);
            }
            let mut changed = false;
            let forward = {
                let mut rt = lock(&conv.rt);
                // dentro la sezione critica: una riga tardiva non sovrascrive lo stato salvato all'uscita
                if rt.finished {
                    return;
                }
                changed |= self.persist_metadata(session_id, &ev);
                let forward = match &ev {
                    ClaudeEvent::Ask { request_id, .. } => {
                        rt.ask_seq += 1;
                        let seq = rt.ask_seq;
                        rt.pending.insert(request_id.clone(), (seq, ev.clone()));
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
    fn load_image(&self, project: &Project, path: &str) -> Result<(Value, u64)> {
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
        Ok((json!({"type": "image", "source": {"type": "base64", "media_type": media_type, "data": data}}), meta.len()))
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
    // (il mutex resta tenuto per tutta la scrittura: le righe non si mescolano; `stop` non ne dipende)
    pipe.write_all(line.as_bytes()).and_then(|_| pipe.flush()).map_err(|e| format!("scrittura verso claude fallita: {e}"))?;
    Ok(())
}

/// Aspetta che l'uscita del processo sia stata gestita, al massimo `timeout`.
fn wait_finished(conv: &Conv, timeout: Duration) -> bool {
    let rt = lock(&conv.rt);
    let (rt, _) = conv.ended.wait_timeout_while(rt, timeout, |r| !r.finished).unwrap_or_else(|p| p.into_inner());
    rt.finished
}

/// Blocca finché il processo `pid` (nostro figlio) esce, lasciandolo zombie (`WNOWAIT`).
fn wait_exit_no_reap(pid: u32) {
    let id: libc::id_t = pid;
    loop {
        // SAFETY: `info` è una struttura valida in cui il kernel scrive; `WNOWAIT` non raccoglie il figlio.
        let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
        let r = unsafe { libc::waitid(libc::P_PID, id, &mut info, libc::WEXITED | libc::WNOWAIT) };
        if r == 0 || std::io::Error::last_os_error().raw_os_error() != Some(libc::EINTR) {
            return;
        }
    }
}

/// Tiene gli ultimi `STDERR_TAIL` byte di stderr in un buffer condiviso (letto all'uscita senza aspettare la
/// chiusura della pipe).
fn collect_tail(stderr: Option<std::process::ChildStderr>, tail: &Mutex<Vec<u8>>) {
    let Some(mut err) = stderr else { return };
    let mut buf = [0u8; 8192];
    while let Ok(n) = err.read(&mut buf) {
        if n == 0 {
            break;
        }
        let mut t = lock(tail);
        t.extend_from_slice(&buf[..n]);
        if t.len() > STDERR_TAIL {
            let excess = t.len() - STDERR_TAIL;
            t.drain(..excess);
        }
    }
}

// ---------- integrazione con Tauri ----------

use tauri::{AppHandle, Emitter};

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
    fn startup_downgrades_working_to_waiting() {
        let env = setup(&[]);
        let sid = "11111111-1111-4111-8111-111111111111";
        env.store.insert_owned_conversation("p1", sid, "/Users/dev/x", None, "default").unwrap();
        env.store.save_conversation_state(sid, "working", 1, 0, false).unwrap();
        // riavvio dell'app: nuovo motore sullo stesso archivio
        let engine = Engine::new(env.engine.0.launch.clone(), env.store.clone(), env.sink.clone(), env.sessions_root.clone(), env.images.clone());
        assert_eq!(status(&env, sid), "waiting");
        assert!(!engine.is_running(sid), "nessun turno riparte da solo");
        let marker = engine.pending_asks("p1", sid).unwrap();
        assert!(matches!(&marker[..], [ClaudeEvent::Unknown { label, .. }] if label == INTERRUPTED_LABEL), "{marker:?}");
        assert_eq!(env.store.waiting_total().unwrap(), 1);
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
    fn pending_asks_lists_open_requests_until_answered() {
        let fx = fixture("permission-request");
        let env = setup(&[("FAKE_CLAUDE_FIXTURE", &fx)]);
        let sid = env.engine.start("p1", StartOpts::default()).unwrap();
        assert!(env.engine.pending_asks("p1", &sid).unwrap().is_empty());
        env.engine.send("p1", &sid, "scrivi", &[]).unwrap();
        wait_until("richiesta", || env.sink.has_kind(&sid, "ask"));
        let asks = env.engine.pending_asks("p1", &sid).unwrap();
        assert_eq!(asks.len(), 1);
        assert!(matches!(&asks[0], ClaudeEvent::Ask { request_id, .. } if request_id == "c6b1b64f-8800-4372-b30a-5b4c4e3c465b"));
        assert!(env.engine.pending_asks("p2", &sid).is_err()); // altro progetto
        env.engine.answer("p1", &sid, "c6b1b64f-8800-4372-b30a-5b4c4e3c465b", "allow").unwrap();
        assert!(env.engine.pending_asks("p1", &sid).unwrap().is_empty());
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

    fn spawns(env: &Env, sid: &str) -> usize {
        read(&env.out.join(format!("{sid}.spawns"))).lines().count()
    }

    fn alive(pid: i32) -> bool {
        // SAFETY: segnale 0 = solo verifica di esistenza
        unsafe { libc::kill(pid, 0) == 0 }
    }

    #[test]
    fn stray_child_holding_stderr_does_not_block_exit_and_send_resumes() {
        let fx = fixture("simple-turn");
        let env = setup(&[("FAKE_CLAUDE_FIXTURE", &fx), ("FAKE_CLAUDE_CRASH_AFTER", "4"), ("FAKE_CLAUDE_STRAY", "stderr")]);
        let sid = env.engine.start("p1", StartOpts::default()).unwrap();
        env.engine.send("p1", &sid, "ciao", &[]).unwrap();
        let t = Instant::now();
        wait_until("uscita gestita", || !env.engine.is_running(&sid) && status(&env, &sid) == "waiting");
        assert!(t.elapsed() < Duration::from_secs(3), "{:?}", t.elapsed());
        wait_until("evento di uscita", || env.sink.events(&sid).iter().any(|e| e["label"] == "Processo terminato"
            && e["json"].as_str().unwrap_or("").contains("errore finto")));
        assert!(env.engine.is_ours(&sid)); // appena uscito: le scritture recenti sono nostre
        env.engine.send("p1", &sid, "di nuovo", &[]).unwrap();
        wait_until("secondo avvio", || spawns(&env, &sid) == 2);
        // il nuovo processo riceve il messaggio (poi, con lo stesso finto, esce di nuovo)
        wait_until("messaggio ripreso", || stdin_lines(&env, &sid).iter().any(|l| l["message"]["content"] == "di nuovo"));
    }

    #[test]
    fn send_right_after_exit_resumes_instead_of_erroring() {
        // il primo processo esce subito lasciando lo stdout aperto a un discendente: per un attimo resta nella mappa
        let env = setup(&[("FAKE_CLAUDE_EXIT_AT_START", "1"), ("FAKE_CLAUDE_STRAY", "stdout")]);
        let sid = env.engine.start("p1", StartOpts::default()).unwrap();
        wait_until("primo processo uscito", || env.out.join(format!("{sid}.exited")).exists());
        std::thread::sleep(Duration::from_millis(50));
        env.engine.send("p1", &sid, "ciao", &[]).unwrap();
        wait_until("secondo avvio", || spawns(&env, &sid) == 2);
        wait_until("messaggio ricevuto", || stdin_lines(&env, &sid).iter().any(|l| l["type"] == "user"));
        assert!(env.engine.is_running(&sid));
    }

    #[test]
    fn concurrent_sends_on_a_stopped_session_spawn_once() {
        let env = setup(&[]);
        let sid = "55555555-5555-4555-8555-555555555555";
        env.store.upsert_synced_conversation("p1", sid, &env.folders[0].to_string_lossy(), "t", 0, 0).unwrap();
        std::thread::scope(|s| {
            for text in ["uno", "due"] {
                let engine = env.engine.clone();
                s.spawn(move || engine.send("p1", sid, text, &[]).unwrap());
            }
        });
        wait_until("due messaggi", || stdin_lines(&env, sid).iter().filter(|l| l["type"] == "user").count() == 2);
        std::thread::sleep(Duration::from_millis(200)); // un eventuale secondo avvio avrebbe già scritto
        assert_eq!(spawns(&env, sid), 1);
    }

    #[test]
    fn stop_closes_an_idle_process_and_keeps_status() {
        let fx = fixture("simple-turn");
        let env = setup(&[("FAKE_CLAUDE_FIXTURE", &fx)]);
        let sid = env.engine.start("p1", StartOpts::default()).unwrap();
        env.engine.send("p1", &sid, "ciao", &[]).unwrap();
        wait_until("fine turno", || status(&env, &sid) == "waiting");
        env.engine.mark_viewed("p1", &sid).unwrap();
        let t = Instant::now();
        env.engine.stop_owned("p1", &sid).unwrap();
        assert!(t.elapsed() < Duration::from_secs(2), "{:?}", t.elapsed()); // stdin chiuso: esce da solo
        assert!(!env.engine.is_running(&sid));
        assert_eq!(status(&env, &sid), "idle");
        assert!(!env.sink.events(&sid).iter().any(|e| e["label"] == "Processo terminato"));
        assert!(env.engine.stop_owned("p2", &sid).is_err());
        env.engine.stop_owned("p1", &sid).unwrap(); // senza processo: nulla
    }

    #[test]
    fn stop_kills_a_stuck_turn_and_its_children() {
        let env = setup(&[("FAKE_CLAUDE_HANG", "1")]);
        let sid = env.engine.start("p1", StartOpts::default()).unwrap();
        env.engine.send("p1", &sid, "lavora", &[]).unwrap();
        let hang = env.out.join(format!("{sid}.hang"));
        wait_until("processo bloccato", || read(&hang).ends_with('\n'));
        let child: i32 = read(&hang).trim().parse().unwrap();
        assert_eq!(status(&env, &sid), "working");
        let t = Instant::now();
        env.engine.stop("66666666-6666-4666-8666-666666666666"); // sessione senza processo: nulla
        env.engine.stop(&sid);
        let took = t.elapsed();
        assert!(took >= Duration::from_millis(1900) && took < Duration::from_secs(4), "{took:?}");
        assert!(!env.engine.is_running(&sid));
        assert_eq!(status(&env, &sid), "waiting"); // turno tagliato
        wait_until("discendente ucciso con il gruppo", || !alive(child));
    }

    #[test]
    fn stop_all_stops_every_process() {
        let env = setup(&[]);
        let a = env.engine.start("p1", StartOpts::default()).unwrap();
        let b = env.engine.start("p2", StartOpts::default()).unwrap();
        env.engine.stop_all();
        assert!(!env.engine.is_running(&a) && !env.engine.is_running(&b));
    }

    #[test]
    fn stop_is_bounded_when_the_child_does_not_read_stdin() {
        let env = setup(&[("FAKE_CLAUDE_NO_STDIN", "1")]);
        let sid = env.engine.start("p1", StartOpts::default()).unwrap();
        let hang = env.out.join(format!("{sid}.hang"));
        wait_until("processo fermo", || read(&hang).ends_with('\n'));
        let child: i32 = read(&hang).trim().parse().unwrap();
        // invio grande (ben oltre il buffer della pipe): lo scrittore resta bloccato tenendo il mutex di stdin
        let img = env.images.join("grande.png");
        std::fs::write(&img, vec![0u8; 4 * 1024 * 1024]).unwrap();
        let sender = {
            let (engine, sid, img) = (env.engine.clone(), sid.clone(), img.to_string_lossy().into_owned());
            std::thread::spawn(move || engine.send("p1", &sid, "guarda", &[img]))
        };
        wait_until("turno avviato", || status(&env, &sid) == "working");
        std::thread::sleep(Duration::from_millis(200));
        assert!(!sender.is_finished(), "la scrittura doveva restare bloccata");
        let t = Instant::now();
        env.engine.stop(&sid);
        let took = t.elapsed();
        assert!(took < Duration::from_secs(5), "{took:?}");
        assert!(!env.engine.is_running(&sid));
        assert_eq!(status(&env, &sid), "waiting"); // turno tagliato
        wait_until("gruppo ucciso", || !alive(child));
        // lo scrittore bloccato riceve EPIPE e non riavvia la sessione fermata
        assert!(sender.join().unwrap().is_err());
        assert_eq!(spawns(&env, &sid), 1);
    }

    #[test]
    fn send_during_stop_waits_for_the_old_process() {
        let env = setup(&[("FAKE_CLAUDE_HANG", "1")]);
        let sid = env.engine.start("p1", StartOpts::default()).unwrap();
        env.engine.send("p1", &sid, "lavora", &[]).unwrap();
        let hang = env.out.join(format!("{sid}.hang"));
        wait_until("processo bloccato", || read(&hang).ends_with('\n'));
        let first_child: i32 = read(&hang).trim().parse().unwrap();
        let stopper = {
            let (engine, sid) = (env.engine.clone(), sid.clone());
            std::thread::spawn(move || engine.stop(&sid))
        };
        std::thread::sleep(Duration::from_millis(300)); // dentro l'attesa di `stop`
        let t = Instant::now();
        env.engine.send("p1", &sid, "dopo", &[]).unwrap();
        // l'invio ha aspettato che il vecchio processo fosse ucciso e ripulito, poi ne ha avviato uno solo
        assert!(t.elapsed() >= Duration::from_millis(1500), "{:?}", t.elapsed());
        stopper.join().unwrap();
        wait_until("discendente del primo processo morto", || !alive(first_child));
        wait_until("secondo avvio", || spawns(&env, &sid) == 2);
        wait_until("messaggio al nuovo processo", || stdin_lines(&env, &sid).iter().any(|l| l["message"]["content"] == "dopo"));
        env.engine.stop(&sid);
    }

    #[test]
    fn archiving_stops_the_process() {
        let env = setup(&[]);
        let sid = env.engine.start("p1", StartOpts::default()).unwrap();
        env.engine.archive("p1", &sid, false).unwrap();
        assert!(env.engine.is_running(&sid));
        env.engine.archive("p1", &sid, true).unwrap();
        assert!(!env.engine.is_running(&sid));
        assert!(env.store.get_conversation(&sid).unwrap().unwrap().archived);
    }

    #[test]
    fn start_without_claude_leaves_no_conversation() {
        let env = setup(&[]);
        let launch: Arc<LaunchCell> = Arc::new(LazyLock::new(Box::new(|| Launch { claude: None, env: HashMap::new() })));
        let engine = Engine::new(launch, env.store.clone(), env.sink.clone(), env.sessions_root.clone(), env.images.clone());
        assert!(engine.start("p1", StartOpts::default()).is_err());
        assert!(env.store.list_conversations("p1").unwrap().is_empty());
        // avvio che fallisce dopo l'inserimento (eseguibile inesistente): riga rimossa
        let launch: Arc<LaunchCell> =
            Arc::new(LazyLock::new(Box::new(|| Launch { claude: Some("/nonexistent/claude".into()), env: HashMap::new() })));
        let engine = Engine::new(launch, env.store.clone(), env.sink.clone(), env.sessions_root.clone(), env.images.clone());
        assert!(engine.start("p1", StartOpts::default()).is_err());
        assert!(env.store.list_conversations("p1").unwrap().is_empty());
    }

    #[test]
    fn images_are_bounded_per_message() {
        let env = setup(&[]);
        let sid = env.engine.start("p1", StartOpts::default()).unwrap();
        let paths: Vec<String> = (0..11)
            .map(|i| {
                let p = env.images.join(format!("{i}.png"));
                std::fs::write(&p, [0x89, b'P', b'N', b'G']).unwrap();
                p.to_string_lossy().into_owned()
            })
            .collect();
        let err = env.engine.send("p1", &sid, "x", &paths).unwrap_err();
        assert!(err.contains("10 immagini"), "{err}");
        let big: Vec<String> = (0..2)
            .map(|i| {
                let p = env.images.join(format!("big{i}.png"));
                std::fs::write(&p, vec![0u8; 16 * 1024 * 1024]).unwrap();
                p.to_string_lossy().into_owned()
            })
            .collect();
        let err = env.engine.send("p1", &sid, "x", &big).unwrap_err();
        assert!(err.contains("30 MB"), "{err}");
        assert!(!stdin_lines(&env, &sid).iter().any(|l| l["type"] == "user"));
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
