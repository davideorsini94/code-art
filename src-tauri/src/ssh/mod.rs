//! SSH con l'`ssh` di sistema (nessuna libreria). Questo modulo: elenco dei server, salvati nel database dell'app
//! oltre a quelli letti (sola lettura) da `~/.ssh/config`.
pub mod config;

use crate::store::Store;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, WebviewWindow};

/// Dimensione massima di `~/.ssh/config` che si legge.
const MAX_CONFIG_BYTES: u64 = 256 * 1024;

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
    validate_target("Host", &s.host)?;
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
    }
    Ok(())
}

fn non_empty(v: &Option<String>) -> Option<String> {
    v.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string)
}

/// Crea o aggiorna un server salvato (mai uno da config: il suo id inizia con `config:`).
pub fn save_server(store: &Store, input: &ServerInput) -> Result<Server, String> {
    validate_server(input)?;
    let id = match input.id.as_deref() {
        Some(id) if id.starts_with("config:") => return Err("I server di ~/.ssh/config non si modificano qui".into()),
        Some(id) => {
            if !store.ssh_server_exists(id)? {
                return Err("Server non trovato".into());
            }
            id.to_string()
        }
        None => uuid::Uuid::new_v4().to_string(),
    };
    let server = Server {
        id,
        name: input.name.trim().to_string(),
        host: input.host.clone(),
        user: non_empty(&input.user),
        port: input.port,
        identity_file: non_empty(&input.identity_file),
        start_dir: non_empty(&input.start_dir),
        source: ServerSource::Saved,
        has_password: false,
    };
    store.ssh_server_upsert(&server)?;
    Ok(server)
}

pub fn delete_server(store: &Store, id: &str) -> Result<(), String> {
    if id.starts_with("config:") {
        return Err("I server di ~/.ssh/config non si eliminano qui".into());
    }
    store.ssh_server_delete(id)
}

fn config_servers(text: &str) -> Vec<Server> {
    config::parse_ssh_config(text)
        .into_iter()
        .filter(|h| validate_target("Alias", &h.alias).is_ok())
        .filter_map(|h| {
            let host = h.host_name.clone().unwrap_or_else(|| h.alias.clone());
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
pub fn list_servers(store: &Store, config_text: Option<&str>) -> Result<Vec<Server>, String> {
    let mut out = store.ssh_servers()?;
    out.extend(config_text.map(config_servers).unwrap_or_default());
    Ok(out)
}

/// `$HOME/.ssh/config` come testo: file assente, illeggibile, troppo grande o non UTF-8 = nessun server da config.
fn read_user_config() -> Option<String> {
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

fn trusted_store(app: &AppHandle, window: &WebviewWindow) -> Result<Store, String> {
    let project_id = crate::projects::window_project_id(window)?;
    let store = app.state::<Store>().inner().clone();
    crate::trust::ensure_project_trusted(&store, &project_id)?;
    Ok(store)
}

#[tauri::command]
pub async fn ssh_servers(app: AppHandle, window: WebviewWindow) -> Result<Vec<Server>, String> {
    let store = trusted_store(&app, &window)?;
    crate::projects::off_main(move || list_servers(&store, read_user_config().as_deref())).await
}

#[tauri::command]
pub async fn ssh_server_save(app: AppHandle, window: WebviewWindow, input: ServerInput) -> Result<Server, String> {
    let store = trusted_store(&app, &window)?;
    crate::projects::off_main(move || save_server(&store, &input)).await
}

#[tauri::command]
pub async fn ssh_server_delete(app: AppHandle, window: WebviewWindow, id: String) -> Result<(), String> {
    let store = trusted_store(&app, &window)?;
    crate::projects::off_main(move || delete_server(&store, &id)).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(name: &str, host: &str) -> ServerInput {
        ServerInput { name: name.into(), host: host.into(), ..Default::default() }
    }

    fn store() -> (tempfile::TempDir, Store) {
        let d = tempfile::tempdir().unwrap();
        let s = Store::open(&d.path().join("db.sqlite")).unwrap();
        (d, s)
    }

    #[test]
    fn validation_rules() {
        assert!(validate_server(&input("Dev", "example.test")).is_ok());
        for bad in ["", "a b", "-oProxyCommand=x", "a\nb", "a\u{0}b", "a\tb"] {
            assert!(validate_server(&input("Dev", bad)).is_err(), "host {bad:?}");
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
        let a = save_server(&st, &input(" Beta ", "b.example.test")).unwrap();
        assert_eq!(a.name, "Beta");
        let mut second = input("Alfa", "a.example.test");
        second.port = Some(2200);
        second.user = Some("dev".into());
        second.start_dir = Some("/home/dev".into());
        let b = save_server(&st, &second).unwrap();
        assert_ne!(a.id, b.id);
        let names: Vec<_> = list_servers(&st, None).unwrap().into_iter().map(|s| s.name).collect();
        assert_eq!(names, ["Alfa", "Beta"]);
        let mut upd = input("Alfa 2", "a2.example.test");
        upd.id = Some(b.id.clone());
        let u = save_server(&st, &upd).unwrap();
        assert_eq!(u.id, b.id);
        let all = list_servers(&st, None).unwrap();
        assert_eq!(all.len(), 2);
        let got = all.iter().find(|s| s.id == b.id).unwrap();
        assert_eq!((got.host.as_str(), got.user.clone(), got.port), ("a2.example.test", None, None));
        delete_server(&st, &a.id).unwrap();
        assert_eq!(list_servers(&st, None).unwrap().len(), 1);
        let mut ghost = input("X", "x.example.test");
        ghost.id = Some("manca".into());
        assert!(save_server(&st, &ghost).is_err());
        assert!(save_server(&st, &input("X", "-bad")).is_err());
    }

    #[test]
    fn config_servers_are_listed_read_only() {
        let (_d, st) = store();
        save_server(&st, &input("Mio", "m.example.test")).unwrap();
        let cfg = "Host web\n HostName w.example.test\n User dev\nHost solo\nHost -bad\n";
        let all = list_servers(&st, Some(cfg)).unwrap();
        let ids: Vec<_> = all.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(&ids[1..], ["config:web", "config:solo"]);
        assert_eq!(all[1].source, ServerSource::Config);
        assert_eq!(all[1].host, "w.example.test");
        assert_eq!(all[2].host, "solo");
        let mut edit = input("X", "x.example.test");
        edit.id = Some("config:web".into());
        assert!(save_server(&st, &edit).is_err());
        assert!(delete_server(&st, "config:web").is_err());
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
