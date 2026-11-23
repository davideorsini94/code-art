//! Connessioni ai database (Postgres, MongoDB): elenco delle connessioni salvate nel database dell'app, le loro
//! password (solo nel Portachiavi, servizio `Code Art DB`) e la cronologia delle query. Niente rete qui: le sessioni
//! stanno nei moduli dei driver.
pub mod pg;
pub mod pg_edit;
pub mod pg_query;
#[cfg(test)]
pub mod testutil;
pub mod tunnel;

use crate::ssh::{self, keychain, Secrets};
use std::sync::Arc;
use crate::store::Store;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, WebviewWindow};

/// Voci di cronologia tenute per connessione.
pub const HISTORY_KEEP: usize = 200;
/// Dimensione massima del testo di una voce di cronologia.
pub const MAX_HISTORY_BYTES: usize = 100 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DbKind {
    Postgres,
    Mongo,
}

impl DbKind {
    pub fn as_str(self) -> &'static str {
        match self {
            DbKind::Postgres => "postgres",
            DbKind::Mongo => "mongo",
        }
    }
    pub fn parse(s: &str) -> Option<DbKind> {
        match s {
            "postgres" => Some(DbKind::Postgres),
            "mongo" => Some(DbKind::Mongo),
            _ => None,
        }
    }
    pub fn default_port(self) -> u16 {
        match self {
            DbKind::Postgres => 5432,
            DbKind::Mongo => 27017,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DbTls {
    Prefer,
    Require,
    Disable,
}

impl DbTls {
    pub fn as_str(self) -> &'static str {
        match self {
            DbTls::Prefer => "prefer",
            DbTls::Require => "require",
            DbTls::Disable => "disable",
        }
    }
    pub fn parse(s: &str) -> Option<DbTls> {
        match s {
            "prefer" => Some(DbTls::Prefer),
            "require" => Some(DbTls::Require),
            "disable" => Some(DbTls::Disable),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DbConnection {
    pub id: String,
    pub kind: DbKind,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub user: Option<String>,
    pub database: Option<String>,
    pub tls: DbTls,
    pub accept_invalid_certs: bool,
    /// Server SSH (salvato o alias di `~/.ssh/config`) da cui passa il tunnel.
    pub ssh_server_id: Option<String>,
    pub has_password: bool,
}

/// Campi modificabili; `id` assente = nuova connessione, `port` assente = quella predefinita del tipo.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DbConnectionInput {
    pub id: Option<String>,
    pub kind: DbKind,
    pub name: String,
    pub host: String,
    pub port: Option<u16>,
    pub user: Option<String>,
    pub database: Option<String>,
    pub tls: DbTls,
    #[serde(default)]
    pub accept_invalid_certs: bool,
    pub ssh_server_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntry {
    pub id: i64,
    pub text: String,
    pub at_ms: i64,
}

/// Il Portachiavi di macOS (servizio `Code Art DB`, account = id della connessione).
pub struct DbKeychain;

impl Secrets for DbKeychain {
    fn has(&self, id: &str) -> Result<bool, String> {
        keychain::exists(keychain::DB_SERVICE, id)
    }
    fn set(&self, id: &str, secret: &str) -> Result<(), String> {
        keychain::set(keychain::DB_SERVICE, id, secret)
    }
    fn clear(&self, id: &str) -> Result<(), String> {
        keychain::delete(keychain::DB_SERVICE, id)
    }
}

fn validate_text(what: &str, v: &str) -> Result<(), String> {
    if v.chars().any(|c| c.is_control()) {
        return Err(format!("{what}: caratteri di controllo non ammessi"));
    }
    Ok(())
}

fn non_empty(v: &Option<String>) -> Option<String> {
    v.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string)
}

/// Regole di campo; `sshServerId` si controlla a parte (serve l'elenco dei server).
pub fn validate_connection(c: &DbConnectionInput) -> Result<(), String> {
    let name = c.name.trim();
    if name.is_empty() {
        return Err("Nome: campo obbligatorio".into());
    }
    if name.chars().count() > 100 {
        return Err("Nome: al massimo 100 caratteri".into());
    }
    validate_text("Nome", name)?;
    ssh::validate_host("Host", &c.host)?;
    if c.port == Some(0) {
        return Err("Porta: da 1 a 65535".into());
    }
    if let Some(u) = &c.user {
        validate_text("Utente", u)?;
    }
    if let Some(d) = &c.database {
        validate_text("Database", d)?;
    }
    Ok(())
}

/// Crea o aggiorna una connessione. `ssh_known` dice se un id di server SSH esiste (salvato o alias di config).
pub fn save_connection(
    store: &Store,
    secrets: &dyn Secrets,
    input: &DbConnectionInput,
    ssh_known: &dyn Fn(&str) -> Result<bool, String>,
) -> Result<DbConnection, String> {
    validate_connection(input)?;
    let ssh_server_id = non_empty(&input.ssh_server_id);
    if let Some(s) = &ssh_server_id
        && !ssh_known(s)?
    {
        return Err("Server SSH non trovato".into());
    }
    let id = match input.id.as_deref() {
        Some(id) => {
            if !store.db_connection_exists(id)? {
                return Err("Connessione non trovata".into());
            }
            id.to_string()
        }
        None => uuid::Uuid::new_v4().to_string(),
    };
    let has_password = input.id.is_some() && secrets.has(&id).unwrap_or(false);
    let conn = DbConnection {
        id,
        kind: input.kind,
        name: input.name.trim().to_string(),
        host: input.host.clone(),
        port: input.port.unwrap_or_else(|| input.kind.default_port()),
        user: non_empty(&input.user),
        database: non_empty(&input.database),
        tls: input.tls,
        accept_invalid_certs: input.accept_invalid_certs,
        ssh_server_id,
        has_password,
    };
    store.db_connection_upsert(&conn)?;
    Ok(conn)
}

/// Salvate (per nome); `has_password` dal Portachiavi (non accessibile = nessuna password).
pub fn list_connections(store: &Store, secrets: &dyn Secrets) -> Result<Vec<DbConnection>, String> {
    let mut out = store.db_connections()?;
    for c in &mut out {
        c.has_password = secrets.has(&c.id).unwrap_or(false);
    }
    Ok(out)
}

/// Elimina la connessione, la sua cronologia e la password: prima il Portachiavi, così non restano segreti senza
/// connessione.
pub fn delete_connection(store: &Store, secrets: &dyn Secrets, id: &str) -> Result<(), String> {
    secrets.clear(id)?;
    store.db_connection_delete(id)
}

fn ensure_known(store: &Store, id: &str) -> Result<(), String> {
    if store.db_connection_exists(id)? { Ok(()) } else { Err("Connessione non trovata".into()) }
}

pub fn set_password(store: &Store, secrets: &dyn Secrets, id: &str, password: &str) -> Result<(), String> {
    ensure_known(store, id)?;
    ssh::validate_password(password)?;
    secrets.set(id, password)
}

pub fn clear_password(store: &Store, secrets: &dyn Secrets, id: &str) -> Result<(), String> {
    ensure_known(store, id)?;
    secrets.clear(id)
}

pub fn history_add(store: &Store, id: &str, text: &str, at_ms: i64) -> Result<(), String> {
    ensure_known(store, id)?;
    if text.trim().is_empty() {
        return Err("Testo: campo obbligatorio".into());
    }
    if text.len() > MAX_HISTORY_BYTES {
        return Err("Testo: al massimo 100 KB".into());
    }
    store.db_history_add(id, text, at_ms)
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[tauri::command]
pub async fn db_connections(app: AppHandle, window: WebviewWindow) -> Result<Vec<DbConnection>, String> {
    let store = ssh::trusted_store(&app, &window)?;
    crate::projects::off_main(move || list_connections(&store, &DbKeychain)).await
}

#[tauri::command]
pub async fn db_connection_save(app: AppHandle, window: WebviewWindow, input: DbConnectionInput) -> Result<DbConnection, String> {
    let store = ssh::trusted_store(&app, &window)?;
    crate::projects::off_main(move || {
        let known = |id: &str| Ok(ssh::find_server(&store, ssh::read_user_config().as_deref(), id).is_ok());
        save_connection(&store, &DbKeychain, &input, &known)
    })
    .await
}

#[tauri::command]
pub async fn db_connection_delete(app: AppHandle, window: WebviewWindow, id: String) -> Result<(), String> {
    let store = ssh::trusted_store(&app, &window)?;
    crate::projects::off_main(move || delete_connection(&store, &DbKeychain, &id)).await
}

/// Salva la password nel Portachiavi; non torna mai alla WebView.
#[tauri::command]
pub async fn db_password_set(app: AppHandle, window: WebviewWindow, id: String, password: String) -> Result<(), String> {
    let store = ssh::trusted_store(&app, &window)?;
    crate::projects::off_main(move || set_password(&store, &DbKeychain, &id, &password)).await
}

#[tauri::command]
pub async fn db_password_clear(app: AppHandle, window: WebviewWindow, id: String) -> Result<(), String> {
    let store = ssh::trusted_store(&app, &window)?;
    crate::projects::off_main(move || clear_password(&store, &DbKeychain, &id)).await
}

#[tauri::command]
pub async fn db_history(app: AppHandle, window: WebviewWindow, id: String) -> Result<Vec<HistoryEntry>, String> {
    let store = ssh::trusted_store(&app, &window)?;
    crate::projects::off_main(move || {
        ensure_known(&store, &id)?;
        store.db_history(&id)
    })
    .await
}

#[tauri::command]
pub async fn db_history_add(app: AppHandle, window: WebviewWindow, id: String, text: String) -> Result<(), String> {
    let store = ssh::trusted_store(&app, &window)?;
    crate::projects::off_main(move || history_add(&store, &id, &text, now_ms())).await
}

/// Connette (sessione condivisa da tutte le finestre; lo stato arriva con `db:state`). Già connessa o in
/// connessione: `Ok` subito. `Err("auth")` = serve la password. Con un server SSH, deve essere già connesso.
#[tauri::command]
pub async fn db_connect(app: AppHandle, window: WebviewWindow, id: String) -> Result<(), String> {
    let store = ssh::trusted_store(&app, &window)?;
    let sessions = app.state::<Arc<pg::Sessions>>().inner().clone();
    let (conn, password, server) = crate::projects::off_main(move || {
        let conn = store.db_connections()?.into_iter().find(|c| c.id == id).ok_or_else(|| "Connessione non trovata".to_string())?;
        let server = match &conn.ssh_server_id {
            Some(s) => Some(ssh::find_server(&store, ssh::read_user_config().as_deref(), s)?),
            None => None,
        };
        // senza password salvata si prova comunque (autenticazione `trust`); se serve, il server risponde `auth`
        let password = keychain::get(keychain::DB_SERVICE, &conn.id).unwrap_or(None);
        Ok((conn, password, server))
    })
    .await?;
    match conn.kind {
        DbKind::Postgres => sessions.connect(&conn.id, pg::PgParams::from_connection(&conn), password, server).await,
        // Task 5: MongoDB
        DbKind::Mongo => Err("tipo di connessione non ancora supportato".into()),
    }
}

#[tauri::command]
pub async fn db_disconnect(app: AppHandle, window: WebviewWindow, id: String) -> Result<(), String> {
    ssh::trusted_store(&app, &window)?;
    let sessions = app.state::<Arc<pg::Sessions>>().inner().clone();
    crate::projects::off_main(move || {
        sessions.disconnect(&id);
        Ok(())
    })
    .await
}

/// Stato delle sessioni note in questo avvio (le altre sono `disconnected`).
#[tauri::command]
pub async fn db_states(app: AppHandle, window: WebviewWindow) -> Result<Vec<ssh::conn::StateEvent>, String> {
    ssh::trusted_store(&app, &window)?;
    Ok(app.state::<Arc<pg::Sessions>>().states())
}

/// Schemi, tabelle e viste della connessione (esclusi gli schemi di sistema).
#[tauri::command]
pub async fn db_pg_tree(app: AppHandle, window: WebviewWindow, id: String) -> Result<pg_query::PgTree, String> {
    ssh::trusted_store(&app, &window)?;
    let sessions = app.state::<Arc<pg::Sessions>>().inner().clone();
    pg_query::tree(&sessions, &id).await
}

#[tauri::command]
pub async fn db_pg_columns(app: AppHandle, window: WebviewWindow, id: String, schema: String, table: String) -> Result<Vec<pg_query::PgColumn>, String> {
    ssh::trusted_store(&app, &window)?;
    let sessions = app.state::<Arc<pg::Sessions>>().inner().clone();
    pg_query::columns(&sessions, &id, &schema, &table).await
}

/// Anteprima leggibile delle istruzioni che le modifiche produrrebbero (valori inseriti solo per la lettura, mai
/// eseguiti). Le colonne vengono dal catalogo del server; nomi sconosciuti o tabella senza chiave primaria = errore.
#[tauri::command]
pub async fn db_pg_preview_edits(app: AppHandle, window: WebviewWindow, id: String, schema: String, table: String, edits: Vec<pg_edit::RowEdit>) -> Result<Vec<String>, String> {
    ssh::trusted_store(&app, &window)?;
    let sessions = app.state::<Arc<pg::Sessions>>().inner().clone();
    Ok(pg_edit::previews(&pg_edit::plan(&sessions, &id, &schema, &table, &edits).await?))
}

/// Applica le modifiche: conferma nativa del core («Applica modifiche», «Annulla» predefinito) con il riepilogo, poi
/// una sola transazione; ogni istruzione deve toccare esattamente una riga, altrimenti `ROLLBACK`.
#[tauri::command]
pub async fn db_pg_apply_edits(app: AppHandle, window: WebviewWindow, id: String, schema: String, table: String, edits: Vec<pg_edit::RowEdit>) -> Result<pg_edit::ApplyResult, String> {
    ssh::trusted_store(&app, &window)?;
    let sessions = app.state::<Arc<pg::Sessions>>().inner().clone();
    let stmts = pg_edit::plan(&sessions, &id, &schema, &table, &edits).await?;
    let message = pg_edit::confirm_message(&stmts);
    let confirm = crate::git::write::NativeConfirm { app: app.clone(), window: window.clone() };
    let ok = crate::projects::off_main(move || {
        use crate::git::write::Confirm;
        Ok(confirm.confirm(pg_edit::APPLY_TITLE, &message, pg_edit::BTN_APPLY))
    })
    .await?;
    if !ok {
        return Err(pg_query::DECLINED.into());
    }
    pg_edit::execute(app.state::<Arc<pg_query::Queries>>().inner(), &sessions, &id, &stmts).await
}

/// Esegue un testo SQL. `UPDATE`/`DELETE` senza `WHERE`, `DROP` e `TRUNCATE` (controllo lessicale di aiuto) chiedono
/// prima una conferma nativa del core (titolo fisso, «Annulla» predefinito): rifiutata = `Err("annullato")`, nulla eseguito.
#[tauri::command]
pub async fn db_query(app: AppHandle, window: WebviewWindow, id: String, text: String) -> Result<pg_query::QueryResult, String> {
    ssh::trusted_store(&app, &window)?;
    let flags = pg_query::destructive(&text);
    if !flags.is_empty() {
        let message = pg_query::confirm_message(&flags);
        let confirm = crate::git::write::NativeConfirm { app: app.clone(), window: window.clone() };
        let ok = crate::projects::off_main(move || {
            use crate::git::write::Confirm;
            Ok(confirm.confirm(pg_query::DESTRUCTIVE_TITLE, &message, pg_query::BTN_RUN))
        })
        .await?;
        if !ok {
            return Err(pg_query::DECLINED.into());
        }
    }
    let sessions = app.state::<Arc<pg::Sessions>>().inner().clone();
    app.state::<Arc<pg_query::Queries>>().inner().clone().query(&sessions, &id, &text).await
}

/// Pagina successiva di un risultato con cursore aperto.
#[tauri::command]
pub async fn db_query_more(app: AppHandle, window: WebviewWindow, result_id: u64) -> Result<pg_query::QueryPage, String> {
    ssh::trusted_store(&app, &window)?;
    let sessions = app.state::<Arc<pg::Sessions>>().inner().clone();
    app.state::<Arc<pg_query::Queries>>().inner().clone().more(&sessions, result_id).await
}

#[tauri::command]
pub async fn db_query_close(app: AppHandle, window: WebviewWindow, result_id: u64) -> Result<(), String> {
    ssh::trusted_store(&app, &window)?;
    let sessions = app.state::<Arc<pg::Sessions>>().inner().clone();
    app.state::<Arc<pg_query::Queries>>().inner().clone().close(&sessions, result_id).await
}

/// Annulla la query in corso sulla connessione (`CancelToken`, stesso TLS).
#[tauri::command]
pub async fn db_query_cancel(app: AppHandle, window: WebviewWindow, id: String) -> Result<(), String> {
    ssh::trusted_store(&app, &window)?;
    let sessions = app.state::<Arc<pg::Sessions>>().inner().clone();
    pg_query::cancel(&sessions, &id).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;

    #[derive(Default)]
    struct Fake(RefCell<HashMap<String, String>>);
    impl Secrets for Fake {
        fn has(&self, id: &str) -> Result<bool, String> {
            Ok(self.0.borrow().contains_key(id))
        }
        fn set(&self, id: &str, secret: &str) -> Result<(), String> {
            self.0.borrow_mut().insert(id.into(), secret.into());
            Ok(())
        }
        fn clear(&self, id: &str) -> Result<(), String> {
            self.0.borrow_mut().remove(id);
            Ok(())
        }
    }

    fn store() -> (tempfile::TempDir, Store) {
        let d = tempfile::tempdir().unwrap();
        let s = Store::open(&d.path().join("db.sqlite")).unwrap();
        (d, s)
    }

    fn input(name: &str, host: &str) -> DbConnectionInput {
        DbConnectionInput {
            id: None,
            kind: DbKind::Postgres,
            name: name.into(),
            host: host.into(),
            port: None,
            user: None,
            database: None,
            tls: DbTls::Prefer,
            accept_invalid_certs: false,
            ssh_server_id: None,
        }
    }

    fn no_ssh(_: &str) -> Result<bool, String> {
        Ok(false)
    }

    #[test]
    fn validation_rules() {
        assert!(validate_connection(&input("Dev", "example.test")).is_ok());
        assert!(validate_connection(&input("", "example.test")).is_err());
        assert!(validate_connection(&input("  ", "example.test")).is_err());
        assert!(validate_connection(&input(&"a".repeat(100), "example.test")).is_ok());
        assert!(validate_connection(&input(&"a".repeat(101), "example.test")).is_err());
        assert!(validate_connection(&input("a\nb", "example.test")).is_err());
        for bad in ["", "-h", "a b", "a,b", "a/b", "u@h", "a\nb", "*", "bücher.test"] {
            assert!(validate_connection(&input("Dev", bad)).is_err(), "host {bad:?}");
        }
        for ok in ["example.test", "192.0.2.1", "fe80::1", "db_1-x"] {
            assert!(validate_connection(&input("Dev", ok)).is_ok(), "host {ok:?}");
        }
        let mut c = input("Dev", "example.test");
        c.port = Some(0);
        assert!(validate_connection(&c).is_err());
        c.port = Some(1);
        assert!(validate_connection(&c).is_ok());
        c.port = Some(65535);
        assert!(validate_connection(&c).is_ok());
        c.user = Some("a\u{0}".into());
        assert!(validate_connection(&c).is_err());
        c.user = Some("app".into());
        c.database = Some("d\tb".into());
        assert!(validate_connection(&c).is_err());
        c.database = Some("app".into());
        assert!(validate_connection(&c).is_ok());
    }

    #[test]
    fn save_defaults_ports_and_round_trips() {
        let (_d, store) = store();
        let sec = Fake::default();
        let pg = save_connection(&store, &sec, &input("  Pg  ", "example.test"), &no_ssh).unwrap();
        assert_eq!((pg.name.as_str(), pg.port, pg.kind), ("Pg", 5432, DbKind::Postgres));
        let mut m = input("Mongo", "example.test");
        m.kind = DbKind::Mongo;
        m.user = Some("  ".into());
        m.tls = DbTls::Require;
        m.accept_invalid_certs = true;
        let mongo = save_connection(&store, &sec, &m, &no_ssh).unwrap();
        assert_eq!((mongo.port, mongo.user.clone()), (27017, None));
        let list = list_connections(&store, &sec).unwrap();
        assert_eq!(list.len(), 2);
        let got = list.iter().find(|c| c.id == mongo.id).unwrap();
        assert_eq!(got, &mongo);
        assert!(got.accept_invalid_certs && got.tls == DbTls::Require && got.kind == DbKind::Mongo);
        // modifica di una esistente
        let mut edit = input("Pg2", "other.test");
        edit.id = Some(pg.id.clone());
        edit.port = Some(6543);
        let upd = save_connection(&store, &sec, &edit, &no_ssh).unwrap();
        assert_eq!((upd.id.as_str(), upd.port), (pg.id.as_str(), 6543));
        assert_eq!(list_connections(&store, &sec).unwrap().len(), 2);
        // id sconosciuto e input non valido: nulla di scritto
        edit.id = Some("nope".into());
        assert_eq!(save_connection(&store, &sec, &edit, &no_ssh).unwrap_err(), "Connessione non trovata");
        assert!(save_connection(&store, &sec, &input("", "x.test"), &no_ssh).is_err());
        assert_eq!(list_connections(&store, &sec).unwrap().len(), 2);
    }

    #[test]
    fn ssh_server_must_exist() {
        let (_d, store) = store();
        let sec = Fake::default();
        let mut c = input("Dev", "example.test");
        c.ssh_server_id = Some("config:bastion".into());
        let known = |id: &str| Ok(id == "config:bastion");
        let saved = save_connection(&store, &sec, &c, &known).unwrap();
        assert_eq!(saved.ssh_server_id.as_deref(), Some("config:bastion"));
        c.ssh_server_id = Some("manca".into());
        assert_eq!(save_connection(&store, &sec, &c, &known).unwrap_err(), "Server SSH non trovato");
        // vuoto = nessun tunnel
        c.ssh_server_id = Some("".into());
        assert_eq!(save_connection(&store, &sec, &c, &known).unwrap().ssh_server_id, None);
    }

    #[test]
    fn password_lives_only_in_secrets() {
        let (_d, store) = store();
        let sec = Fake::default();
        let c = save_connection(&store, &sec, &input("Dev", "example.test"), &no_ssh).unwrap();
        assert!(!c.has_password);
        assert!(set_password(&store, &sec, "nope", "pw").is_err());
        assert!(set_password(&store, &sec, &c.id, "").is_err());
        assert!(set_password(&store, &sec, &c.id, "a\nb").is_err());
        set_password(&store, &sec, &c.id, "s3gr3to").unwrap();
        assert!(list_connections(&store, &sec).unwrap()[0].has_password);
        // salvare di nuovo la connessione non perde la password
        let mut edit = input("Dev2", "example.test");
        edit.id = Some(c.id.clone());
        assert!(save_connection(&store, &sec, &edit, &no_ssh).unwrap().has_password);
        clear_password(&store, &sec, &c.id).unwrap();
        assert!(!list_connections(&store, &sec).unwrap()[0].has_password);
        assert!(clear_password(&store, &sec, "nope").is_err());
    }

    #[test]
    fn delete_removes_password_and_history() {
        let (_d, store) = store();
        let sec = Fake::default();
        let a = save_connection(&store, &sec, &input("A", "a.test"), &no_ssh).unwrap();
        let b = save_connection(&store, &sec, &input("B", "b.test"), &no_ssh).unwrap();
        for c in [&a, &b] {
            set_password(&store, &sec, &c.id, "pw").unwrap();
            history_add(&store, &c.id, "select 1", 1).unwrap();
        }
        delete_connection(&store, &sec, &a.id).unwrap();
        assert!(!sec.has(&a.id).unwrap() && sec.has(&b.id).unwrap());
        assert!(store.db_history(&a.id).unwrap().is_empty());
        assert_eq!(store.db_history(&b.id).unwrap().len(), 1);
        assert_eq!(list_connections(&store, &sec).unwrap().len(), 1);
        // già assente: nessun errore
        delete_connection(&store, &sec, &a.id).unwrap();
    }

    #[test]
    fn history_dedups_consecutive_only_and_keeps_200() {
        let (_d, store) = store();
        let sec = Fake::default();
        let c = save_connection(&store, &sec, &input("A", "a.test"), &no_ssh).unwrap();
        let other = save_connection(&store, &sec, &input("B", "b.test"), &no_ssh).unwrap();
        history_add(&store, &c.id, "q1", 1).unwrap();
        history_add(&store, &c.id, "q1", 2).unwrap();
        history_add(&store, &c.id, "q2", 3).unwrap();
        history_add(&store, &c.id, "q1", 4).unwrap();
        history_add(&store, &other.id, "q1", 5).unwrap();
        let h = store.db_history(&c.id).unwrap();
        assert_eq!(h.iter().map(|e| e.text.as_str()).collect::<Vec<_>>(), ["q1", "q2", "q1"]);
        assert_eq!(h[0].at_ms, 4);
        for i in 0..250 {
            history_add(&store, &c.id, &format!("n{i}"), 10 + i).unwrap();
        }
        let h = store.db_history(&c.id).unwrap();
        assert_eq!(h.len(), HISTORY_KEEP);
        assert_eq!(h[0].text, "n249");
        assert_eq!(h[HISTORY_KEEP - 1].text, "n50");
        // l'altra connessione non è toccata
        assert_eq!(store.db_history(&other.id).unwrap().len(), 1);
    }

    #[test]
    fn history_limits() {
        let (_d, store) = store();
        let sec = Fake::default();
        let c = save_connection(&store, &sec, &input("A", "a.test"), &no_ssh).unwrap();
        assert!(history_add(&store, "nope", "q", 1).is_err());
        assert!(history_add(&store, &c.id, "  \n", 1).is_err());
        history_add(&store, &c.id, &"x".repeat(MAX_HISTORY_BYTES), 1).unwrap();
        assert!(history_add(&store, &c.id, &"x".repeat(MAX_HISTORY_BYTES + 1), 2).is_err());
        assert_eq!(store.db_history(&c.id).unwrap().len(), 1);
    }

    #[test]
    fn kind_and_tls_wire_names() {
        assert_eq!(serde_json::to_string(&DbKind::Mongo).unwrap(), "\"mongo\"");
        assert_eq!(serde_json::to_string(&DbTls::Prefer).unwrap(), "\"prefer\"");
        let i: DbConnectionInput =
            serde_json::from_str(r#"{"kind":"postgres","name":"n","host":"h","tls":"disable","sshServerId":null}"#).unwrap();
        assert_eq!((i.kind, i.tls, i.accept_invalid_certs, i.port), (DbKind::Postgres, DbTls::Disable, false, None));
    }
}
