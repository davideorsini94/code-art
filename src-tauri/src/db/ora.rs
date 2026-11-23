//! Sessione Oracle. Le chiamate del driver (`oracle`, ODPI-C) sono bloccanti: ogni sessione vive in un thread dedicato
//! (un attore) che possiede la `Connection` e serve richieste da un canale `std::sync::mpsc`; la risposta torna da un
//! canale `tokio::sync::oneshot`, così il lato async non blocca mai il runtime. Il thread è anche il solo a poter
//! tenere un `ResultSet` aperto tra una pagina e l'altra (tappa 4b, compito 3: aggiunge solo varianti a `Request`
//! e metodi a `Backend`).
//!
//! Annullamento: `oracle::Connection` è `Send + Sync` e `break_execution(&self)` si può chiamare da un altro thread
//! mentre il thread della sessione è dentro una query. Per questo l'annullamento NON passa dal canale (la richiesta
//! resterebbe in coda dietro la query): l'attore espone un `Breaker`, un handle clonabile preso dal backend subito
//! dopo la connessione.
//!
//! La stringa di connessione è `//host:port/servizio`, mai con la password: l'utente e la password vanno a
//! `Connector::new(user, password, stringa)`.
use super::mongo::NOT_CONNECTED;
use super::pg::{sanitize, CONNECT_TIMEOUT, LOST, TIMED_OUT, UNREACHABLE};
use super::tunnel::{self, Tunnel};
use super::DbConnection;
use crate::lock;
use crate::ssh::conn::{ConnState, Connections, StateEvent, AUTH};
use crate::ssh::Server;
use std::collections::HashMap;
use std::panic::AssertUnwindSafe;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use tokio::sync::oneshot;

/// Il codice con cui `db_connect` dice «Instant Client non configurato»: l'interfaccia apre la guida.
pub const ORACLE_CLIENT: &str = "oracle_client";
pub const UNKNOWN_SERVICE: &str = "Servizio sconosciuto sul server";
pub const ACCOUNT_LOCKED: &str = "Account bloccato";
pub const PASSWORD_EXPIRED: &str = "Password scaduta";
/// Ogni quanto, a riposo, il thread controlla che la connessione sia viva.
pub const PING_EVERY: Duration = Duration::from_secs(30);
/// Dopo quanti `ping` falliti di fila la sessione risulta caduta.
pub const PING_FAILURES: u32 = 2;
/// Attesa massima della chiusura del thread (poi lo si abbandona: finirà da solo).
pub const JOIN_TIMEOUT: Duration = Duration::from_secs(3);

/// Impostazioni di sessione subito dopo la connessione: i valori mostrati come testo si riscrivono identici.
pub const NLS_SETTINGS: [(&str, &str); 4] = [
    ("NLS_DATE_FORMAT", "YYYY-MM-DD HH24:MI:SS"),
    ("NLS_TIMESTAMP_FORMAT", "YYYY-MM-DD HH24:MI:SS.FF"),
    ("NLS_TIMESTAMP_TZ_FORMAT", "YYYY-MM-DD HH24:MI:SS.FF TZH:TZM"),
    ("NLS_NUMERIC_CHARACTERS", ".,"),
];

pub fn nls_statement() -> String {
    let parts: Vec<String> = NLS_SETTINGS.iter().map(|(k, v)| format!("{k}='{v}'")).collect();
    format!("ALTER SESSION SET {}", parts.join(" "))
}

// ---------------------------------------------------------------------------------------------------------------
// Parametri e stringa di connessione
// ---------------------------------------------------------------------------------------------------------------

/// Dati per aprire una connessione (senza password e senza tunnel: quelli si passano a parte).
/// `service` è il campo `database` della connessione salvata.
#[derive(Debug, Clone, PartialEq)]
pub struct OraParams {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub service: String,
}

impl OraParams {
    pub fn from_connection(c: &DbConnection) -> OraParams {
        OraParams {
            host: c.host.clone(),
            port: c.port,
            user: c.user.clone().unwrap_or_default(),
            service: c.database.clone().unwrap_or_default(),
        }
    }

    /// `//host:porta/servizio`; con un tunnel SSH ci si collega a 127.0.0.1 sulla porta locale. Mai la password.
    pub fn connect_string(&self, local_port: Option<u16>) -> String {
        let (host, port) = match local_port {
            Some(p) => ("127.0.0.1", p),
            None => (self.host.as_str(), self.port),
        };
        if host.contains(':') {
            format!("//[{}]:{}/{}", host.trim_matches(|c| c == '[' || c == ']'), port, self.service)
        } else {
            format!("//{host}:{port}/{}", self.service)
        }
    }
}

/// Nome del servizio: obbligatorio, lettere, cifre e `._-$#`.
pub fn validate_service(v: &str) -> Result<(), String> {
    let v = v.trim();
    if v.is_empty() {
        return Err("Servizio: campo obbligatorio per Oracle".into());
    }
    if v.chars().count() > 128 {
        return Err("Servizio: al massimo 128 caratteri".into());
    }
    if !v.chars().all(|c| c.is_alphanumeric() || "._-$#".contains(c)) {
        return Err("Servizio: solo lettere, cifre e . _ - $ #".into());
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------------------------
// Errori
// ---------------------------------------------------------------------------------------------------------------

/// Un errore del driver ridotto a quel che serve: codice ORA (errore del server/OCI), codice DPI, testo.
#[derive(Debug, Clone, PartialEq)]
pub struct OraErr {
    pub ora: Option<i32>,
    pub dpi: Option<i32>,
    pub message: String,
}

impl OraErr {
    pub fn new(message: impl Into<String>) -> OraErr {
        OraErr { ora: None, dpi: None, message: message.into() }
    }
    pub fn is_connection_lost(&self) -> bool {
        is_connection_lost(self.ora, self.dpi)
    }
}

impl From<&oracle::Error> for OraErr {
    fn from(e: &oracle::Error) -> OraErr {
        let message = e.db_error().map(|d| d.message().to_string()).unwrap_or_else(|| e.to_string());
        OraErr { ora: e.oci_code(), dpi: e.dpi_code(), message }
    }
}

/// ORA-03113/03114/03135 (fine file sul canale, non connesso, connessione persa) e DPI-1080 (connessione chiusa).
pub fn is_connection_lost(ora: Option<i32>, dpi: Option<i32>) -> bool {
    matches!(ora, Some(3113 | 3114 | 3135)) || dpi == Some(1080)
}

/// Toglie password e descrittori di connessione dal testo di un errore.
fn clean(text: &str, password: Option<&str>) -> String {
    let base = match text.find("(DESCRIPTION") {
        Some(i) => format!("{}(…)", &text[..i]),
        None => text.to_string(),
    };
    sanitize(base.trim(), password)
}

/// Messaggio mostrato (o `auth`) per un errore. Il testo originale entra solo ripulito.
pub fn map_error(e: &OraErr, password: Option<&str>) -> String {
    match e.ora {
        Some(1017) => AUTH.into(),
        Some(12541 | 12545 | 12170 | 12543) => UNREACHABLE.into(),
        Some(12514) => UNKNOWN_SERVICE.into(),
        Some(28000) => ACCOUNT_LOCKED.into(),
        Some(28001) => PASSWORD_EXPIRED.into(),
        _ if e.is_connection_lost() => LOST.into(),
        _ => clean(&e.message, password),
    }
}

// ---------------------------------------------------------------------------------------------------------------
// L'attore: un thread per sessione
// ---------------------------------------------------------------------------------------------------------------

/// Risposta di una richiesta: arriva dal thread della sessione.
pub type Reply<T> = oneshot::Sender<Result<T, String>>;

/// Richieste al thread della sessione. Il compito 3 aggiunge qui query, pagine, chiusura dei risultati e modifiche.
pub enum Request {
    /// Controllo di vita (`Connection::ping`).
    Ping(Reply<()>),
    /// Chiude la sessione: il thread esce.
    Close,
}

/// Annulla la query in corso (`break_execution`): callable da qualunque thread.
pub type Breaker = Arc<dyn Fn() -> Result<(), String> + Send + Sync>;

/// Ciò che il thread sa fare con la connessione. Il compito 3 aggiunge i metodi di query; nei test c'è un finto.
pub trait Backend: Send + 'static {
    fn ping(&mut self) -> Result<(), OraErr>;
    /// Handle di annullamento, preso una volta dopo la connessione.
    fn breaker(&self) -> Breaker;
}

/// Tempi dell'attore (iniettabili nei test).
#[derive(Debug, Clone, Copy)]
pub struct Timing {
    pub ping_every: Duration,
}

impl Default for Timing {
    fn default() -> Timing {
        Timing { ping_every: PING_EVERY }
    }
}

enum Flow {
    Continue,
    Stop,
    Lost(String),
}

/// Smista una richiesta. Un errore di connessione caduta (`is_connection_lost`) in una qualunque richiesta
/// restituisce `Lost`: il thread esce e la sessione diventa `disconnected`.
fn serve(backend: &mut dyn Backend, req: Request) -> Flow {
    match req {
        Request::Ping(reply) => {
            let r = backend.ping();
            let flow = match &r {
                Err(e) if e.is_connection_lost() => Flow::Lost(e.message.clone()),
                _ => Flow::Continue,
            };
            let _ = reply.send(r.map_err(|e| map_error(&e, None)));
            flow
        }
        Request::Close => Flow::Stop,
    }
}

/// Il ciclo del thread: richieste in ordine; a riposo un `ping` ogni `ping_every`, `PING_FAILURES` falliti di fila
/// (o uno solo con connessione persa) = caduta. Restituisce il motivo se la connessione è caduta.
fn run_loop(backend: &mut dyn Backend, rx: &mpsc::Receiver<Request>, timing: Timing) -> Option<String> {
    let mut failures = 0;
    loop {
        match rx.recv_timeout(timing.ping_every) {
            Ok(req) => match serve(backend, req) {
                Flow::Continue => failures = 0,
                Flow::Stop => return None,
                Flow::Lost(m) => return Some(m),
            },
            Err(RecvTimeoutError::Timeout) => match backend.ping() {
                Ok(()) => failures = 0,
                Err(e) if e.is_connection_lost() => return Some(e.message),
                Err(e) => {
                    failures += 1;
                    if failures >= PING_FAILURES {
                        return Some(e.message);
                    }
                }
            },
            // tutti i mittenti sono stati lasciati: chiusura
            Err(RecvTimeoutError::Disconnected) => return None,
        }
    }
}

/// Un attore avviato, in attesa del risultato della connessione.
pub struct Pending {
    tx: mpsc::Sender<Request>,
    join: JoinHandle<()>,
    ready: oneshot::Receiver<Result<Breaker, String>>,
}

impl Pending {
    /// Attende la connessione al più `timeout`. Scaduto: il thread viene abbandonato (chiude da solo la
    /// connessione quando finisce, vedendo il canale chiuso).
    pub async fn ready(self, timeout: Duration) -> Result<Arc<Actor>, String> {
        let Pending { tx, join, ready } = self;
        match tokio::time::timeout(timeout, ready).await {
            Ok(Ok(Ok(breaker))) => Ok(Actor::new(tx, join, breaker)),
            Ok(Ok(Err(e))) => Err(e),
            Ok(Err(_)) => Err("Connessione annullata".into()),
            Err(_) => Err(TIMED_OUT.into()),
        }
    }

    /// Come `ready`, bloccante (test).
    pub fn ready_blocking(self) -> Result<Arc<Actor>, String> {
        let Pending { tx, join, ready } = self;
        match ready.blocking_recv() {
            Ok(Ok(breaker)) => Ok(Actor::new(tx, join, breaker)),
            Ok(Err(e)) => Err(e),
            Err(_) => Err("Connessione annullata".into()),
        }
    }
}

/// Avvia il thread della sessione: `connect` gira nel thread (il driver blocca), poi il ciclo delle richieste.
/// `on_end(motivo)` si chiama solo se la connessione è caduta (non per una chiusura voluta).
pub fn spawn<B: Backend>(
    connect: impl FnOnce() -> Result<B, String> + Send + 'static,
    timing: Timing,
    on_end: impl FnOnce(String) + Send + 'static,
) -> Pending {
    let (tx, rx) = mpsc::channel::<Request>();
    let (ready_tx, ready_rx) = oneshot::channel();
    let join = std::thread::Builder::new()
        .name("oracle-session".into())
        .spawn(move || {
            let outcome = std::panic::catch_unwind(AssertUnwindSafe(|| {
                let mut backend = match connect() {
                    Ok(b) => b,
                    Err(e) => {
                        let _ = ready_tx.send(Err(e));
                        return None;
                    }
                };
                if ready_tx.send(Ok(backend.breaker())).is_err() {
                    // nessuno aspetta più (tempo scaduto): la connessione si chiude qui
                    return None;
                }
                run_loop(&mut backend, &rx, timing)
            }));
            match outcome {
                Ok(Some(reason)) => on_end(reason),
                Ok(None) => {}
                Err(_) => on_end("errore interno nel thread della sessione".into()),
            }
        })
        .expect("thread della sessione Oracle");
    Pending { tx, join, ready: ready_rx }
}

/// Lato chiamante di un attore connesso.
pub struct Actor {
    tx: mpsc::Sender<Request>,
    breaker: Breaker,
    join: Mutex<Option<JoinHandle<()>>>,
}

impl Actor {
    fn new(tx: mpsc::Sender<Request>, join: JoinHandle<()>, breaker: Breaker) -> Arc<Actor> {
        Arc::new(Actor { tx, breaker, join: Mutex::new(Some(join)) })
    }

    /// Invia una richiesta e ne attende la risposta senza bloccare il runtime.
    pub async fn request<T>(&self, build: impl FnOnce(Reply<T>) -> Request) -> Result<T, String> {
        let (reply, rx) = oneshot::channel();
        self.tx.send(build(reply)).map_err(|_| LOST.to_string())?;
        rx.await.map_err(|_| LOST.to_string())?
    }

    /// Come `request`, bloccante (test e codice già fuori dal runtime).
    pub fn request_blocking<T>(&self, build: impl FnOnce(Reply<T>) -> Request) -> Result<T, String> {
        let (reply, rx) = oneshot::channel();
        self.tx.send(build(reply)).map_err(|_| LOST.to_string())?;
        rx.blocking_recv().map_err(|_| LOST.to_string())?
    }

    /// Annulla la query in corso (se c'è): non passa dal canale.
    pub fn break_execution(&self) -> Result<(), String> {
        (self.breaker)()
    }

    /// Chiusura voluta: interrompe l'eventuale query, chiede la chiusura e attende il thread (al più `JOIN_TIMEOUT`).
    /// Da un thread che può bloccarsi.
    pub fn shutdown(&self) {
        let join = lock(&self.join).take();
        let Some(join) = join else { return };
        let _ = (self.breaker)();
        let _ = self.tx.send(Request::Close);
        let start = Instant::now();
        while !join.is_finished() && start.elapsed() < JOIN_TIMEOUT {
            std::thread::sleep(Duration::from_millis(5));
        }
        if join.is_finished() {
            let _ = join.join();
        }
    }
}

impl Drop for Actor {
    fn drop(&mut self) {
        self.shutdown();
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Backend vero: `oracle::Connection`
// ---------------------------------------------------------------------------------------------------------------

pub struct OracleBackend {
    conn: Arc<oracle::Connection>,
}

impl OracleBackend {
    /// Connessione e impostazioni di sessione (NLS, autocommit attivo). La password va solo al `Connector`.
    pub fn open(connect_string: &str, user: &str, password: &str) -> Result<OracleBackend, OraErr> {
        let mut conn = oracle::Connector::new(user, password, connect_string).connect().map_err(|e| OraErr::from(&e))?;
        conn.set_autocommit(true);
        conn.execute(&nls_statement(), &[]).map_err(|e| OraErr::from(&e))?;
        Ok(OracleBackend { conn: Arc::new(conn) })
    }

    pub fn connection(&self) -> &Arc<oracle::Connection> {
        &self.conn
    }
}

impl Backend for OracleBackend {
    fn ping(&mut self) -> Result<(), OraErr> {
        self.conn.ping().map_err(|e| OraErr::from(&e))
    }
    fn breaker(&self) -> Breaker {
        let conn = self.conn.clone();
        Arc::new(move || conn.break_execution().map_err(|e| e.to_string()))
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Le sessioni
// ---------------------------------------------------------------------------------------------------------------

pub type Emitter = Arc<dyn Fn(StateEvent) + Send + Sync>;

struct Session {
    state: ConnState,
    message: Option<String>,
    generation: u64,
    actor: Option<Arc<Actor>>,
    tunnel: Option<Tunnel>,
}

/// Le sessioni Oracle aperte, una per connessione salvata, valide per tutte le finestre.
pub struct OraSessions {
    conns: Arc<Connections>,
    map: Mutex<HashMap<String, Session>>,
    next: AtomicU64,
    emit: Emitter,
}

impl OraSessions {
    pub fn new(conns: Arc<Connections>, emit: Emitter) -> Self {
        OraSessions { conns, map: Mutex::new(HashMap::new()), next: AtomicU64::new(1), emit }
    }

    pub fn states(&self) -> Vec<StateEvent> {
        lock(&self.map).iter().map(|(id, s)| StateEvent { id: id.clone(), state: s.state, message: s.message.clone() }).collect()
    }

    /// L'attore della connessione, se è `connected` (per le query).
    pub fn actor(&self, id: &str) -> Result<Arc<Actor>, String> {
        match lock(&self.map).get(id) {
            Some(Session { state: ConnState::Connected, actor: Some(a), .. }) => Ok(a.clone()),
            _ => Err(NOT_CONNECTED.into()),
        }
    }

    /// Annulla la query in corso sulla connessione (`break_execution`, da un altro thread).
    pub fn cancel(&self, id: &str) -> Result<(), String> {
        self.actor(id)?.break_execution()
    }

    fn emit(&self, id: &str, state: ConnState, message: Option<String>) {
        (self.emit)(StateEvent { id: id.to_string(), state, message });
    }

    fn set_if(&self, id: &str, generation: u64, state: ConnState, message: Option<String>) -> bool {
        {
            let mut map = lock(&self.map);
            match map.get_mut(id) {
                Some(s) if s.generation == generation => {
                    s.state = state;
                    s.message = message.clone();
                }
                _ => return false,
            }
        }
        self.emit(id, state, message);
        true
    }

    /// Connette. Già connessa o in connessione: `Ok` subito. `Err("auth")` = serve la password;
    /// `Err("oracle_client")` = Instant Client non configurato (`client_dir` assente).
    pub async fn connect(
        self: &Arc<Self>,
        id: &str,
        p: OraParams,
        password: Option<String>,
        ssh_server: Option<Server>,
        client_dir: Option<PathBuf>,
    ) -> Result<(), String> {
        let generation = {
            let mut map = lock(&self.map);
            if matches!(map.get(id).map(|s| s.state), Some(ConnState::Connecting | ConnState::Connected)) {
                return Ok(());
            }
            let generation = self.next.fetch_add(1, Ordering::Relaxed);
            map.insert(id.to_string(), Session { state: ConnState::Connecting, message: None, generation, actor: None, tunnel: None });
            generation
        };
        self.emit(id, ConnState::Connecting, None);
        match self.establish(id, generation, &p, password, ssh_server, client_dir).await {
            Ok(()) => Ok(()),
            Err(e) => {
                let shown = match e.as_str() {
                    AUTH => "Autenticazione non riuscita".to_string(),
                    ORACLE_CLIENT => "Instant Client non configurato".to_string(),
                    _ => e.clone(),
                };
                if self.set_if(id, generation, ConnState::Error, Some(shown)) {
                    let t = lock(&self.map).get_mut(id).and_then(|s| s.tunnel.take());
                    if let Some(t) = t {
                        let conns = self.conns.clone();
                        let _ = tauri::async_runtime::spawn_blocking(move || tunnel::close(&conns, &t)).await;
                    }
                }
                Err(e)
            }
        }
    }

    async fn establish(
        self: &Arc<Self>,
        id: &str,
        generation: u64,
        p: &OraParams,
        password: Option<String>,
        ssh_server: Option<Server>,
        client_dir: Option<PathBuf>,
    ) -> Result<(), String> {
        // Instant Client prima di tutto (e prima del tunnel): senza, nient'altro ha senso
        let Some(dir) = client_dir else { return Err(ORACLE_CLIENT.into()) };
        tauri::async_runtime::spawn_blocking(move || super::oracle_client::ensure_initialized(&dir)).await.map_err(|e| e.to_string())??;
        let mut local_port = None;
        if let Some(server) = ssh_server {
            let conns = self.conns.clone();
            let (host, port) = (p.host.clone(), p.port);
            let t = tauri::async_runtime::spawn_blocking(move || tunnel::open(&conns, &server, &host, port)).await.map_err(|e| e.to_string())??;
            local_port = Some(t.port);
            let stored = {
                let mut map = lock(&self.map);
                match map.get_mut(id) {
                    Some(s) if s.generation == generation => {
                        s.tunnel = Some(t.clone());
                        true
                    }
                    _ => false,
                }
            };
            if !stored {
                let conns = self.conns.clone();
                let _ = tauri::async_runtime::spawn_blocking(move || tunnel::close(&conns, &t)).await;
                return Err("Connessione annullata".into());
            }
        }
        let connect_string = p.connect_string(local_port);
        let user = p.user.clone();
        let pw = password.clone().unwrap_or_default();
        let secret = password;
        let me = self.clone();
        let sid = id.to_string();
        let pending = spawn(
            move || OracleBackend::open(&connect_string, &user, &pw).map_err(|e| map_error(&e, secret.as_deref())),
            Timing::default(),
            move |reason| me.ended(&sid, generation, &reason),
        );
        let actor = pending.ready(CONNECT_TIMEOUT).await?;
        let leftover = {
            let mut map = lock(&self.map);
            match map.get_mut(id) {
                Some(s) if s.generation == generation => {
                    s.state = ConnState::Connected;
                    s.message = None;
                    s.actor = Some(actor);
                    None
                }
                _ => Some(actor),
            }
        };
        if let Some(actor) = leftover {
            // disconnessa nel frattempo; la chiusura attende il thread: fuori dal runtime
            let _ = tauri::async_runtime::spawn_blocking(move || actor.shutdown()).await;
            return Err("Connessione annullata".into());
        }
        self.emit(id, ConnState::Connected, None);
        Ok(())
    }

    /// La connessione è caduta (server, rete, tunnel). Se non è stata l'utente → `disconnected`.
    fn ended(self: &Arc<Self>, id: &str, generation: u64, reason: &str) {
        let (tunnel, message) = {
            let mut map = lock(&self.map);
            match map.get_mut(id) {
                Some(s) if s.generation == generation && s.state == ConnState::Connected => {
                    s.state = ConnState::Disconnected;
                    let message = format!("{LOST}: {}", clean(reason, None));
                    s.message = Some(message.clone());
                    // l'attore è già uscito da solo: niente attesa
                    s.actor = None;
                    (s.tunnel.take(), message)
                }
                _ => return,
            }
        };
        self.emit(id, ConnState::Disconnected, Some(message));
        if let Some(t) = tunnel {
            let conns = self.conns.clone();
            tauri::async_runtime::spawn_blocking(move || tunnel::close(&conns, &t));
        }
    }

    fn teardown(&self, old: Option<Session>) {
        if let Some(mut s) = old {
            if let Some(a) = s.actor.take() {
                a.shutdown();
            }
            if let Some(t) = s.tunnel.take() {
                tunnel::close(&self.conns, &t);
            }
        }
    }

    /// Chiusura voluta: ferma la query, chiude il thread e toglie il tunnel. Da un thread che può bloccarsi.
    pub fn disconnect(&self, id: &str) {
        let old = lock(&self.map).remove(id);
        if old.is_none() {
            return;
        }
        self.teardown(old);
        lock(&self.map).insert(
            id.to_string(),
            Session { state: ConnState::Disconnected, message: None, generation: self.next.fetch_add(1, Ordering::Relaxed), actor: None, tunnel: None },
        );
        self.emit(id, ConnState::Disconnected, None);
    }

    /// Uscita dall'app: chiude le sessioni e toglie gli inoltri (prima di `Connections::close_all`).
    pub fn close_all(&self) {
        let all: Vec<Session> = lock(&self.map).drain().map(|(_, s)| s).collect();
        for s in all {
            self.teardown(Some(s));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize};

    fn params() -> OraParams {
        OraParams { host: "db.test".into(), port: 1521, user: "scott".into(), service: "ORCLPDB1".into() }
    }

    #[test]
    fn connect_string_from_fields() {
        assert_eq!(params().connect_string(None), "//db.test:1521/ORCLPDB1");
        assert_eq!(params().connect_string(Some(50123)), "//127.0.0.1:50123/ORCLPDB1");
        let v6 = OraParams { host: "fe80::1".into(), ..params() };
        assert_eq!(v6.connect_string(None), "//[fe80::1]:1521/ORCLPDB1");
    }

    #[test]
    fn params_debug_has_no_password_and_service_is_checked() {
        let c = DbConnection {
            id: "i".into(),
            kind: super::super::DbKind::Oracle,
            name: "n".into(),
            host: "h".into(),
            port: 1521,
            user: Some("scott".into()),
            database: Some("svc".into()),
            tls: super::super::DbTls::Disable,
            accept_invalid_certs: false,
            ssh_server_id: None,
            has_password: true,
        };
        let p = OraParams::from_connection(&c);
        assert_eq!(p.service, "svc");
        let dbg = format!("{p:?}");
        assert!(!dbg.to_lowercase().contains("password"));
        for ok in ["ORCL", "orcl.example.com", "PDB_1", "a-b$c#d"] {
            assert!(validate_service(ok).is_ok(), "{ok}");
        }
        for bad in ["", "  ", "a b", "a/b", "a(b)", "a=b", "a'b", "x;y"] {
            assert!(validate_service(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn nls_statement_sets_all_four() {
        let s = nls_statement();
        assert!(s.starts_with("ALTER SESSION SET "));
        assert!(s.contains("NLS_DATE_FORMAT='YYYY-MM-DD HH24:MI:SS'"));
        assert!(s.contains("NLS_TIMESTAMP_FORMAT='YYYY-MM-DD HH24:MI:SS.FF'"));
        assert!(s.contains("NLS_TIMESTAMP_TZ_FORMAT='YYYY-MM-DD HH24:MI:SS.FF TZH:TZM'"));
        assert!(s.contains("NLS_NUMERIC_CHARACTERS='.,'"));
    }

    fn ora(code: i32, msg: &str) -> OraErr {
        OraErr { ora: Some(code), dpi: None, message: msg.into() }
    }

    #[test]
    fn ora_codes_map_to_messages() {
        assert_eq!(map_error(&ora(1017, "ORA-01017: invalid username/password"), Some("pw")), AUTH);
        for c in [12541, 12545, 12170, 12543] {
            assert_eq!(map_error(&ora(c, "x"), None), UNREACHABLE, "{c}");
        }
        assert_eq!(map_error(&ora(12514, "x"), None), UNKNOWN_SERVICE);
        assert_eq!(map_error(&ora(28000, "x"), None), ACCOUNT_LOCKED);
        assert_eq!(map_error(&ora(28001, "x"), None), PASSWORD_EXPIRED);
        for c in [3113, 3114, 3135] {
            assert_eq!(map_error(&ora(c, "x"), None), LOST);
            assert!(ora(c, "x").is_connection_lost());
        }
        assert!(OraErr { ora: None, dpi: Some(1080), message: "DPI-1080".into() }.is_connection_lost());
        assert!(!ora(942, "ORA-00942").is_connection_lost());
        assert_eq!(map_error(&ora(942, "ORA-00942: table or view does not exist"), None), "ORA-00942: table or view does not exist");
    }

    #[test]
    fn other_messages_are_sanitized() {
        let m = map_error(&ora(1, "boom password=hunter2 fine hunter2 (DESCRIPTION=(ADDRESS=(HOST=h))) resto"), Some("hunter2"));
        assert!(!m.contains("hunter2"), "{m}");
        assert!(!m.contains("DESCRIPTION"), "{m}");
    }

    #[test]
    fn real_driver_errors_carry_codes() {
        #[allow(deprecated)]
        let e = oracle::Error::OciError(oracle::DbError::new(1017, 0, "ORA-01017: invalid username/password", "fn", "act"));
        let o = OraErr::from(&e);
        assert_eq!(o.ora, Some(1017));
        assert_eq!(map_error(&o, None), AUTH);
        #[allow(deprecated)]
        let e = oracle::Error::OciError(oracle::DbError::new(12514, 0, "ORA-12514", "fn", "act"));
        assert_eq!(map_error(&OraErr::from(&e), None), UNKNOWN_SERVICE);
    }

    #[test]
    fn no_client_dir_is_the_oracle_client_code() {
        let conns = Arc::new(Connections::new(crate::ssh::conn::SshPaths::new(std::env::temp_dir().join("ora-test-ssh")), Arc::new(|_| {})));
        let s = Arc::new(OraSessions::new(conns, Arc::new(|_| {})));
        let r = tauri::async_runtime::block_on(s.connect("c1", params(), None, None, None));
        assert_eq!(r.unwrap_err(), ORACLE_CLIENT);
        let st = s.states();
        assert_eq!(st.len(), 1);
        assert_eq!(st[0].state, ConnState::Error);
        assert!(s.actor("c1").is_err());
    }

    // ---- finto backend ----

    #[derive(Default)]
    struct Probe {
        log: Mutex<Vec<String>>,
        broke: AtomicBool,
        pings: AtomicUsize,
        dropped: AtomicUsize,
    }

    struct Fake {
        probe: Arc<Probe>,
        ping_results: Vec<Result<(), OraErr>>,
    }

    impl Backend for Fake {
        fn ping(&mut self) -> Result<(), OraErr> {
            let n = self.probe.pings.fetch_add(1, Ordering::SeqCst);
            self.probe.log.lock().unwrap().push(format!("ping{n}"));
            if self.ping_results.is_empty() { Ok(()) } else { self.ping_results.remove(0) }
        }
        fn breaker(&self) -> Breaker {
            let p = self.probe.clone();
            Arc::new(move || {
                p.broke.store(true, Ordering::SeqCst);
                Ok(())
            })
        }
    }

    impl Drop for Fake {
        fn drop(&mut self) {
            self.probe.dropped.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn start(probe: &Arc<Probe>, ping_results: Vec<Result<(), OraErr>>, every: Duration) -> (Arc<Actor>, mpsc::Receiver<String>) {
        let (end_tx, end_rx) = mpsc::channel();
        let p = probe.clone();
        let pending = spawn(move || Ok(Fake { probe: p, ping_results }), Timing { ping_every: every }, move |r| {
            let _ = end_tx.send(r);
        });
        (pending.ready_blocking().unwrap(), end_rx)
    }

    #[test]
    fn requests_are_answered_in_order_and_close_joins() {
        let probe = Arc::new(Probe::default());
        let (a, end) = start(&probe, vec![], Duration::from_secs(3600));
        for _ in 0..3 {
            a.request_blocking(Request::Ping).unwrap();
        }
        assert_eq!(*probe.log.lock().unwrap(), vec!["ping0", "ping1", "ping2"]);
        a.shutdown();
        // il thread è uscito e il backend è stato rilasciato (la connessione si chiude)
        assert_eq!(probe.dropped.load(Ordering::SeqCst), 1);
        assert!(probe.broke.load(Ordering::SeqCst));
        // chiusura voluta: nessuna notifica di caduta
        assert!(end.try_recv().is_err());
        assert!(a.request_blocking(Request::Ping).is_err());
    }

    #[test]
    fn dropping_the_actor_joins_the_thread() {
        let probe = Arc::new(Probe::default());
        let (a, _end) = start(&probe, vec![], Duration::from_secs(3600));
        drop(a);
        assert_eq!(probe.dropped.load(Ordering::SeqCst), 1);
    }

    /// Backend la cui query lenta resta in corso finché non arriva `break`.
    struct Slow {
        probe: Arc<Probe>,
        started: mpsc::Sender<()>,
        broken: Arc<AtomicBool>,
    }
    impl Backend for Slow {
        fn ping(&mut self) -> Result<(), OraErr> {
            // il «ping» fa da query lenta
            let _ = self.started.send(());
            while !self.broken.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(2));
            }
            self.probe.log.lock().unwrap().push("interrotta".into());
            Err(OraErr { ora: Some(1013), dpi: None, message: "ORA-01013: user requested cancel of current operation".into() })
        }
        fn breaker(&self) -> Breaker {
            let b = self.broken.clone();
            Arc::new(move || {
                b.store(true, Ordering::SeqCst);
                Ok(())
            })
        }
    }

    #[test]
    fn break_is_delivered_while_a_slow_request_runs() {
        let probe = Arc::new(Probe::default());
        let (started_tx, started_rx) = mpsc::channel();
        let broken = Arc::new(AtomicBool::new(false));
        let (p, b) = (probe.clone(), broken.clone());
        let a = spawn(move || Ok(Slow { probe: p, started: started_tx, broken: b }), Timing::default(), |_| {}).ready_blocking().unwrap();
        let a2 = a.clone();
        let worker = std::thread::spawn(move || a2.request_blocking(Request::Ping));
        started_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        // la richiesta è in corso nel thread: il canale non serve, il breaker sì
        assert!(!worker.is_finished());
        a.break_execution().unwrap();
        let r = worker.join().unwrap();
        assert!(r.unwrap_err().contains("ORA-01013"));
        assert_eq!(*probe.log.lock().unwrap(), vec!["interrotta"]);
        a.shutdown();
    }

    #[test]
    fn idle_pings_and_two_failures_drop_the_session() {
        let probe = Arc::new(Probe::default());
        let fail = || Err(OraErr::new("rete"));
        let (_a, end) = start(&probe, vec![Ok(()), fail(), fail()], Duration::from_millis(10));
        let reason = end.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(reason, "rete");
        // ok, errore, errore = 3 ping; un successo azzera il conto
        assert_eq!(probe.pings.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn a_single_failure_between_successes_is_tolerated() {
        let probe = Arc::new(Probe::default());
        let r = vec![Err(OraErr::new("x")), Ok(()), Err(OraErr::new("x")), Ok(())];
        let (a, end) = start(&probe, r, Duration::from_millis(10));
        std::thread::sleep(Duration::from_millis(120));
        assert!(end.try_recv().is_err());
        assert!(probe.pings.load(Ordering::SeqCst) >= 4);
        a.shutdown();
    }

    #[test]
    fn connection_lost_on_a_request_ends_the_session_at_once() {
        let probe = Arc::new(Probe::default());
        let lost = Err(OraErr { ora: Some(3113), dpi: None, message: "ORA-03113".into() });
        let (a, end) = start(&probe, vec![lost], Duration::from_secs(3600));
        let r = a.request_blocking(Request::Ping);
        assert_eq!(r.unwrap_err(), LOST);
        assert_eq!(end.recv_timeout(Duration::from_secs(5)).unwrap(), "ORA-03113");
        assert_eq!(probe.dropped.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn connect_failure_and_timeout() {
        let p = spawn::<Fake>(|| Err("Servizio sconosciuto sul server".into()), Timing::default(), |_| {});
        assert_eq!(p.ready_blocking().err().unwrap(), UNKNOWN_SERVICE);
        // connessione lenta: il chiamante scade e il thread, a connessione finita, si ritira da solo
        let probe = Arc::new(Probe::default());
        let pr = probe.clone();
        let p = spawn(
            move || {
                std::thread::sleep(Duration::from_millis(150));
                Ok(Fake { probe: pr, ping_results: vec![] })
            },
            Timing::default(),
            |_| {},
        );
        let r = tauri::async_runtime::block_on(p.ready(Duration::from_millis(20)));
        assert_eq!(r.err().unwrap(), TIMED_OUT);
        let start = Instant::now();
        while probe.dropped.load(Ordering::SeqCst) == 0 && start.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(probe.dropped.load(Ordering::SeqCst), 1);
    }
}
