//! Sessione Postgres: un `Client` di `tokio-postgres` più il task della sua connessione, per connessione salvata.
//! TLS con `native-tls` (Portachiavi di sistema), tunnel SSH opzionale (`tunnel.rs`). La password arriva dal
//! Portachiavi solo qui e va solo a `Config::password`: mai in stringhe di connessione, log, eventi o errori.
use super::tunnel::{self, Tunnel};
use super::DbTls;
use crate::lock;
use crate::ssh::conn::{ConnState, Connections, StateEvent, AUTH};
use crate::ssh::Server;
use std::collections::HashMap;
use std::future::Future;
use std::net::{IpAddr, Ipv4Addr};
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio_postgres::config::SslMode;
use tokio_postgres::{Client, Config, NoTls};

pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
pub const APPLICATION_NAME: &str = "Code Art";
pub const NO_TLS: &str = "Il server non accetta TLS";
pub const UNREACHABLE: &str = "Server non raggiungibile";
pub const TIMED_OUT: &str = "Server non raggiungibile (tempo scaduto)";
pub const LOST: &str = "Connessione al database persa";
/// Aggiunto agli errori di verifica del certificato (`prefer`/`require` verificano sempre).
pub const CERT_HINT: &str = "Se il server usa un certificato autofirmato, abilita «Accetta certificati non validi» nella connessione (oppure usa TLS `disable`)";

/// Dati per aprire una connessione (senza password e senza tunnel: quelli si passano a parte).
#[derive(Debug, Clone, PartialEq)]
pub struct PgParams {
    pub host: String,
    pub port: u16,
    pub user: Option<String>,
    pub database: Option<String>,
    pub tls: DbTls,
    pub accept_invalid_certs: bool,
}

impl PgParams {
    pub fn from_connection(c: &super::DbConnection) -> PgParams {
        PgParams {
            host: c.host.clone(),
            port: c.port,
            user: c.user.clone(),
            database: c.database.clone(),
            tls: c.tls,
            accept_invalid_certs: c.accept_invalid_certs,
        }
    }
}

/// `Config` con i campi separati (mai una stringa di connessione). Con un tunnel il nome per il certificato resta
/// l'host vero (`host`), mentre ci si collega a 127.0.0.1 sulla porta locale (`hostaddr`).
pub fn build_config(p: &PgParams, password: Option<&str>, local_port: Option<u16>) -> Config {
    let mut c = Config::new();
    c.host(&p.host);
    match local_port {
        Some(lp) => {
            c.hostaddr(IpAddr::V4(Ipv4Addr::LOCALHOST));
            c.port(lp);
        }
        None => {
            c.port(p.port);
        }
    }
    c.user(p.user.as_deref().unwrap_or("postgres"));
    if let Some(d) = &p.database {
        c.dbname(d);
    }
    if let Some(pw) = password {
        c.password(pw);
    }
    c.connect_timeout(CONNECT_TIMEOUT);
    c.application_name(APPLICATION_NAME);
    c.ssl_mode(match p.tls {
        DbTls::Disable => SslMode::Disable,
        DbTls::Prefer => SslMode::Prefer,
        DbTls::Require => SslMode::Require,
    });
    c
}

/// Toglie dal testo ogni frammento `password=...` e la password stessa, se presente.
pub fn sanitize(text: &str, password: Option<&str>) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    // minuscole solo ASCII: gli indici restano quelli del testo originale (`to_lowercase` cambia la lunghezza di `İ`)
    while let Some(i) = rest.to_ascii_lowercase().find("password=") {
        out.push_str(&rest[..i]);
        out.push_str("password=***");
        let tail = &rest[i + "password=".len()..];
        let end = tail.find(char::is_whitespace).unwrap_or(tail.len());
        rest = &tail[end..];
    }
    out.push_str(rest);
    match password {
        Some(pw) if !pw.is_empty() => out.replace(pw, "***"),
        _ => out,
    }
}

/// Messaggio (o `auth`) per un errore di connessione: `sqlstate` se è un errore del server, `text` = catena dei
/// `Display` dell'errore.
pub fn classify(sqlstate: Option<&str>, text: &str, password: Option<&str>) -> String {
    if matches!(sqlstate, Some("28P01" | "28000")) {
        return AUTH.into();
    }
    let low = text.to_lowercase();
    // niente password salvata e il server la vuole
    if low.contains("password missing") {
        return AUTH.into();
    }
    if low.contains("does not support tls") {
        return NO_TLS.into();
    }
    let clean = sanitize(text, password);
    if sqlstate.is_some() {
        return format!("Errore del server: {clean}");
    }
    if low.contains("certificate") {
        return format!("Errore TLS: {clean}. {CERT_HINT}");
    }
    if ["handshake", "tls", "ssl"].iter().any(|k| low.contains(k)) {
        return format!("Errore TLS: {clean}");
    }
    if ["connection refused", "timed out", "timeout", "lookup", "dns", "no route", "unreachable", "failed to connect", "connection reset", "broken pipe", "os error"]
        .iter()
        .any(|k| low.contains(k))
    {
        return format!("{UNREACHABLE}: {clean}");
    }
    clean
}

pub(super) fn error_text(e: &tokio_postgres::Error) -> String {
    use std::error::Error;
    let mut s = e.to_string();
    let mut src = e.source();
    while let Some(x) = src {
        s.push_str(": ");
        s.push_str(&x.to_string());
        src = x.source();
    }
    s
}

pub(super) fn map_error(e: &tokio_postgres::Error, password: Option<&str>) -> String {
    classify(e.code().map(|c| c.code()), &error_text(e), password)
}

type ConnFuture = Pin<Box<dyn Future<Output = Result<(), tokio_postgres::Error>> + Send>>;

/// Apre il `Client` e la future della sua connessione (da far girare con `spawn`). Con la password esplicita
/// (i test non passano dal Portachiavi).
pub async fn open_client(p: &PgParams, password: Option<&str>, local_port: Option<u16>) -> Result<(Client, ConnFuture), String> {
    // TCP + TLS + autenticazione, tutto entro il timeout
    tokio::time::timeout(CONNECT_TIMEOUT, open_client_inner(p, password, local_port)).await.unwrap_or_else(|_| Err(TIMED_OUT.into()))
}

/// Il connettore TLS con le stesse regole della connessione (anche per l'annullamento delle query).
pub fn tls_connector(p: &PgParams) -> Result<postgres_native_tls::MakeTlsConnector, String> {
    let mut b = native_tls::TlsConnector::builder();
    if p.accept_invalid_certs {
        b.danger_accept_invalid_certs(true).danger_accept_invalid_hostnames(true);
    }
    Ok(postgres_native_tls::MakeTlsConnector::new(b.build().map_err(|e| format!("Errore TLS: {e}"))?))
}

async fn open_client_inner(p: &PgParams, password: Option<&str>, local_port: Option<u16>) -> Result<(Client, ConnFuture), String> {
    let cfg = build_config(p, password, local_port);
    match p.tls {
        DbTls::Disable => {
            let (client, conn) = cfg.connect(NoTls).await.map_err(|e| map_error(&e, password))?;
            Ok((client, Box::pin(conn)))
        }
        DbTls::Prefer | DbTls::Require => {
            let (client, conn) = cfg.connect(tls_connector(p)?).await.map_err(|e| map_error(&e, password))?;
            Ok((client, Box::pin(conn)))
        }
    }
}

struct Session {
    state: ConnState,
    message: Option<String>,
    generation: u64,
    client: Option<Arc<Client>>,
    cancel: Option<tokio_postgres::CancelToken>,
    params: Option<PgParams>,
    task: Option<tauri::async_runtime::JoinHandle<()>>,
    tunnel: Option<Tunnel>,
}

/// Una sessione viva, per eseguire query. `generation` cambia a ogni nuova connessione.
#[derive(Clone)]
pub struct Handle {
    pub client: Arc<Client>,
    pub cancel: tokio_postgres::CancelToken,
    pub params: PgParams,
    pub generation: u64,
}

pub type Emitter = Arc<dyn Fn(StateEvent) + Send + Sync>;

/// Le sessioni Postgres aperte, una per connessione salvata, valide per tutte le finestre.
pub struct Sessions {
    conns: Arc<Connections>,
    map: Mutex<HashMap<String, Session>>,
    next: AtomicU64,
    emit: Emitter,
}

impl Sessions {
    pub fn new(conns: Arc<Connections>, emit: Emitter) -> Self {
        Sessions { conns, map: Mutex::new(HashMap::new()), next: AtomicU64::new(1), emit }
    }

    pub fn states(&self) -> Vec<StateEvent> {
        let mut v: Vec<StateEvent> =
            lock(&self.map).iter().map(|(id, s)| StateEvent { id: id.clone(), state: s.state, message: s.message.clone() }).collect();
        v.sort_by(|a, b| a.id.cmp(&b.id));
        v
    }

    /// Il `Client` della connessione, se è `connected` (per le query).
    pub fn client(&self, id: &str) -> Result<Arc<Client>, String> {
        match lock(&self.map).get(id) {
            Some(s) if s.state == ConnState::Connected => s.client.clone().ok_or_else(|| "Non connesso al database".to_string()),
            _ => Err("Non connesso al database".into()),
        }
    }

    /// Tutto quel che serve per una query: client, token di annullamento, parametri (per il TLS) e generazione.
    pub fn handle(&self, id: &str) -> Result<Handle, String> {
        match lock(&self.map).get(id) {
            Some(Session { state: ConnState::Connected, client: Some(c), cancel: Some(t), params: Some(p), generation, .. }) => {
                Ok(Handle { client: c.clone(), cancel: t.clone(), params: p.clone(), generation: *generation })
            }
            _ => Err("Non connesso al database".into()),
        }
    }

    fn emit(&self, id: &str, state: ConnState, message: Option<String>) {
        (self.emit)(StateEvent { id: id.to_string(), state, message });
    }

    /// Nuova voce nello stato `state` (sostituisce la vecchia: chi la teneva ne esce con la generazione superata).
    /// Restituisce la generazione e la vecchia voce, da chiudere fuori dal lucchetto.
    fn replace(&self, id: &str, state: ConnState, message: Option<String>) -> (u64, Option<Session>) {
        let generation = self.next.fetch_add(1, Ordering::Relaxed);
        let old = lock(&self.map).insert(id.to_string(), Session { state, message, generation, client: None, cancel: None, params: None, task: None, tunnel: None });
        (generation, old)
    }

    fn teardown(&self, old: Option<Session>) {
        if let Some(mut s) = old {
            if let Some(t) = s.task.take() {
                t.abort();
            }
            s.client = None;
            if let Some(t) = s.tunnel.take() {
                tunnel::close(&self.conns, &t);
            }
        }
    }

    /// Connette. Già connesso o in connessione: `Ok` subito. `Err("auth")` = serve la password. Da chiamare dal
    /// runtime async; il lavoro bloccante (tunnel) va su un thread a parte.
    pub async fn connect(
        self: &Arc<Self>,
        id: &str,
        p: PgParams,
        password: Option<String>,
        ssh_server: Option<Server>,
    ) -> Result<(), String> {
        let generation = {
            let mut map = lock(&self.map);
            if matches!(map.get(id).map(|s| s.state), Some(ConnState::Connecting | ConnState::Connected)) {
                return Ok(());
            }
            let generation = self.next.fetch_add(1, Ordering::Relaxed);
            map.insert(
                id.to_string(),
                Session { state: ConnState::Connecting, message: None, generation, client: None, cancel: None, params: None, task: None, tunnel: None },
            );
            generation
        };
        self.emit(id, ConnState::Connecting, None);
        match self.establish(id, generation, &p, password.as_deref(), ssh_server).await {
            Ok(()) => Ok(()),
            Err(e) => {
                let shown = if e == AUTH { "Autenticazione non riuscita".to_string() } else { e.clone() };
                if self.set_if(id, generation, ConnState::Error, Some(shown)) {
                    // un tunnel già aperto non serve più
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

    async fn establish(
        self: &Arc<Self>,
        id: &str,
        generation: u64,
        p: &PgParams,
        password: Option<&str>,
        ssh_server: Option<Server>,
    ) -> Result<(), String> {
        let mut local_port = None;
        if let Some(server) = ssh_server {
            let conns = self.conns.clone();
            let (host, port) = (p.host.clone(), p.port);
            let t = tauri::async_runtime::spawn_blocking(move || tunnel::open(&conns, &server, &host, port))
                .await
                .map_err(|e| e.to_string())??;
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
                // disconnessa nel frattempo: l'inoltro non serve
                let conns = self.conns.clone();
                let _ = tauri::async_runtime::spawn_blocking(move || tunnel::close(&conns, &t)).await;
                return Err("Connessione annullata".into());
            }
        }
        let (client, conn) = open_client(p, password, local_port).await?;
        let me = self.clone();
        let sid = id.to_string();
        let task = tauri::async_runtime::spawn(async move {
            let r = conn.await;
            me.ended(&sid, generation, r.err().map(|e| error_text(&e)));
        });
        let mut map = lock(&self.map);
        match map.get_mut(id) {
            Some(s) if s.generation == generation => {
                s.state = ConnState::Connected;
                s.message = None;
                s.cancel = Some(client.cancel_token());
                s.params = Some(p.clone());
                s.client = Some(Arc::new(client));
                s.task = Some(task);
            }
            _ => {
                task.abort();
                return Err("Connessione annullata".into());
            }
        }
        drop(map);
        self.emit(id, ConnState::Connected, None);
        Ok(())
    }

    /// La connessione si è chiusa (server, rete, tunnel caduto). Se non è stata l'utente → `disconnected`.
    fn ended(self: &Arc<Self>, id: &str, generation: u64, err: Option<String>) {
        let tunnel = {
            let mut map = lock(&self.map);
            match map.get_mut(id) {
                Some(s) if s.generation == generation && s.state == ConnState::Connected => {
                    s.state = ConnState::Disconnected;
                    s.message = Some(err.map(|e| format!("{LOST}: {}", sanitize(&e, None))).unwrap_or_else(|| LOST.into()));
                    s.client = None;
                    s.cancel = None;
                    s.task = None;
                    s.tunnel.take()
                }
                _ => return,
            }
        };
        let message = lock(&self.map).get(id).and_then(|s| s.message.clone());
        self.emit(id, ConnState::Disconnected, message);
        if let Some(t) = tunnel {
            let conns = self.conns.clone();
            tauri::async_runtime::spawn_blocking(move || tunnel::close(&conns, &t));
        }
    }

    /// Chiusura voluta: chiude il client, ferma il task, toglie il tunnel. Da un thread che può bloccarsi.
    pub fn disconnect(&self, id: &str) {
        if !lock(&self.map).contains_key(id) {
            return;
        }
        let (_, old) = self.replace(id, ConnState::Disconnected, None);
        self.teardown(old);
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

    fn params(tls: DbTls) -> PgParams {
        PgParams { host: "db.example.test".into(), port: 5432, user: Some("app".into()), database: Some("d".into()), tls, accept_invalid_certs: false }
    }

    #[test]
    fn config_never_leaks_the_password() {
        let c = build_config(&params(DbTls::Prefer), Some("s3kr3t-pw"), None);
        assert!(!format!("{c:?}").contains("s3kr3t-pw"));
        assert_eq!(c.get_password(), Some(&b"s3kr3t-pw"[..]));
        assert_eq!(c.get_application_name(), Some("Code Art"));
        assert_eq!(c.get_connect_timeout(), Some(&Duration::from_secs(10)));
        assert_eq!(c.get_user(), Some("app"));
        assert_eq!(c.get_dbname(), Some("d"));
    }

    #[test]
    fn tls_modes_and_tunnel_address() {
        assert_eq!(build_config(&params(DbTls::Prefer), None, None).get_ssl_mode(), SslMode::Prefer);
        assert_eq!(build_config(&params(DbTls::Require), None, None).get_ssl_mode(), SslMode::Require);
        assert_eq!(build_config(&params(DbTls::Disable), None, None).get_ssl_mode(), SslMode::Disable);
        let c = build_config(&params(DbTls::Prefer), None, Some(50123));
        // si va al tunnel, ma il nome per il certificato resta quello vero
        assert_eq!(c.get_ports(), &[50123]);
        assert_eq!(c.get_hostaddrs(), &[IpAddr::V4(Ipv4Addr::LOCALHOST)]);
        assert_eq!(format!("{:?}", c.get_hosts()), format!("{:?}", build_config(&params(DbTls::Prefer), None, None).get_hosts()));
    }

    #[test]
    fn sanitize_removes_password_fragments() {
        assert_eq!(sanitize("host=a password=hunter2 user=b", None), "host=a password=*** user=b");
        assert_eq!(sanitize("Password=x", None), "password=***");
        assert_eq!(sanitize("boom hunter2 boom", Some("hunter2")), "boom *** boom");
        assert_eq!(sanitize("nulla", Some("")), "nulla");
        // caratteri che cambiano lunghezza in minuscolo: niente panico, indici giusti
        assert_eq!(sanitize("İİ password=x İ", None), "İİ password=*** İ");
        assert_eq!(sanitize("\u{212A}elvin PASSWORD=y", None), "\u{212A}elvin password=***");
    }

    #[test]
    fn classification() {
        assert_eq!(classify(Some("28P01"), "password authentication failed for user \"a\"", Some("pw")), AUTH);
        assert_eq!(classify(Some("28000"), "no pg_hba.conf entry", None), AUTH);
        assert_eq!(classify(None, "invalid configuration: password missing", None), AUTH);
        assert_eq!(classify(None, "error performing TLS handshake: server does not support TLS", None), NO_TLS);
        let cert = classify(None, "error performing TLS handshake: certificate verify failed", None);
        assert!(cert.starts_with("Errore TLS") && cert.contains("Accetta certificati non validi"), "{cert}");
        assert!(!classify(None, "error performing TLS handshake: bad record", None).contains("Accetta"));
        assert!(classify(None, "error connecting to server: Connection refused (os error 61)", None).starts_with(UNREACHABLE));
        assert!(classify(None, "timed out", None).starts_with(UNREACHABLE));
        assert_eq!(classify(Some("3D000"), "database \"x\" does not exist", None), "Errore del server: database \"x\" does not exist");
        // nessun segreto in uscita, in nessun ramo
        for (st, t) in [(None, "x password=pw1 y"), (Some("3D000"), "oops pw1"), (None, "certificate pw1"), (None, "refused pw1")] {
            assert!(!classify(st, t, Some("pw1")).contains("pw1"), "{t}");
        }
    }

    #[test]
    fn unsupported_state_without_session() {
        let conns = Arc::new(Connections::new(crate::ssh::conn::SshPaths::new(std::env::temp_dir()), Arc::new(|_| {})));
        let s = Sessions::new(conns, Arc::new(|_| {}));
        assert!(s.client("x").is_err());
        assert!(s.states().is_empty());
        s.disconnect("x");
        s.close_all();
    }

    #[test]
    #[ignore = "avvia un cluster Postgres temporaneo (initdb e pg_ctl nel PATH)"]
    fn live_cluster_disable_and_password() {
        let Some(cluster) = crate::db::testutil::Cluster::start() else {
            eprintln!("initdb/pg_ctl non trovati: saltato");
            return;
        };
        let p = PgParams { host: "127.0.0.1".into(), port: cluster.port, user: Some("codeart".into()), database: Some("postgres".into()), tls: DbTls::Disable, accept_invalid_certs: false };
        tauri::async_runtime::block_on(async {
            let (client, conn) = open_client(&p, Some("test-pass"), None).await.expect("connessione");
            let task = tauri::async_runtime::spawn(conn);
            let row = client.query_one("SELECT 1::int4", &[]).await.unwrap();
            assert_eq!(row.get::<_, i32>(0), 1);
            drop(client);
            let _ = task.await;
            // password sbagliata: codice `auth`, mai la password nel messaggio
            let err = open_client(&p, Some("sbagliata"), None).await.err().expect("deve fallire");
            assert_eq!(err, AUTH);
            // senza password
            assert_eq!(open_client(&p, None, None).await.err().expect("deve fallire"), AUTH);
            // `require` su un server senza TLS: messaggio chiaro
            let mut r = p.clone();
            r.tls = DbTls::Require;
            assert_eq!(open_client(&r, Some("test-pass"), None).await.err().expect("deve fallire"), NO_TLS);
            // porta chiusa
            let mut c = p.clone();
            c.port = super::tunnel::pick_port().unwrap();
            let e = open_client(&c, Some("test-pass"), None).await.err().expect("deve fallire");
            assert!(e.starts_with(UNREACHABLE), "{e}");
            // una sessione completa: connect, query, caduta del server -> disconnected, disconnect volontario
            let conns = Arc::new(Connections::new(crate::ssh::conn::SshPaths::new(std::env::temp_dir()), Arc::new(|_| {})));
            let log = Arc::new(Mutex::new(Vec::<StateEvent>::new()));
            let l2 = log.clone();
            let sessions = Arc::new(Sessions::new(conns, Arc::new(move |e| l2.lock().unwrap().push(e))));
            sessions.connect("c1", p.clone(), Some("test-pass".into()), None).await.unwrap();
            let q = sessions.client("c1").unwrap().query_one("SELECT 2::int4", &[]).await.unwrap();
            assert_eq!(q.get::<_, i32>(0), 2);
            sessions.connect("c1", p.clone(), Some("test-pass".into()), None).await.unwrap(); // no-op
            sessions.disconnect("c1");
            assert!(sessions.client("c1").is_err());
            let states: Vec<ConnState> = log.lock().unwrap().iter().map(|e| e.state).collect();
            assert_eq!(states, [ConnState::Connecting, ConnState::Connected, ConnState::Disconnected]);
            // password sbagliata: stato error, Err("auth")
            assert_eq!(sessions.connect("c2", p.clone(), Some("x".into()), None).await.unwrap_err(), AUTH);
            assert_eq!(sessions.states().iter().find(|s| s.id == "c2").unwrap().state, ConnState::Error);
            // caduta: si ferma il cluster mentre la sessione è aperta
            sessions.connect("c3", p.clone(), Some("test-pass".into()), None).await.unwrap();
            cluster.stop();
            for _ in 0..50 {
                if sessions.states().iter().find(|s| s.id == "c3").unwrap().state == ConnState::Disconnected {
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            assert_eq!(sessions.states().iter().find(|s| s.id == "c3").unwrap().state, ConnState::Disconnected);
        });
    }
}
