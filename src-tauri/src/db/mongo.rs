//! Sessione, albero, ricerche e modifiche MongoDB (driver ufficiale, TLS `rustls`). Il `Client` si costruisce da
//! `ClientOptions` programmatiche: mai una URI con la password. Filtri, ordinamenti, proiezioni e pipeline arrivano dalla
//! WebView come testo JSON esteso (rilassato o canonico) e il core li trasforma in documenti BSON: niente è mai
//! valutato come codice. I documenti tornano come JSON esteso rilassato (stringhe) più le colonne di primo livello.
use super::pg::{sanitize, Emitter, CONNECT_TIMEOUT, TIMED_OUT, UNREACHABLE};
use super::pg_query::{truncate_cell, QueryColumn, QueryPage, QueryResult, EXPIRED, MAX_TOTAL, PAGE};
use super::tunnel::{self, Tunnel};
use super::{DbConnection, DbTls};
use crate::lock;
use crate::ssh::conn::{ConnState, Connections, StateEvent, AUTH};
use crate::ssh::Server;
use mongodb::bson::{doc, Bson, Document};
use mongodb::error::{Error as MongoError, ErrorKind};
use mongodb::options::{ClientOptions, Credential, ServerAddress, Tls, TlsOptions};
use mongodb::{Client, Cursor};
use serde::de::{Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tauri::async_runtime::Mutex as AsyncMutex;

pub const APP_NAME: &str = "Code Art";
/// Gli id dei risultati MongoDB partono da qui: così `db_query_more`/`db_query_close` distinguono i due tipi.
pub const ID_BASE: u64 = 1 << 62;
pub const NOT_CONNECTED: &str = "Non connesso al database";
pub const EDIT_TITLE: &str = "Modifica documento";
pub const DELETE_TITLE: &str = "Elimina documento";
pub const WRITES_TITLE: &str = "La pipeline scrive dati";
pub const BTN_SAVE: &str = "Salva";
pub const BTN_INSERT: &str = "Inserisci";
pub const BTN_DELETE: &str = "Elimina";
pub const BTN_RUN: &str = "Esegui";

pub fn is_mongo_result(result_id: u64) -> bool {
    result_id >= ID_BASE
}

// ---------------------------------------------------------------------------------------------------------------
// JSON esteso → BSON (l'ordine delle chiavi conta: ordinamenti, proiezioni, pipeline)
// ---------------------------------------------------------------------------------------------------------------

/// JSON che conserva l'ordine delle chiavi (con `serde_json::Value` dipenderebbe da una feature del crate).
#[derive(Debug)]
enum Oj {
    Null,
    Bool(bool),
    Num(serde_json::Number),
    Str(String),
    Arr(Vec<Oj>),
    Obj(Vec<(String, Oj)>),
}

struct OjVisitor;

impl<'de> Visitor<'de> for OjVisitor {
    type Value = Oj;
    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("un valore JSON")
    }
    fn visit_unit<E>(self) -> Result<Oj, E> {
        Ok(Oj::Null)
    }
    fn visit_bool<E>(self, v: bool) -> Result<Oj, E> {
        Ok(Oj::Bool(v))
    }
    fn visit_i64<E>(self, v: i64) -> Result<Oj, E> {
        Ok(Oj::Num(v.into()))
    }
    fn visit_u64<E>(self, v: u64) -> Result<Oj, E> {
        Ok(Oj::Num(v.into()))
    }
    fn visit_f64<E: serde::de::Error>(self, v: f64) -> Result<Oj, E> {
        serde_json::Number::from_f64(v).map(Oj::Num).ok_or_else(|| E::custom("numero non finito"))
    }
    fn visit_str<E>(self, v: &str) -> Result<Oj, E> {
        Ok(Oj::Str(v.to_string()))
    }
    fn visit_string<E>(self, v: String) -> Result<Oj, E> {
        Ok(Oj::Str(v))
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut a: A) -> Result<Oj, A::Error> {
        let mut v = Vec::new();
        while let Some(x) = a.next_element()? {
            v.push(x);
        }
        Ok(Oj::Arr(v))
    }
    fn visit_map<A: MapAccess<'de>>(self, mut a: A) -> Result<Oj, A::Error> {
        let mut v = Vec::new();
        while let Some((k, x)) = a.next_entry::<String, Oj>()? {
            v.push((k, x));
        }
        Ok(Oj::Obj(v))
    }
}

impl<'de> Deserialize<'de> for Oj {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Oj, D::Error> {
        d.deserialize_any(OjVisitor)
    }
}

/// Chiavi che aprono un wrapper di JSON esteso (`{"$oid": …}`): gli altri oggetti con `$` (`$gt`, `$match`…) sono operatori.
const EXT_KEYS: [&str; 17] = [
    "$oid", "$date", "$numberInt", "$numberLong", "$numberDouble", "$numberDecimal", "$binary", "$uuid", "$timestamp",
    "$regularExpression", "$symbol", "$code", "$minKey", "$maxKey", "$undefined", "$dbPointer", "$regex",
];

fn is_wrapper(v: &[(String, Oj)]) -> bool {
    match v.first() {
        // `$regex` + `$options` sono anche un operatore di query: lasciati come documento
        Some((k, _)) if k == "$regex" => false,
        Some((k, _)) => EXT_KEYS.contains(&k.as_str()),
        None => false,
    }
}

fn oj_value(o: Oj) -> serde_json::Value {
    match o {
        Oj::Null => serde_json::Value::Null,
        Oj::Bool(b) => b.into(),
        Oj::Num(n) => serde_json::Value::Number(n),
        Oj::Str(s) => s.into(),
        Oj::Arr(v) => serde_json::Value::Array(v.into_iter().map(oj_value).collect()),
        Oj::Obj(v) => serde_json::Value::Object(v.into_iter().map(|(k, x)| (k, oj_value(x))).collect()),
    }
}

fn oj_bson(o: Oj) -> Result<Bson, String> {
    match o {
        Oj::Obj(v) if is_wrapper(&v) => Bson::try_from(oj_value(Oj::Obj(v))).map_err(|e| e.to_string()),
        Oj::Obj(v) => {
            let mut d = Document::new();
            for (k, x) in v {
                d.insert(k, oj_bson(x)?);
            }
            Ok(Bson::Document(d))
        }
        Oj::Arr(v) => Ok(Bson::Array(v.into_iter().map(oj_bson).collect::<Result<_, _>>()?)),
        other => Bson::try_from(oj_value(other)).map_err(|e| e.to_string()),
    }
}

fn parse_oj(text: &str) -> Result<Oj, String> {
    serde_json::from_str::<Oj>(text).map_err(|e| e.to_string())
}

/// Un documento da testo JSON esteso. `empty_ok`: testo vuoto = `{}`. `what` = «Filtro non valido»…
pub fn parse_document(what: &str, text: &str, empty_ok: bool) -> Result<Document, String> {
    if text.trim().is_empty() && empty_ok {
        return Ok(Document::new());
    }
    match parse_oj(text).and_then(oj_bson) {
        Ok(Bson::Document(d)) => Ok(d),
        Ok(_) => Err(format!("{what}: deve essere un oggetto JSON")),
        Err(e) => Err(format!("{what}: {e}")),
    }
}

/// Un valore BSON qualsiasi (l'`_id`: ObjectId, stringa, numero…).
pub fn parse_value(what: &str, text: &str) -> Result<Bson, String> {
    parse_oj(text).and_then(oj_bson).map_err(|e| format!("{what}: {e}"))
}

/// Pipeline: array JSON di stadi (oggetti); testo vuoto = errore.
pub fn parse_pipeline(text: &str) -> Result<Vec<Document>, String> {
    const W: &str = "Pipeline non valida";
    match parse_oj(text).and_then(oj_bson) {
        Ok(Bson::Array(v)) => v
            .into_iter()
            .enumerate()
            .map(|(i, s)| match s {
                Bson::Document(d) => Ok(d),
                _ => Err(format!("{W}: lo stadio {} deve essere un oggetto JSON", i + 1)),
            })
            .collect(),
        Ok(_) => Err(format!("{W}: deve essere un array di stadi")),
        Err(e) => Err(format!("{W}: {e}")),
    }
}

/// `$out` e `$merge` (chiavi di primo livello degli stadi) scrivono in una collezione.
pub fn pipeline_writes(stages: &[Document]) -> bool {
    stages.iter().any(|s| s.keys().any(|k| k == "$out" || k == "$merge"))
}

// ---------------------------------------------------------------------------------------------------------------
// BSON → JSON esteso rilassato, colonne e celle
// ---------------------------------------------------------------------------------------------------------------

fn leaf_json(b: &Bson) -> String {
    serde_json::to_string(&b.clone().into_relaxed_extjson()).unwrap_or_else(|_| "null".into())
}

fn write_json(b: &Bson, pretty: bool, level: usize, out: &mut String) {
    let nl = |out: &mut String, lv: usize| {
        if pretty {
            out.push('\n');
            out.push_str(&"  ".repeat(lv));
        }
    };
    match b {
        Bson::Document(d) if d.is_empty() => out.push_str("{}"),
        Bson::Document(d) => {
            out.push('{');
            for (i, (k, v)) in d.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                nl(out, level + 1);
                out.push_str(&serde_json::to_string(k).unwrap_or_default());
                out.push_str(if pretty { ": " } else { ":" });
                write_json(v, pretty, level + 1, out);
            }
            nl(out, level);
            out.push('}');
        }
        Bson::Array(a) if a.is_empty() => out.push_str("[]"),
        Bson::Array(a) => {
            out.push('[');
            for (i, v) in a.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                nl(out, level + 1);
                write_json(v, pretty, level + 1, out);
            }
            nl(out, level);
            out.push(']');
        }
        other => out.push_str(&leaf_json(other)),
    }
}

/// JSON esteso rilassato del documento, con l'ordine dei campi originale (`pretty` = rientrato, per l'editor).
pub fn doc_json(d: &Document, pretty: bool) -> String {
    let mut s = String::new();
    write_json(&Bson::Document(d.clone()), pretty, 0, &mut s);
    s
}

/// Testo di una cella: stringhe così come sono, tutto il resto JSON esteso rilassato compatto.
pub fn cell_text(b: &Bson) -> String {
    match b {
        Bson::String(s) => s.clone(),
        other => {
            let mut s = String::new();
            write_json(other, false, 0, &mut s);
            s
        }
    }
}

/// Colonne di primo livello della pagina: `_id` per prima, poi le altre nell'ordine in cui compaiono.
pub fn columns_of(docs: &[Document]) -> Vec<String> {
    let mut cols: Vec<String> = Vec::new();
    for d in docs {
        for k in d.keys() {
            if !cols.contains(k) {
                cols.push(k.clone());
            }
        }
    }
    if let Some(i) = cols.iter().position(|c| c == "_id") {
        let id = cols.remove(i);
        cols.insert(0, id);
    }
    cols
}

struct Built {
    docs: Vec<String>,
    columns: Vec<QueryColumn>,
    rows: Vec<Vec<Option<String>>>,
    truncated: Vec<[usize; 2]>,
}

fn build(docs: &[Document]) -> Built {
    let cols = columns_of(docs);
    let mut rows = Vec::with_capacity(docs.len());
    let mut truncated = Vec::new();
    for (ri, d) in docs.iter().enumerate() {
        let mut row = Vec::with_capacity(cols.len());
        for (ci, c) in cols.iter().enumerate() {
            row.push(d.get(c).map(|v| {
                let (s, t) = truncate_cell(&cell_text(v));
                if t {
                    truncated.push([ri, ci]);
                }
                s
            }));
        }
        rows.push(row);
    }
    Built {
        docs: docs.iter().map(|d| doc_json(d, true)).collect(),
        columns: cols.into_iter().map(|name| QueryColumn { name }).collect(),
        rows,
        truncated,
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Opzioni di connessione ed errori
// ---------------------------------------------------------------------------------------------------------------

/// Dati per aprire una connessione (senza password e senza tunnel: quelli si passano a parte).
#[derive(Debug, Clone, PartialEq)]
pub struct MongoParams {
    pub host: String,
    pub port: u16,
    pub user: Option<String>,
    pub database: Option<String>,
    pub tls: DbTls,
    pub accept_invalid_certs: bool,
}

impl MongoParams {
    pub fn from_connection(c: &DbConnection) -> MongoParams {
        MongoParams {
            host: c.host.clone(),
            port: c.port,
            user: c.user.clone(),
            database: c.database.clone(),
            tls: c.tls,
            accept_invalid_certs: c.accept_invalid_certs,
        }
    }
}

fn is_loopback(host: &str) -> bool {
    let h = host.trim_matches(|c| c == '[' || c == ']').to_ascii_lowercase();
    h == "localhost" || h.ends_with(".localhost") || h == "::1" || h.starts_with("127.")
}

/// MongoDB non ha un TLS opportunistico: `prefer` = TLS per gli host remoti, nessun TLS per localhost e per i tunnel
/// SSH (il certificato non corrisponderebbe a 127.0.0.1); `require` = TLS; `disable` = niente TLS.
pub fn tls_enabled(p: &MongoParams, tunnel: bool) -> bool {
    match p.tls {
        DbTls::Disable => false,
        DbTls::Require => true,
        DbTls::Prefer => !tunnel && !is_loopback(&p.host),
    }
}

/// `ClientOptions` dai campi (mai una URI). Utente senza password: `Err(AUTH)`, il server la vorrebbe comunque.
pub fn build_options(p: &MongoParams, password: Option<&str>, local_port: Option<u16>) -> Result<ClientOptions, String> {
    let address = match local_port {
        Some(lp) => ServerAddress::Tcp { host: "127.0.0.1".into(), port: Some(lp) },
        None => ServerAddress::Tcp { host: p.host.clone(), port: Some(p.port) },
    };
    let credential = match p.user.as_deref() {
        Some(user) => {
            let password = password.ok_or_else(|| AUTH.to_string())?;
            Some(
                Credential::builder()
                    .username(user.to_string())
                    .password(password.to_string())
                    .source(p.database.clone().unwrap_or_else(|| "admin".into()))
                    .build(),
            )
        }
        None => None,
    };
    let tls = if tls_enabled(p, local_port.is_some()) {
        let mut t = TlsOptions::builder().build();
        if p.accept_invalid_certs {
            t.allow_invalid_certificates = Some(true);
        }
        Tls::Enabled(t)
    } else {
        Tls::Disabled
    };
    let opts = ClientOptions::builder()
        .hosts(vec![address])
        .app_name(APP_NAME.to_string())
        .connect_timeout(CONNECT_TIMEOUT)
        .server_selection_timeout(CONNECT_TIMEOUT)
        .direct_connection(local_port.is_some())
        .credential(credential)
        .tls(tls)
        .build();
    Ok(opts)
}

/// Messaggio (o `auth`) da un errore: `auth_kind` = errore di autenticazione, `code`/`name` = errore di comando.
pub fn classify(auth_kind: bool, selection: bool, code: Option<i32>, code_name: &str, text: &str, password: Option<&str>) -> String {
    let low = text.to_lowercase();
    if auth_kind || code == Some(18) || code_name == "AuthenticationFailed" || low.contains("authentication failed") {
        return AUTH.into();
    }
    let clean = sanitize(text, password);
    if code.is_some() {
        return match code {
            Some(13) => format!("Non autorizzato: {clean}"),
            _ => format!("Errore del server: {clean} (codice {})", code.unwrap_or_default()),
        };
    }
    if ["certificate", "handshake", "tls", "ssl", "invalid peer"].iter().any(|k| low.contains(k)) {
        return format!("Errore TLS: {clean}");
    }
    if selection || ["connection refused", "timed out", "timeout", "dns", "no route", "unreachable", "connection reset", "broken pipe", "os error"].iter().any(|k| low.contains(k)) {
        return format!("{UNREACHABLE}: {clean}");
    }
    clean
}

fn error_text(e: &MongoError) -> String {
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

pub fn map_error(e: &MongoError, password: Option<&str>) -> String {
    let (auth, selection) = (
        matches!(*e.kind, ErrorKind::Authentication { .. }),
        matches!(*e.kind, ErrorKind::ServerSelection { .. } | ErrorKind::ConnectionPoolCleared { .. }),
    );
    let (code, name) = match &*e.kind {
        ErrorKind::Command(c) => (Some(c.code), c.code_name.clone()),
        _ => (None, String::new()),
    };
    let text = match &*e.kind {
        ErrorKind::Command(c) => c.message.clone(),
        _ => error_text(e),
    };
    classify(auth, selection, code, &name, &text, password)
}

// ---------------------------------------------------------------------------------------------------------------
// Sessioni
// ---------------------------------------------------------------------------------------------------------------

struct Session {
    state: ConnState,
    message: Option<String>,
    generation: u64,
    client: Option<Client>,
    tunnel: Option<Tunnel>,
}

/// Una sessione viva: client (clonabile, condivide il pool), parametri e generazione.
#[derive(Clone)]
pub struct Handle {
    pub client: Client,
    pub params: MongoParams,
    pub generation: u64,
}

struct Open {
    conn: String,
    generation: u64,
    cursor: Cursor<Document>,
    total: usize,
    /// Il cursore ha già un documento pronto (letto per sapere se restano righe).
    pending: bool,
    done: bool,
    /// Copia dei documenti già consegnati (interi, non troncati), per l'esportazione; al massimo `MAX_TOTAL`.
    docs: Vec<Document>,
}

/// Le sessioni MongoDB aperte (una per connessione salvata) e i loro risultati con cursore (al più uno per connessione:
/// una nuova ricerca chiude il precedente; dopo una riconnessione il risultato è scaduto).
pub struct MongoSessions {
    conns: Arc<Connections>,
    map: Mutex<HashMap<String, Session>>,
    params: Mutex<HashMap<String, MongoParams>>,
    next: AtomicU64,
    next_result: AtomicU64,
    results: Mutex<HashMap<u64, Arc<AsyncMutex<Open>>>>,
    by_conn: Mutex<HashMap<String, u64>>,
    emit: Emitter,
}

#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MongoDb {
    pub database: String,
    pub collections: Vec<String>,
}

fn check_name(what: &str, v: &str) -> Result<(), String> {
    if v.is_empty() || v.chars().any(|c| c.is_control()) {
        return Err(format!("{what}: nome non valido"));
    }
    Ok(())
}

impl MongoSessions {
    pub fn new(conns: Arc<Connections>, emit: Emitter) -> Self {
        MongoSessions {
            conns,
            map: Mutex::new(HashMap::new()),
            params: Mutex::new(HashMap::new()),
            next: AtomicU64::new(1),
            next_result: AtomicU64::new(ID_BASE),
            results: Mutex::new(HashMap::new()),
            by_conn: Mutex::new(HashMap::new()),
            emit,
        }
    }

    pub fn states(&self) -> Vec<StateEvent> {
        lock(&self.map).iter().map(|(id, s)| StateEvent { id: id.clone(), state: s.state, message: s.message.clone() }).collect()
    }

    pub fn knows(&self, id: &str) -> bool {
        lock(&self.map).contains_key(id)
    }

    pub fn handle(&self, id: &str) -> Result<Handle, String> {
        match (lock(&self.map).get(id), lock(&self.params).get(id)) {
            (Some(Session { state: ConnState::Connected, client: Some(c), generation, .. }), Some(p)) => {
                Ok(Handle { client: c.clone(), params: p.clone(), generation: *generation })
            }
            _ => Err(NOT_CONNECTED.into()),
        }
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

    fn teardown(&self, old: Option<Session>) {
        if let Some(mut s) = old {
            if let Some(c) = s.client.take() {
                tauri::async_runtime::spawn(async move { c.shutdown().await });
            }
            if let Some(t) = s.tunnel.take() {
                tunnel::close(&self.conns, &t);
            }
        }
    }

    fn drop_results_of(&self, id: &str) {
        let rid = lock(&self.by_conn).remove(id);
        if let Some(r) = rid {
            lock(&self.results).remove(&r);
        }
    }

    /// Connette (stessa logica della sessione Postgres). `Err("auth")` = serve la password.
    pub async fn connect(self: &Arc<Self>, id: &str, p: MongoParams, password: Option<String>, ssh_server: Option<Server>) -> Result<(), String> {
        let generation = {
            let mut map = lock(&self.map);
            if matches!(map.get(id).map(|s| s.state), Some(ConnState::Connecting | ConnState::Connected)) {
                return Ok(());
            }
            let generation = self.next.fetch_add(1, Ordering::Relaxed);
            map.insert(id.to_string(), Session { state: ConnState::Connecting, message: None, generation, client: None, tunnel: None });
            generation
        };
        self.emit(id, ConnState::Connecting, None);
        match self.establish(id, generation, &p, password.as_deref(), ssh_server).await {
            Ok(()) => Ok(()),
            Err(e) => {
                let shown = if e == AUTH { "Autenticazione non riuscita".to_string() } else { e.clone() };
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

    async fn establish(self: &Arc<Self>, id: &str, generation: u64, p: &MongoParams, password: Option<&str>, ssh_server: Option<Server>) -> Result<(), String> {
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
        let opts = build_options(p, password, local_port)?;
        let client = open_client(opts, p.database.as_deref().unwrap_or("admin"), password).await?;
        let mut map = lock(&self.map);
        match map.get_mut(id) {
            Some(s) if s.generation == generation => {
                s.state = ConnState::Connected;
                s.message = None;
                s.client = Some(client);
            }
            _ => return Err("Connessione annullata".into()),
        }
        drop(map);
        lock(&self.params).insert(id.to_string(), p.clone());
        self.emit(id, ConnState::Connected, None);
        Ok(())
    }

    /// Chiusura voluta: chiude il client, scarta i risultati, toglie il tunnel.
    pub fn disconnect(&self, id: &str) {
        let old = lock(&self.map).remove(id);
        if old.is_none() {
            return;
        }
        self.drop_results_of(id);
        lock(&self.params).remove(id);
        self.teardown(old);
        // lo stato resta noto come `disconnected`
        lock(&self.map).insert(id.to_string(), Session { state: ConnState::Disconnected, message: None, generation: self.next.fetch_add(1, Ordering::Relaxed), client: None, tunnel: None });
        self.emit(id, ConnState::Disconnected, None);
    }

    /// Uscita dall'app.
    pub fn close_all(&self) {
        let all: Vec<Session> = lock(&self.map).drain().map(|(_, s)| s).collect();
        lock(&self.results).clear();
        lock(&self.by_conn).clear();
        for s in all {
            self.teardown(Some(s));
        }
    }

    // ---- albero ----

    /// Database e collezioni, in ordine alfabetico. Senza il permesso di elencare i database: solo quello della
    /// connessione, se indicato.
    pub async fn tree(&self, id: &str) -> Result<Vec<MongoDb>, String> {
        let h = self.handle(id)?;
        let mut names = match h.client.list_database_names().await {
            Ok(n) => n,
            Err(e) => match (&h.params.database, map_error(&e, None).starts_with("Non autorizzato")) {
                (Some(d), true) => vec![d.clone()],
                _ => return Err(map_error(&e, None)),
            },
        };
        names.sort();
        let mut out = Vec::with_capacity(names.len());
        for database in names {
            // un database che non si può elencare compare senza collezioni
            let mut collections = h.client.database(&database).list_collection_names().await.unwrap_or_default();
            collections.sort();
            out.push(MongoDb { database, collections });
        }
        Ok(out)
    }

    // ---- ricerca ----

    fn collection(&self, id: &str, db: &str, coll: &str) -> Result<(Handle, mongodb::Collection<Document>), String> {
        check_name("Database", db)?;
        check_name("Collezione", coll)?;
        let h = self.handle(id)?;
        let c = h.client.database(db).collection::<Document>(coll);
        Ok((h, c))
    }

    pub async fn find(&self, id: &str, db: &str, coll: &str, filter: &str, sort: &str, projection: &str) -> Result<QueryResult, String> {
        let filter = parse_document("Filtro non valido", filter, true)?;
        let sort = parse_document("Ordinamento non valido", sort, true)?;
        let projection = parse_document("Proiezione non valida", projection, true)?;
        let (h, c) = self.collection(id, db, coll)?;
        let mut action = c.find(filter).batch_size(PAGE as u32);
        if !sort.is_empty() {
            action = action.sort(sort);
        }
        if !projection.is_empty() {
            action = action.projection(projection);
        }
        let cursor = action.await.map_err(|e| map_error(&e, None))?;
        self.first_page(id, h.generation, cursor).await
    }

    pub async fn aggregate(&self, id: &str, db: &str, coll: &str, stages: Vec<Document>) -> Result<QueryResult, String> {
        let (h, c) = self.collection(id, db, coll)?;
        let cursor = c.aggregate(stages).batch_size(PAGE as u32).await.map_err(|e| map_error(&e, None))?;
        let cursor = cursor.with_type::<Document>();
        self.first_page(id, h.generation, cursor).await
    }

    async fn first_page(&self, id: &str, generation: u64, cursor: Cursor<Document>) -> Result<QueryResult, String> {
        self.drop_results_of(id);
        let mut open = Open { conn: id.to_string(), generation, cursor, total: 0, pending: false, done: false, docs: Vec::new() };
        let docs = read_page(&mut open).await?;
        let built = build(&docs);
        let mut notices = Vec::new();
        let has_more = more_left(&mut open, &mut notices);
        // il risultato resta tenuto (anche finito, con la copia dei documenti) finché `close` o una nuova ricerca lo sostituisce
        let rid = self.next_result.fetch_add(1, Ordering::Relaxed);
        open.docs = docs;
        lock(&self.results).insert(rid, Arc::new(AsyncMutex::new(open)));
        lock(&self.by_conn).insert(id.to_string(), rid);
        let result_id = Some(rid);
        Ok(QueryResult { result_id, columns: built.columns, rows: built.rows, has_more, truncated: built.truncated, notices, docs: built.docs, ..Default::default() })
    }

    pub async fn more(&self, result_id: u64) -> Result<QueryPage, String> {
        let open = lock(&self.results).get(&result_id).cloned().ok_or(EXPIRED)?;
        let mut open = open.lock().await;
        let alive = matches!(self.handle(&open.conn), Ok(h) if h.generation == open.generation);
        if !alive {
            drop(open);
            self.remove_result(result_id);
            return Err(EXPIRED.into());
        }
        let docs = match read_page(&mut open).await {
            Ok(d) => d,
            Err(e) => {
                drop(open);
                self.remove_result(result_id);
                return Err(e);
            }
        };
        let built = build(&docs);
        let mut notices = Vec::new();
        let has_more = more_left(&mut open, &mut notices);
        open.docs.extend(docs);
        drop(open);
        Ok(QueryPage { rows: built.rows, has_more, truncated: built.truncated, notices, docs: built.docs, columns: built.columns })
    }

    /// Per l'esportazione: tutti i documenti del risultato (quelli già consegnati, tenuti in copia, più i restanti, fino
    /// a `MAX_TOTAL`). Consuma il cursore: alla fine il risultato è finito ma resta tenuto (si può esportare di nuovo).
    pub async fn drain(&self, result_id: u64) -> Result<Vec<Document>, String> {
        let open = lock(&self.results).get(&result_id).cloned().ok_or(EXPIRED)?;
        let mut open = open.lock().await;
        let alive = matches!(self.handle(&open.conn), Ok(h) if h.generation == open.generation);
        if !alive {
            drop(open);
            self.remove_result(result_id);
            return Err(EXPIRED.into());
        }
        while more_left(&mut open, &mut Vec::new()) {
            match read_page(&mut open).await {
                Ok(d) => open.docs.extend(d),
                Err(e) => {
                    drop(open);
                    self.remove_result(result_id);
                    return Err(e);
                }
            }
        }
        Ok(open.docs.clone())
    }

    fn remove_result(&self, result_id: u64) {
        lock(&self.results).remove(&result_id);
        lock(&self.by_conn).retain(|_, v| *v != result_id);
    }

    /// Chiude un risultato (il cursore si libera). Già chiuso o scaduto: nulla.
    pub fn close(&self, result_id: u64) {
        self.remove_result(result_id);
    }

    // ---- modifiche ----

    pub async fn replace(&self, id: &str, db: &str, coll: &str, key: &Bson, doc: Document) -> Result<(), String> {
        let (_, c) = self.collection(id, db, coll)?;
        let r = c.replace_one(doc! { "_id": key.clone() }, doc).await.map_err(|e| map_error(&e, None))?;
        if r.matched_count != 1 {
            return Err("Nessun documento con questo _id: non modificato".into());
        }
        Ok(())
    }

    /// Inserisce e restituisce l'`_id` (JSON esteso rilassato).
    pub async fn insert(&self, id: &str, db: &str, coll: &str, doc: Document) -> Result<String, String> {
        let (_, c) = self.collection(id, db, coll)?;
        let r = c.insert_one(doc).await.map_err(|e| map_error(&e, None))?;
        Ok(cell_text(&r.inserted_id))
    }

    pub async fn delete(&self, id: &str, db: &str, coll: &str, key: &Bson) -> Result<(), String> {
        let (_, c) = self.collection(id, db, coll)?;
        let r = c.delete_one(doc! { "_id": key.clone() }).await.map_err(|e| map_error(&e, None))?;
        if r.deleted_count != 1 {
            return Err("Nessun documento con questo _id: non eliminato".into());
        }
        Ok(())
    }
}

async fn open_client(opts: ClientOptions, auth_db: &str, password: Option<&str>) -> Result<Client, String> {
    let run = async {
        let client = Client::with_options(opts).map_err(|e| map_error(&e, password))?;
        client.database(auth_db).run_command(doc! { "ping": 1 }).await.map_err(|e| map_error(&e, password))?;
        Ok(client)
    };
    tokio::time::timeout(CONNECT_TIMEOUT, run).await.unwrap_or_else(|_| Err(TIMED_OUT.into()))
}

/// Legge fino a `PAGE` documenti; poi guarda se ce n'è un altro (resta pronto nel cursore).
async fn read_page(o: &mut Open) -> Result<Vec<Document>, String> {
    let mut docs = Vec::new();
    while docs.len() < PAGE && !o.done {
        let have = if std::mem::take(&mut o.pending) { true } else { o.cursor.advance().await.map_err(|e| map_error(&e, None))? };
        if !have {
            o.done = true;
            break;
        }
        docs.push(o.cursor.deserialize_current().map_err(|e| map_error(&e, None))?);
    }
    o.total += docs.len();
    if !o.done {
        o.pending = o.cursor.advance().await.map_err(|e| map_error(&e, None))?;
        o.done = !o.pending;
    }
    Ok(docs)
}

/// Restano documenti da leggere? Oltre il tetto di `MAX_TOTAL` no, con un avviso.
fn more_left(o: &mut Open, notices: &mut Vec<String>) -> bool {
    if o.done {
        return false;
    }
    if o.total >= MAX_TOTAL {
        notices.push(format!("limite di {MAX_TOTAL} documenti raggiunto: gli altri non vengono letti"));
        return false;
    }
    true
}

/// Testo del dialogo di conferma per una modifica o un'eliminazione.
pub fn confirm_message(db: &str, coll: &str, key: &Bson, what: &str) -> String {
    let id: String = cell_text(key).chars().take(200).collect();
    format!("{what}\n\n{db}.{coll}\n_id: {id}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use mongodb::bson::oid::ObjectId;

    fn params(tls: DbTls) -> MongoParams {
        MongoParams { host: "db.example.test".into(), port: 27017, user: Some("app".into()), database: Some("appdb".into()), tls, accept_invalid_certs: false }
    }

    #[test]
    fn parses_filters_with_extended_json() {
        assert!(parse_document("Filtro non valido", "", true).unwrap().is_empty());
        assert!(parse_document("Filtro non valido", "  \n", true).unwrap().is_empty());
        let d = parse_document("Filtro non valido", r#"{"_id":{"$oid":"507f1f77bcf86cd799439011"},"n":{"$gt":5},"s":"x"}"#, true).unwrap();
        assert_eq!(d.get_object_id("_id").unwrap(), ObjectId::parse_str("507f1f77bcf86cd799439011").unwrap());
        assert_eq!(d.get_document("n").unwrap().get_i32("$gt").unwrap(), 5);
        let d = parse_document("Filtro non valido", r#"{"d":{"$date":"2020-01-02T03:04:05Z"},"big":{"$numberLong":"9007199254740993"},"x":{"$numberDecimal":"1.50"}}"#, true).unwrap();
        assert!(matches!(d.get("d"), Some(Bson::DateTime(_))));
        assert_eq!(d.get_i64("big").unwrap(), 9007199254740993);
        assert!(matches!(d.get("x"), Some(Bson::Decimal128(_))));
        // numeri: i32 se entrano, poi i64, poi f64
        let d = parse_document("F", r#"{"a":1,"b":5000000000,"c":1.5}"#, true).unwrap();
        assert!(matches!(d.get("a"), Some(Bson::Int32(1))));
        assert!(matches!(d.get("b"), Some(Bson::Int64(5_000_000_000))));
        assert!(matches!(d.get("c"), Some(Bson::Double(_))));
    }

    #[test]
    fn key_order_is_kept() {
        let d = parse_document("Ordinamento non valido", r#"{"zeta":-1,"alfa":1,"meta":1}"#, true).unwrap();
        assert_eq!(d.keys().collect::<Vec<_>>(), ["zeta", "alfa", "meta"]);
        assert_eq!(doc_json(&d, false), r#"{"zeta":-1,"alfa":1,"meta":1}"#);
    }

    #[test]
    fn invalid_inputs_have_readable_errors() {
        let e = parse_document("Filtro non valido", "{", true).unwrap_err();
        assert!(e.starts_with("Filtro non valido: "), "{e}");
        let e = parse_document("Filtro non valido", "[1]", true).unwrap_err();
        assert_eq!(e, "Filtro non valido: deve essere un oggetto JSON");
        let e = parse_document("Filtro non valido", r#"{"_id":{"$oid":"zz"}}"#, true).unwrap_err();
        assert!(e.starts_with("Filtro non valido: "), "{e}");
        assert!(parse_document("Documento non valido", "", false).is_err());
        let e = parse_pipeline("{}").unwrap_err();
        assert_eq!(e, "Pipeline non valida: deve essere un array di stadi");
        let e = parse_pipeline("[1]").unwrap_err();
        assert_eq!(e, "Pipeline non valida: lo stadio 1 deve essere un oggetto JSON");
        assert!(parse_pipeline("").unwrap_err().starts_with("Pipeline non valida: "));
        assert!(parse_value("_id non valido", "").is_err());
        assert!(parse_value("_id", r#"{"$oid":"507f1f77bcf86cd799439011"}"#).unwrap().as_object_id().is_some());
        assert_eq!(parse_value("_id", "\"abc\"").unwrap(), Bson::String("abc".into()));
        assert_eq!(parse_value("_id", "7").unwrap(), Bson::Int32(7));
    }

    #[test]
    fn pipeline_write_stages_are_detected() {
        let p = parse_pipeline(r#"[{"$match":{"a":1}},{"$group":{"_id":"$a"}}]"#).unwrap();
        assert_eq!(p.len(), 2);
        assert!(!pipeline_writes(&p));
        assert!(pipeline_writes(&parse_pipeline(r#"[{"$match":{}},{"$out":"x"}]"#).unwrap()));
        assert!(pipeline_writes(&parse_pipeline(r#"[{"$merge":{"into":"x"}}]"#).unwrap()));
        // un `$out` nel valore di un altro campo non è uno stadio
        assert!(!pipeline_writes(&parse_pipeline(r#"[{"$match":{"note":"$out"}}]"#).unwrap()));
        assert!(!pipeline_writes(&parse_pipeline("[]").unwrap()));
    }

    #[test]
    fn bson_to_relaxed_json() {
        let oid = ObjectId::parse_str("507f1f77bcf86cd799439011").unwrap();
        let d = parse_document(
            "F",
            r#"{"_id":{"$oid":"507f1f77bcf86cd799439011"},"when":{"$date":"2020-01-02T03:04:05Z"},"dec":{"$numberDecimal":"1.50"},
                "bin":{"$binary":{"base64":"AQID","subType":"00"}},"n":3,"nest":{"a":[1,{"b":null}],"e":{}},"arr":[]}"#,
            true,
        )
        .unwrap();
        let j = doc_json(&d, false);
        assert!(j.starts_with(&format!(r#"{{"_id":{{"$oid":"{}"}}"#, oid.to_hex())), "{j}");
        assert!(j.contains(r#""when":{"$date":"2020-01-02T03:04:05Z"}"#), "{j}");
        assert!(j.contains(r#""dec":{"$numberDecimal":"1.50"}"#), "{j}");
        assert!(j.contains(r#""bin":{"$binary":{"base64":"AQID","subType":"00"}}"#), "{j}");
        assert!(j.contains(r#""nest":{"a":[1,{"b":null}],"e":{}},"arr":[]"#), "{j}");
        // il testo prodotto si rilegge nello stesso documento
        assert_eq!(parse_document("F", &j, true).unwrap(), d);
        assert_eq!(parse_document("F", &doc_json(&d, true), true).unwrap(), d);
        assert!(doc_json(&d, true).contains("\n  \"n\": 3"));
    }

    #[test]
    fn cell_rendering() {
        assert_eq!(cell_text(&Bson::String("ciao \"x\"".into())), "ciao \"x\"");
        assert_eq!(cell_text(&Bson::Int32(7)), "7");
        assert_eq!(cell_text(&Bson::Null), "null");
        assert_eq!(cell_text(&Bson::Boolean(true)), "true");
        let oid = ObjectId::parse_str("507f1f77bcf86cd799439011").unwrap();
        assert_eq!(cell_text(&Bson::ObjectId(oid)), r#"{"$oid":"507f1f77bcf86cd799439011"}"#);
        let v = parse_value("x", r#"{"a":[1,"b"],"d":{"$date":"2020-01-02T03:04:05Z"}}"#).unwrap();
        assert_eq!(cell_text(&v), r#"{"a":[1,"b"],"d":{"$date":"2020-01-02T03:04:05Z"}}"#);
    }

    #[test]
    fn columns_and_rows_of_a_page() {
        let docs = vec![
            parse_document("F", r#"{"a":1,"_id":1,"nome":"x"}"#, true).unwrap(),
            parse_document("F", r#"{"_id":2,"b":{"c":1},"a":null}"#, true).unwrap(),
        ];
        assert_eq!(columns_of(&docs), ["_id", "a", "nome", "b"]);
        let b = build(&docs);
        assert_eq!(b.columns.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["_id", "a", "nome", "b"]);
        assert_eq!(b.rows[0], [Some("1".into()), Some("1".into()), Some("x".into()), None]);
        assert_eq!(b.rows[1], [Some("2".into()), Some("null".into()), None, Some(r#"{"c":1}"#.into())]);
        assert_eq!(b.docs.len(), 2);
        assert!(b.truncated.is_empty());
        assert!(columns_of(&[]).is_empty());
    }

    #[test]
    fn long_cells_are_truncated_and_reported() {
        let big = "é".repeat(40_000);
        let mut d = Document::new();
        d.insert("_id", 1);
        d.insert("big", big.clone());
        let b = build(&[d]);
        assert_eq!(b.truncated, vec![[0, 1]]);
        let cell = b.rows[0][1].as_ref().unwrap();
        assert!(cell.len() <= super::super::pg_query::MAX_CELL && cell.chars().all(|c| c == 'é'));
        // il documento intero resta disponibile per l'editor
        assert!(b.docs[0].contains(&big));
    }

    #[test]
    fn options_per_tls_mode_and_tunnel() {
        let tls_of = |o: &ClientOptions| matches!(o.tls, Some(Tls::Enabled(_)));
        let o = build_options(&params(DbTls::Prefer), Some("s3kr3t-pw"), None).unwrap();
        assert!(tls_of(&o));
        assert_eq!(o.hosts, vec![ServerAddress::Tcp { host: "db.example.test".into(), port: Some(27017) }]);
        assert_eq!(o.direct_connection, Some(false));
        assert_eq!(o.app_name.as_deref(), Some("Code Art"));
        assert_eq!(o.connect_timeout, Some(CONNECT_TIMEOUT));
        assert_eq!(o.server_selection_timeout, Some(CONNECT_TIMEOUT));
        let cred = o.credential.as_ref().unwrap();
        assert_eq!((cred.username.as_deref(), cred.source.as_deref(), cred.password.as_deref()), (Some("app"), Some("appdb"), Some("s3kr3t-pw")));
        assert!(!format!("{o:?}").contains("s3kr3t-pw"));
        // modi
        assert!(tls_of(&build_options(&params(DbTls::Require), Some("p"), None).unwrap()));
        assert!(!tls_of(&build_options(&params(DbTls::Disable), Some("p"), None).unwrap()));
        let mut local = params(DbTls::Prefer);
        local.host = "localhost".into();
        assert!(!tls_of(&build_options(&local, Some("p"), None).unwrap()));
        local.tls = DbTls::Require;
        assert!(tls_of(&build_options(&local, Some("p"), None).unwrap()));
        // certificato non valido solo se esplicito
        let mut bad = params(DbTls::Require);
        let Some(Tls::Enabled(t)) = build_options(&bad, Some("p"), None).unwrap().tls else { panic!() };
        assert_ne!(t.allow_invalid_certificates, Some(true));
        bad.accept_invalid_certs = true;
        let Some(Tls::Enabled(t)) = build_options(&bad, Some("p"), None).unwrap().tls else { panic!() };
        assert_eq!(t.allow_invalid_certificates, Some(true));
        // tunnel: 127.0.0.1 sulla porta locale, connessione diretta, prefer senza TLS
        let o = build_options(&params(DbTls::Prefer), Some("p"), Some(40123)).unwrap();
        assert_eq!(o.hosts, vec![ServerAddress::Tcp { host: "127.0.0.1".into(), port: Some(40123) }]);
        assert_eq!(o.direct_connection, Some(true));
        assert!(!tls_of(&o));
        assert!(tls_of(&build_options(&params(DbTls::Require), Some("p"), Some(40123)).unwrap()));
    }

    #[test]
    fn credential_rules() {
        // senza utente: nessuna credenziale; senza database: authSource admin
        let mut p = params(DbTls::Disable);
        p.user = None;
        assert!(build_options(&p, None, None).unwrap().credential.is_none());
        p.user = Some("u".into());
        p.database = None;
        let o = build_options(&p, Some("pw"), None).unwrap();
        assert_eq!(o.credential.unwrap().source.as_deref(), Some("admin"));
        // utente senza password salvata: serve la password
        assert_eq!(build_options(&p, None, None).unwrap_err(), AUTH);
    }

    #[test]
    fn errors_are_mapped_and_sanitized() {
        // i tipi d'errore del driver non si costruiscono da fuori: `Custom` passa dal testo, ripulito dalla password
        let custom = MongoError::custom("boom con hunter2 dentro");
        assert!(map_error(&custom, Some("hunter2")).starts_with("Kind: Custom"));
        assert_eq!(classify(true, false, None, "", "SCRAM failure", Some("pw")), AUTH);
        assert_eq!(classify(false, false, Some(18), "AuthenticationFailed", "Authentication failed.", None), AUTH);
        assert_eq!(classify(false, true, None, "", "Kind: Authentication failed, labels: {}", None), AUTH);
        assert_eq!(classify(false, false, Some(13), "Unauthorized", "not authorized on admin", None), "Non autorizzato: not authorized on admin");
        assert_eq!(classify(false, true, None, "", "No available servers", None), "Server non raggiungibile: No available servers");
        let tls = classify(false, true, None, "", "Servers: [ Error: invalid peer certificate: UnknownIssuer ]", None);
        assert!(tls.starts_with("Errore TLS: "), "{tls}");
        let leak = classify(false, true, None, "", "failed with password=hunter2 and again hunter2", Some("hunter2"));
        assert!(!leak.contains("hunter2"), "{leak}");
        let cmd = classify(false, false, Some(2), "BadValue", "bad", None);
        assert_eq!(cmd, "Errore del server: bad (codice 2)");
        assert_eq!(classify(false, false, None, "", "qualcosa", None), "qualcosa");
    }

    #[test]
    fn confirm_text_names_collection_and_id() {
        let m = confirm_message("app", "utenti", &Bson::ObjectId(ObjectId::parse_str("507f1f77bcf86cd799439011").unwrap()), "Elimina");
        assert!(m.contains("app.utenti") && m.contains("507f1f77bcf86cd799439011"), "{m}");
        let long = confirm_message("d", "c", &Bson::String("x".repeat(500)), "Elimina");
        assert!(long.len() < 300);
    }

    #[test]
    fn names_and_result_ids() {
        assert!(check_name("Database", "").is_err());
        assert!(check_name("Database", "a\nb").is_err());
        assert!(check_name("Database", "app").is_ok());
        assert!(is_mongo_result(ID_BASE) && !is_mongo_result(5));
    }
}
