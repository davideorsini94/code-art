//! SSH con l'`ssh` di sistema (nessuna libreria). Questo modulo: elenco dei server, salvati nel database dell'app
//! oltre a quelli letti (sola lettura) da `~/.ssh/config`, e le loro password (solo nel Portachiavi); le connessioni
//! condivise sono in `conn`.
pub mod askpass;
pub mod config;
pub mod conn;
pub mod fs;
pub mod keychain;
pub mod transfer;

use crate::fs_tree::FsError;
use crate::store::Store;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tauri::{AppHandle, Manager, WebviewWindow};

/// Dimensione massima di `~/.ssh/config` che si legge.
const MAX_CONFIG_BYTES: u64 = 256 * 1024;
/// Lunghezza massima di una password salvata.
const MAX_PASSWORD_BYTES: usize = 1024;

/// Dove stanno le password dei server (account = id del server): il Portachiavi nell'app, una mappa nei test.
pub trait Secrets {
    fn has(&self, id: &str) -> Result<bool, String>;
    fn set(&self, id: &str, secret: &str) -> Result<(), String>;
    fn clear(&self, id: &str) -> Result<(), String>;
}

/// Il Portachiavi di macOS (servizio `Code Art SSH`).
pub struct Keychain;

impl Secrets for Keychain {
    fn has(&self, id: &str) -> Result<bool, String> {
        keychain::exists(keychain::SERVICE, id)
    }
    fn set(&self, id: &str, secret: &str) -> Result<(), String> {
        keychain::set(keychain::SERVICE, id, secret)
    }
    fn clear(&self, id: &str) -> Result<(), String> {
        keychain::delete(keychain::SERVICE, id)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ServerSource {
    Saved,
    Config,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Server {
    pub id: String,
    pub name: String,
    pub host: String,
    pub user: Option<String>,
    pub port: Option<u16>,
    pub identity_file: Option<String>,
    pub start_dir: Option<String>,
    pub source: ServerSource,
    pub has_password: bool,
}

/// Campi modificabili di un server salvato; `id` assente = nuovo server.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerInput {
    pub id: Option<String>,
    pub name: String,
    pub host: String,
    pub user: Option<String>,
    pub port: Option<u16>,
    pub identity_file: Option<String>,
    pub start_dir: Option<String>,
}

/// Destinazione (host, utente, alias): non vuota, senza spazi né caratteri di controllo, mai con `-` iniziale
/// (la destinazione non si può proteggere con `--`: questa è l'unica difesa contro l'iniezione di opzioni).
pub fn validate_target(what: &str, v: &str) -> Result<(), String> {
    if v.is_empty() {
        return Err(format!("{what}: campo obbligatorio"));
    }
    if v.starts_with('-') {
        return Err(format!("{what}: non può iniziare con «-»"));
    }
    if v.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(format!("{what}: spazi e caratteri di controllo non sono ammessi"));
    }
    Ok(())
}

/// Host o alias: come `validate_target`, e solo lettere e cifre ASCII, `.`, `-`, `_`, `:` (IPv6). Niente `*`, `?`,
/// `!`, `,`, `@`, `/`…: in known_hosts sarebbero caratteri jolly, liste o altri host, nella destinazione un altro
/// utente o un URI. I nomi internazionali vanno scritti in punycode.
pub fn validate_host(what: &str, v: &str) -> Result<(), String> {
    validate_target(what, v)?;
    if !v.bytes().all(is_host_byte) {
        return Err(format!("{what}: solo lettere, cifre, «.», «-», «_» e «:» (nomi internazionali in punycode)"));
    }
    Ok(())
}

pub fn is_host_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b".-_:".contains(&b)
}

fn validate_text(what: &str, v: &str) -> Result<(), String> {
    if v.chars().any(|c| c.is_control()) {
        return Err(format!("{what}: caratteri di controllo non ammessi"));
    }
    Ok(())
}

pub fn validate_server(s: &ServerInput) -> Result<(), String> {
    let name = s.name.trim();
    if name.is_empty() {
        return Err("Nome: campo obbligatorio".into());
    }
    if name.chars().count() > 100 {
        return Err("Nome: al massimo 100 caratteri".into());
    }
    validate_text("Nome", name)?;
    validate_host("Host", &s.host)?;
    if let Some(u) = s.user.as_deref().filter(|u| !u.is_empty()) {
        validate_target("Utente", u)?;
    }
    if s.port == Some(0) {
        return Err("Porta: da 1 a 65535".into());
    }
    if let Some(f) = &s.identity_file {
        validate_text("File della chiave", f)?;
    }
    if let Some(d) = &s.start_dir {
        validate_text("Cartella iniziale", d)?;
        let d = d.trim();
        if !(d.is_empty() || d == "~" || d.starts_with("~/") || d.starts_with('/')) {
            return Err("Cartella iniziale: percorso assoluto («/…») o nella home («~», «~/…»)".into());
        }
    }
    Ok(())
}

fn non_empty(v: &Option<String>) -> Option<String> {
    v.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string)
}

/// Crea o aggiorna un server salvato (mai uno da config: il suo id inizia con `config:`). Un server esistente passa
/// prima da `disconnect` (dopo la validazione, prima della scrittura): la connessione aperta porterebbe al vecchio
/// indirizzo.
pub fn save_server(store: &Store, secrets: &dyn Secrets, input: &ServerInput, disconnect: &dyn Fn(&str)) -> Result<Server, String> {
    validate_server(input)?;
    let id = match input.id.as_deref() {
        Some(id) if id.starts_with("config:") => return Err("I server di ~/.ssh/config non si modificano qui".into()),
        Some(id) => {
            if !store.ssh_server_exists(id)? {
                return Err("Server non trovato".into());
            }
            disconnect(id);
            id.to_string()
        }
        None => uuid::Uuid::new_v4().to_string(),
    };
    let has_password = input.id.is_some() && secrets.has(&id).unwrap_or(false);
    let server = Server {
        id,
        name: input.name.trim().to_string(),
        host: input.host.clone(),
        user: non_empty(&input.user),
        port: input.port,
        identity_file: non_empty(&input.identity_file),
        start_dir: non_empty(&input.start_dir),
        source: ServerSource::Saved,
        has_password,
    };
    store.ssh_server_upsert(&server)?;
    Ok(server)
}

/// Elimina un server salvato e la sua password: prima la connessione (`disconnect`), poi il Portachiavi, così non
/// restano segreti senza server.
pub fn delete_server(store: &Store, secrets: &dyn Secrets, id: &str, disconnect: &dyn Fn(&str)) -> Result<(), String> {
    if id.starts_with("config:") {
        return Err("I server di ~/.ssh/config non si eliminano qui".into());
    }
    disconnect(id);
    secrets.clear(id)?;
    store.ssh_server_delete(id)
}

/// Password non vuota, al massimo 1024 byte, senza a capo né caratteri nulli (`ssh` legge una riga da askpass).
/// I messaggi non riportano mai la password.
pub fn validate_password(password: &str) -> Result<(), String> {
    if password.is_empty() {
        return Err("Password: campo obbligatorio".into());
    }
    if password.len() > MAX_PASSWORD_BYTES {
        return Err(format!("Password: al massimo {MAX_PASSWORD_BYTES} byte"));
    }
    if password.contains(['\n', '\r', '\0']) {
        return Err("Password: a capo e caratteri nulli non sono ammessi".into());
    }
    Ok(())
}

/// Le password si gestiscono solo per server esistenti: salvati, o alias attuali di `~/.ssh/config`.
fn ensure_known(store: &Store, config_text: Option<&str>, id: &str) -> Result<(), String> {
    let known = if id.starts_with("config:") {
        config_text.map(config_servers).unwrap_or_default().iter().any(|s| s.id == id)
    } else {
        store.ssh_server_exists(id)?
    };
    if known { Ok(()) } else { Err("Server non trovato".into()) }
}

pub fn set_password(
    store: &Store,
    secrets: &dyn Secrets,
    config_text: Option<&str>,
    id: &str,
    password: &str,
) -> Result<(), String> {
    ensure_known(store, config_text, id)?;
    validate_password(password)?;
    secrets.set(id, password)
}

pub fn clear_password(store: &Store, secrets: &dyn Secrets, config_text: Option<&str>, id: &str) -> Result<(), String> {
    ensure_known(store, config_text, id)?;
    secrets.clear(id)
}

fn config_servers(text: &str) -> Vec<Server> {
    config::parse_ssh_config(text)
        .into_iter()
        .filter(|h| validate_host("Alias", &h.alias).is_ok())
        .filter_map(|h| {
            let host = h.host_name.clone().unwrap_or_else(|| h.alias.clone());
            // `HostName` può contenere token (`%h`) che OpenSSH espande: si mostra solo, la verifica vera è su `ssh -G`
            validate_target("Host", &host).ok()?;
            Some(Server {
                id: format!("config:{}", h.alias),
                name: h.alias,
                host,
                user: h.user.filter(|u| validate_target("Utente", u).is_ok()),
                port: h.port,
                identity_file: h.identity_file.filter(|f| validate_text("File", f).is_ok()),
                start_dir: None,
                source: ServerSource::Config,
                has_password: false,
            })
        })
        .collect()
}

/// Salvati (per nome) seguiti da quelli della config; `config_text` = contenuto di `~/.ssh/config`, se c'è.
/// `has_password` dal Portachiavi (senza leggere il segreto; Portachiavi non accessibile = nessuna password).
pub fn list_servers(store: &Store, secrets: &dyn Secrets, config_text: Option<&str>) -> Result<Vec<Server>, String> {
    let mut out = store.ssh_servers()?;
    out.extend(config_text.map(config_servers).unwrap_or_default());
    for s in &mut out {
        s.has_password = secrets.has(&s.id).unwrap_or(false);
    }
    Ok(out)
}

/// `$HOME/.ssh/config` come testo: file assente, illeggibile, troppo grande o non UTF-8 = nessun server da config.
pub fn read_user_config() -> Option<String> {
    let path = std::path::PathBuf::from(std::env::var_os("HOME")?).join(".ssh/config");
    read_config_file(&path)
}

fn read_config_file(path: &std::path::Path) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() > MAX_CONFIG_BYTES {
        return None;
    }
    std::fs::read_to_string(path).ok()
}

pub(crate) fn trusted_store(app: &AppHandle, window: &WebviewWindow) -> Result<Store, String> {
    let project_id = crate::projects::window_project_id(window)?;
    let store = app.state::<Store>().inner().clone();
    crate::trust::ensure_project_trusted(&store, &project_id)?;
    Ok(store)
}

#[tauri::command]
pub async fn ssh_servers(app: AppHandle, window: WebviewWindow) -> Result<Vec<Server>, String> {
    let store = trusted_store(&app, &window)?;
    crate::projects::off_main(move || list_servers(&store, &Keychain, read_user_config().as_deref())).await
}

#[tauri::command]
pub async fn ssh_server_save(app: AppHandle, window: WebviewWindow, input: ServerInput) -> Result<Server, String> {
    let store = trusted_store(&app, &window)?;
    let conns = app.state::<Arc<conn::Connections>>().inner().clone();
    crate::projects::off_main(move || {
        save_server(&store, &Keychain, &input, &|id| {
            conns.disconnect_if_live(id);
        })
    })
    .await
}

#[tauri::command]
pub async fn ssh_server_delete(app: AppHandle, window: WebviewWindow, id: String) -> Result<(), String> {
    let store = trusted_store(&app, &window)?;
    let conns = app.state::<Arc<conn::Connections>>().inner().clone();
    crate::projects::off_main(move || {
        delete_server(&store, &Keychain, &id, &|id| {
            conns.disconnect_if_live(id);
        })
    })
    .await
}

/// Salva la password di un server nel Portachiavi; la password non torna mai alla WebView.
#[tauri::command]
pub async fn ssh_password_set(app: AppHandle, window: WebviewWindow, id: String, password: String) -> Result<(), String> {
    let store = trusted_store(&app, &window)?;
    crate::projects::off_main(move || set_password(&store, &Keychain, read_user_config().as_deref(), &id, &password)).await
}

#[tauri::command]
pub async fn ssh_password_clear(app: AppHandle, window: WebviewWindow, id: String) -> Result<(), String> {
    let store = trusted_store(&app, &window)?;
    crate::projects::off_main(move || clear_password(&store, &Keychain, read_user_config().as_deref(), &id)).await
}

/// Il server `id` (salvato o alias attuale di `~/.ssh/config`), senza toccare il Portachiavi.
pub fn find_server(store: &Store, config_text: Option<&str>, id: &str) -> Result<Server, String> {
    let mut all = store.ssh_servers()?;
    all.extend(config_text.map(config_servers).unwrap_or_default());
    all.into_iter().find(|s| s.id == id).ok_or_else(|| "Server non trovato".into())
}

/// Connette il server (connessione condivisa da tutte le finestre). Server nuovo: le impronte della sua chiave in un
/// dialogo nativo sulla finestra chiamante (titolo fisso, "Annulla" predefinito). `Err("auth")`: serve una password.
/// Già connesso o in connessione: `Ok` subito, lo stato arriva con l'evento `ssh:state`.
#[tauri::command]
pub async fn ssh_connect(app: AppHandle, window: WebviewWindow, id: String) -> Result<(), String> {
    use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind, MessageDialogResult};
    let store = trusted_store(&app, &window)?;
    let conns = app.state::<Arc<conn::Connections>>().inner().clone();
    crate::projects::off_main(move || {
        let server = find_server(&store, read_user_config().as_deref(), &id)?;
        let confirm = |message: &str| {
            let answer = app
                .dialog()
                .message(message)
                .title(conn::NEW_HOST_TITLE)
                .kind(MessageDialogKind::Warning)
                .buttons(MessageDialogButtons::OkCancelCustom(conn::BTN_CANCEL.into(), conn::BTN_TRUST_HOST.into()))
                .parent(&window)
                .blocking_show_with_result();
            matches!(answer, MessageDialogResult::Custom(l) if l == conn::BTN_TRUST_HOST)
        };
        conns.connect(&server, &confirm)
    })
    .await
}

#[tauri::command]
pub async fn ssh_disconnect(app: AppHandle, window: WebviewWindow, id: String) -> Result<(), String> {
    trusted_store(&app, &window)?;
    let conns = app.state::<Arc<conn::Connections>>().inner().clone();
    crate::projects::off_main(move || conns.disconnect(&id)).await
}

/// Stato delle connessioni note in questo avvio (le altre sono `disconnected`).
#[tauri::command]
pub async fn ssh_states(app: AppHandle, window: WebviewWindow) -> Result<Vec<conn::StateEvent>, String> {
    trusted_store(&app, &window)?;
    Ok(app.state::<Arc<conn::Connections>>().states())
}

/// File sul server: fiducia, server, lavoro bloccante fuori dal thread principale; `f` riceve il server e il modo di
/// eseguire uno script sulla sua connessione condivisa.
async fn remote<T: Send + 'static>(
    app: AppHandle,
    window: WebviewWindow,
    id: String,
    f: impl FnOnce(&Server, fs::Runner) -> Result<T, FsError> + Send + 'static,
) -> Result<T, FsError> {
    let store = trusted_store(&app, &window).map_err(|e| FsError::new("io", e))?;
    let conns = app.state::<Arc<conn::Connections>>().inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let server = find_server(&store, read_user_config().as_deref(), &id).map_err(|e| FsError::new("io", e))?;
        let run = |script: &str, args: &[&str], stdin: Option<Vec<u8>>| conns.run(&server, script, args, stdin);
        f(&server, &run)
    })
    .await
    .map_err(|e| FsError::new("io", e.to_string()))?
}

#[tauri::command]
pub async fn ssh_home(app: AppHandle, window: WebviewWindow, id: String) -> Result<String, FsError> {
    remote(app, window, id, |_, r| fs::home(r)).await
}

#[tauri::command]
pub async fn ssh_list(app: AppHandle, window: WebviewWindow, id: String, dir: String) -> Result<fs::RemoteListing, FsError> {
    remote(app, window, id, move |_, r| fs::list(r, &fs::resolve(r, &dir)?)).await
}

#[tauri::command]
pub async fn ssh_read(app: AppHandle, window: WebviewWindow, id: String, path: String) -> Result<fs::RemoteContent, FsError> {
    remote(app, window, id, move |_, r| fs::read(r, &fs::resolve(r, &path)?)).await
}

/// `version` = quella letta (`size:mtime`); vuota = crea un file nuovo. Restituisce la nuova versione.
#[tauri::command]
pub async fn ssh_write(app: AppHandle, window: WebviewWindow, id: String, path: String, text: String, version: String) -> Result<String, FsError> {
    remote(app, window, id, move |_, r| fs::write(r, &fs::resolve(r, &path)?, &text, &version)).await
}

#[tauri::command]
pub async fn ssh_mkdir(app: AppHandle, window: WebviewWindow, id: String, path: String) -> Result<(), FsError> {
    remote(app, window, id, move |_, r| fs::mkdir(r, &fs::resolve(r, &path)?)).await
}

#[tauri::command]
pub async fn ssh_rename(app: AppHandle, window: WebviewWindow, id: String, from: String, to: String) -> Result<(), FsError> {
    remote(app, window, id, move |_, r| fs::rename(r, &fs::resolve(r, &from)?, &fs::resolve(r, &to)?)).await
}

/// Elimina sul server dopo una conferma nativa del core (titolo fisso, "Annulla" predefinito): nessuna conferma passa
/// dalla WebView.
#[tauri::command]
pub async fn ssh_delete(app: AppHandle, window: WebviewWindow, id: String, path: String) -> Result<(), FsError> {
    use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind, MessageDialogResult};
    let dialogs = app.clone();
    let win = window.clone();
    remote(app, window, id, move |server, r| {
        let confirm = |message: &str| {
            let answer = dialogs
                .dialog()
                .message(message)
                .title(fs::DELETE_TITLE)
                .kind(MessageDialogKind::Warning)
                .buttons(MessageDialogButtons::OkCancelCustom(conn::BTN_CANCEL.into(), fs::BTN_DELETE.into()))
                .parent(&win)
                .blocking_show_with_result();
            matches!(answer, MessageDialogResult::Custom(l) if l == fs::BTN_DELETE)
        };
        fs::delete(r, &server.name, &fs::resolve(r, &path)?, &confirm)
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(name: &str, host: &str) -> ServerInput {
        ServerInput { name: name.into(), host: host.into(), ..Default::default() }
    }

    /// Portachiavi finto in memoria.
    #[derive(Default)]
    struct Fake(std::cell::RefCell<std::collections::HashMap<String, String>>);
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

    #[test]
    fn validation_rules() {
        assert!(validate_server(&input("Dev", "example.test")).is_ok());
        for bad in [
            "", "a b", "-oProxyCommand=x", "a\nb", "a\u{0}b", "a\tb", "*", "a,b", "!a", "a?b", "dev@example.test", "ssh://example.test",
            "a/b", "[::1]", "a|b", "@revoked", "bücher.example.test",
        ] {
            assert!(validate_server(&input("Dev", bad)).is_err(), "host {bad:?}");
        }
        for ok in ["example.test", "xn--bcher-kva.example.test", "192.0.2.1", "fe80::1", "dev_box-1"] {
            assert!(validate_server(&input("Dev", ok)).is_ok(), "host {ok:?}");
        }
        let mut u = input("Dev", "example.test");
        u.user = Some("-x".into());
        assert!(validate_server(&u).is_err());
        u.user = Some("de v".into());
        assert!(validate_server(&u).is_err());
        u.user = Some("dev".into());
        u.port = Some(0);
        assert!(validate_server(&u).is_err());
        u.port = Some(65535);
        assert!(validate_server(&u).is_ok());
        u.identity_file = Some("~/.ssh/id_x".into());
        u.start_dir = Some("/home/dev/app".into());
        assert!(validate_server(&u).is_ok());
        u.start_dir = Some("/home/\ndev".into());
        assert!(validate_server(&u).is_err());
        // cartella iniziale: vuota, `~`, `~/…` o assoluta
        for ok in ["", "  ", "~", "~/app", "/", "/srv/app", " /srv/app "] {
            u.start_dir = Some(ok.into());
            assert!(validate_server(&u).is_ok(), "{ok:?}");
        }
        for bad in ["app", "./app", "~dev/app", "../x", "$HOME/app"] {
            u.start_dir = Some(bad.into());
            assert!(validate_server(&u).unwrap_err().starts_with("Cartella iniziale"), "{bad:?}");
        }
        u.start_dir = None;
        u.identity_file = Some("a\u{0}".into());
        assert!(validate_server(&u).is_err());
        assert!(validate_server(&input("  ", "example.test")).is_err());
        assert!(validate_server(&input(&"x".repeat(101), "example.test")).is_err());
        assert!(validate_server(&input(&"x".repeat(100), "example.test")).is_ok());
    }

    #[test]
    fn save_update_list_delete() {
        let (_d, st) = store();
        let a = save_server(&st, &Fake::default(), &input(" Beta ", "b.example.test"), &|_| {}).unwrap();
        assert_eq!(a.name, "Beta");
        let mut second = input("Alfa", "a.example.test");
        second.port = Some(2200);
        second.user = Some("dev".into());
        second.start_dir = Some("/home/dev".into());
        let b = save_server(&st, &Fake::default(), &second, &|_| {}).unwrap();
        assert_ne!(a.id, b.id);
        let names: Vec<_> = list_servers(&st, &Fake::default(), None).unwrap().into_iter().map(|s| s.name).collect();
        assert_eq!(names, ["Alfa", "Beta"]);
        let mut upd = input("Alfa 2", "a2.example.test");
        upd.id = Some(b.id.clone());
        let u = save_server(&st, &Fake::default(), &upd, &|_| {}).unwrap();
        assert_eq!(u.id, b.id);
        let all = list_servers(&st, &Fake::default(), None).unwrap();
        assert_eq!(all.len(), 2);
        let got = all.iter().find(|s| s.id == b.id).unwrap();
        assert_eq!((got.host.as_str(), got.user.clone(), got.port), ("a2.example.test", None, None));
        delete_server(&st, &Fake::default(), &a.id, &|_| {}).unwrap();
        assert_eq!(list_servers(&st, &Fake::default(), None).unwrap().len(), 1);
        let mut ghost = input("X", "x.example.test");
        ghost.id = Some("manca".into());
        assert!(save_server(&st, &Fake::default(), &ghost, &|_| {}).is_err());
        assert!(save_server(&st, &Fake::default(), &input("X", "-bad"), &|_| {}).is_err());
    }

    #[test]
    fn saving_or_deleting_an_existing_server_disconnects_it_first() {
        let (_d, st) = store();
        let k = Fake::default();
        let calls = std::cell::RefCell::new(Vec::<String>::new());
        let rec = |id: &str| calls.borrow_mut().push(id.to_string());
        // nuovo: niente da disconnettere
        let a = save_server(&st, &k, &input("A", "a.example.test"), &rec).unwrap();
        assert!(calls.borrow().is_empty());
        // modifica non valida: nessuna disconnessione, nessuna scrittura
        let mut bad = input("A", "-x");
        bad.id = Some(a.id.clone());
        assert!(save_server(&st, &k, &bad, &rec).is_err());
        assert!(calls.borrow().is_empty());
        // modifica: prima la disconnessione, poi il record nuovo
        let mut upd = input("A", "staging.example.test");
        upd.id = Some(a.id.clone());
        let seen = std::cell::RefCell::new(None);
        save_server(&st, &k, &upd, &|id| *seen.borrow_mut() = Some(find_server(&st, None, id).unwrap().host)).unwrap();
        assert_eq!(seen.borrow().as_deref(), Some("a.example.test"), "disconnesso prima di scrivere");
        // eliminazione: disconnesso
        delete_server(&st, &k, &a.id, &rec).unwrap();
        assert_eq!(calls.borrow().as_slice(), std::slice::from_ref(&a.id));
        assert!(delete_server(&st, &k, "config:web", &rec).is_err());
        assert_eq!(calls.borrow().len(), 1);
    }

    #[test]
    fn config_servers_are_listed_read_only() {
        let (_d, st) = store();
        save_server(&st, &Fake::default(), &input("Mio", "m.example.test"), &|_| {}).unwrap();
        let cfg = "Host web\n HostName w.example.test\n User dev\nHost solo\nHost -bad a,b u@h\n";
        let all = list_servers(&st, &Fake::default(), Some(cfg)).unwrap();
        let ids: Vec<_> = all.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(&ids[1..], ["config:web", "config:solo"]);
        assert_eq!(all[1].source, ServerSource::Config);
        assert_eq!(all[1].host, "w.example.test");
        assert_eq!(all[2].host, "solo");
        let mut edit = input("X", "x.example.test");
        edit.id = Some("config:web".into());
        assert!(save_server(&st, &Fake::default(), &edit, &|_| {}).is_err());
        assert!(delete_server(&st, &Fake::default(), "config:web", &|_| {}).is_err());
    }

    #[test]
    fn passwords_live_in_secrets_for_known_servers_only() {
        let (_d, st) = store();
        let k = Fake::default();
        let a = save_server(&st, &k, &input("A", "a.example.test"), &|_| {}).unwrap();
        assert!(!a.has_password);
        let cfg = Some("Host web\n HostName w.example.test\n");
        set_password(&st, &k, cfg, &a.id, "pàss word").unwrap();
        set_password(&st, &k, cfg, "config:web", "altra").unwrap();
        assert_eq!(k.0.borrow().get(&a.id).map(String::as_str), Some("pàss word"));
        let all = list_servers(&st, &k, cfg).unwrap();
        assert!(all.iter().all(|s| s.has_password));
        // aggiornare un server conserva la password e lo dice
        let mut upd = input("A2", "a.example.test");
        upd.id = Some(a.id.clone());
        assert!(save_server(&st, &k, &upd, &|_| {}).unwrap().has_password);
        // server sconosciuti: né salvati né alias attuali della config
        for id in ["manca", "config:altro", ""] {
            assert_eq!(set_password(&st, &k, cfg, id, "x").unwrap_err(), "Server non trovato");
            assert_eq!(clear_password(&st, &k, cfg, id).unwrap_err(), "Server non trovato");
        }
        assert!(set_password(&st, &k, None, "config:web", "x").is_err());
        clear_password(&st, &k, cfg, "config:web").unwrap();
        assert!(!list_servers(&st, &k, cfg).unwrap()[1].has_password);
        // eliminare il server elimina anche la password
        delete_server(&st, &k, &a.id, &|_| {}).unwrap();
        assert!(k.0.borrow().is_empty());
    }

    #[test]
    fn password_rules_never_echo_the_secret() {
        let (_d, st) = store();
        let k = Fake::default();
        let a = save_server(&st, &k, &input("A", "a.example.test"), &|_| {}).unwrap();
        let long = "s".repeat(1025);
        for bad in ["", long.as_str(), "segreto\nriga", "segreto\r", "segreto\0"] {
            let e = set_password(&st, &k, None, &a.id, bad).unwrap_err();
            assert!(!e.contains("segreto") && !e.contains("sss"), "{e}");
            assert!(e.starts_with("Password: "), "{e}");
        }
        assert!(k.0.borrow().is_empty());
        set_password(&st, &k, None, &a.id, &"s".repeat(1024)).unwrap();
        assert!(validate_password("è").is_ok());
    }

    #[test]
    fn find_server_covers_saved_and_config() {
        let (_d, st) = store();
        let a = save_server(&st, &Fake::default(), &input("A", "a.example.test"), &|_| {}).unwrap();
        let cfg = Some("Host web\n HostName w.example.test\n");
        assert_eq!(find_server(&st, cfg, &a.id).unwrap().host, "a.example.test");
        let w = find_server(&st, cfg, "config:web").unwrap();
        assert_eq!((w.source, conn::destination(&w).as_str()), (ServerSource::Config, "web"));
        assert_eq!(find_server(&st, None, "config:web").unwrap_err(), "Server non trovato");
        assert_eq!(find_server(&st, cfg, "manca").unwrap_err(), "Server non trovato");
    }

    #[test]
    fn config_file_reading_is_defensive() {
        let d = tempfile::tempdir().unwrap();
        assert_eq!(read_config_file(&d.path().join("manca")), None);
        let f = d.path().join("config");
        std::fs::write(&f, "Host a\n").unwrap();
        assert_eq!(read_config_file(&f).as_deref(), Some("Host a\n"));
        std::fs::write(&f, vec![b'#'; 256 * 1024 + 1]).unwrap();
        assert_eq!(read_config_file(&f), None);
        std::fs::write(&f, [0xff, 0xfe]).unwrap();
        assert_eq!(read_config_file(&f), None);
    }
}
