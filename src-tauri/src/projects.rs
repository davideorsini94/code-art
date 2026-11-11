//! Progetti: logica pura (testabile con `&Store`), comandi Tauri e gestione finestre.
use crate::store::{Project, Result, Store};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::{AppHandle, Manager, State, TitleBarStyle, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use tauri_plugin_dialog::DialogExt;

pub const PROJECT_PREFIX: &str = "proj-";
const WELCOME_LABEL: &str = "welcome";

// ---------- logica pura ----------

pub(crate) fn normalize(path: &str) -> String {
    let t = path.trim_end_matches('/');
    if t.is_empty() { "/".to_string() } else { t.to_string() }
}

fn must_get(store: &Store, id: &str) -> Result<Project> {
    store.get_project(id)?.ok_or_else(|| format!("progetto {id} inesistente"))
}

pub fn open_folder(store: &Store, path: &str) -> Result<Project> {
    let path = normalize(path);
    if let Some(p) = store.list_projects()?.into_iter().find(|p| p.folders.first() == Some(&path)) {
        return Ok(p);
    }
    let name = Path::new(&path)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.clone());
    let p = Project { id: uuid::Uuid::new_v4().to_string(), name, folders: vec![path] };
    store.upsert_project(&p)?;
    Ok(p)
}

pub fn rename(store: &Store, id: &str, name: &str) -> Result<Project> {
    let name = name.trim();
    if name.is_empty() {
        return Err("il nome non può essere vuoto".into());
    }
    let mut p = must_get(store, id)?;
    p.name = name.to_string();
    store.upsert_project(&p)?;
    Ok(p)
}

pub fn add_folder(store: &Store, id: &str, path: &str) -> Result<Project> {
    let path = normalize(path);
    let mut p = must_get(store, id)?;
    if !p.folders.contains(&path) {
        p.folders.push(path);
        store.upsert_project(&p)?;
    }
    Ok(p)
}

pub fn relocate(store: &Store, id: &str, old: &str, new: &str) -> Result<Project> {
    let (old, new) = (normalize(old), normalize(new));
    let mut p = must_get(store, id)?;
    let slot = p.folders.iter_mut().find(|f| **f == old).ok_or("cartella non trovata nel progetto")?;
    *slot = new;
    store.upsert_project(&p)?;
    Ok(p)
}

pub fn missing_folders(store: &Store, id: &str) -> Result<Vec<String>> {
    Ok(must_get(store, id)?.folders.into_iter().filter(|f| !Path::new(f).is_dir()).collect())
}

/// Branch corrente della cartella principale, letto da `.git/HEAD` (anche worktree con `.git` file).
pub fn branch(store: &Store, id: &str) -> Result<Option<String>> {
    let p = must_get(store, id)?;
    let Some(root) = p.folders.first() else { return Ok(None) };
    let mut git = Path::new(root).join(".git");
    if git.is_file() {
        let Ok(text) = std::fs::read_to_string(&git) else { return Ok(None) };
        let Some(dir) = text.trim().strip_prefix("gitdir:") else { return Ok(None) };
        let dir = dir.trim();
        git = if Path::new(dir).is_absolute() { dir.into() } else { Path::new(root).join(dir) };
    }
    let Ok(head) = std::fs::read_to_string(git.join("HEAD")) else { return Ok(None) };
    let head = head.trim();
    Ok(match head.strip_prefix("ref: refs/heads/") {
        Some(b) => Some(b.to_string()),
        None if head.len() >= 7 && head.chars().all(|c| c.is_ascii_hexdigit()) => Some(head[..7].to_string()),
        None => None,
    })
}

// ---------- finestre ----------

/// Impostato all'uscita dell'app: la chiusura delle finestre in quel momento non deve azzerare `was_open`.
#[derive(Default)]
pub struct Quitting(pub AtomicBool);

pub fn window_label(project_id: &str) -> String {
    format!("{PROJECT_PREFIX}{project_id}")
}

/// Nome di un evento di un progetto (`conv:event`, `fs:changed`, `pty:data`…): l'id del progetto fa parte del
/// nome. Un ascoltatore globale (`listen` senza finestra) di un'altra finestra riceve ogni evento emesso con
/// `emit_to`; con l'id nel nome deve almeno conoscere l'UUID del progetto, che le altre finestre non vedono.
/// Lo stesso nome lo costruisce l'interfaccia (`src/lib/events.ts`) dal proprio id di progetto.
pub fn project_event(name: &str, project_id: &str) -> String {
    format!("{name}:{project_id}")
}

/// Emette un evento di progetto alla sola finestra del progetto (vedi `project_event`).
pub fn emit_to_project<S: serde::Serialize + Clone>(app: &AppHandle, project_id: &str, name: &str, payload: S) {
    use tauri::Emitter;
    let _ = app.emit_to(window_label(project_id).as_str(), project_event(name, project_id).as_str(), payload);
}

/// La WebView resta sempre sulla pagina dell'app: si naviga solo verso la sua origine (`tauri://localhost`,
/// `http(s)://tauri.localhost`) e, nelle build di sviluppo, verso l'URL del server di Vite. Tutto il resto (un file o
/// un link trascinato fuori dalle zone di rilascio, un `location =` da una pagina compromessa) è bloccato; i link
/// esterni passano solo dal browser di sistema (`opener`).
pub fn allow_navigation(url: &tauri::Url, dev_url: Option<&tauri::Url>) -> bool {
    match (url.scheme(), url.host_str()) {
        ("tauri", Some("localhost")) | ("http" | "https", Some("tauri.localhost")) => true,
        _ => dev_url.is_some_and(|d| {
            d.scheme() == url.scheme() && d.host_str() == url.host_str() && d.port_or_known_default() == url.port_or_known_default()
        }),
    }
}

/// Il titolo nativo resta impostato (Mission Control, menu Finestra, Dock) ma non va disegnato
/// nella barra: lo disegna già `Titlebar.svelte`.
fn build_window(app: &AppHandle, label: &str, title: &str) -> tauri::Result<WebviewWindow> {
    let dev_url = if cfg!(debug_assertions) { app.config().build.dev_url.clone() } else { None };
    WebviewWindowBuilder::new(app, label, WebviewUrl::App("index.html".into()))
        .on_navigation(move |url| allow_navigation(url, dev_url.as_ref()))
        .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny)
        .title(title)
        .inner_size(1400.0, 860.0)
        .min_inner_size(900.0, 560.0)
        .transparent(false) // finestra opaca (scelta dell'utente): il vetro è solo dentro la pagina
        .title_bar_style(TitleBarStyle::Overlay)
        .hidden_title(true)
        .disable_drag_drop_handler() // serve per il drag & drop HTML5 dell'albero file
        .build()
}

/// Apre (o porta in primo piano) la finestra del progetto e ne ricorda lo stato "aperto".
pub fn show_project_window(app: &AppHandle, project: &Project) -> Result<()> {
    let store = app.state::<Store>();
    let label = window_label(&project.id);
    if let Some(w) = app.get_webview_window(&label) {
        let _ = w.unminimize();
        w.set_focus().map_err(|e| e.to_string())?;
    } else {
        build_window(app, &label, &project.name).map_err(|e| e.to_string())?;
    }
    store.set_open_flag(&project.id, true)?;
    crate::fs_tree::watch_project(app, project);
    crate::claude::sessions::watch_project(app, project);
    if let Some(w) = app.get_webview_window(WELCOME_LABEL) {
        let _ = w.close();
    }
    let _ = rebuild_menu(app);
    Ok(())
}

pub fn show_welcome_window(app: &AppHandle) -> Result<()> {
    if let Some(w) = app.get_webview_window(WELCOME_LABEL) {
        w.set_focus().map_err(|e| e.to_string())?;
    } else {
        build_window(app, WELCOME_LABEL, "Code Art").map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub fn restore_windows(app: &AppHandle) -> Result<()> {
    let open = app.state::<Store>().open_projects()?;
    if open.is_empty() {
        return show_welcome_window(app);
    }
    for p in &open {
        if let Err(e) = show_project_window(app, p) {
            eprintln!("impossibile riaprire il progetto {}: {e}", p.name);
        }
    }
    Ok(())
}

/// La chiusura di una finestra azzera `was_open` solo se non stiamo uscendo dall'app.
pub fn should_clear_open_flag(quitting: bool) -> bool {
    !quitting
}

/// Dimentica lo stato "aperto" di un progetto dopo la chiusura della sua finestra.
pub fn handle_window_closed(store: &Store, label: &str, quitting: bool) -> Result<()> {
    if !should_clear_open_flag(quitting) {
        return Ok(());
    }
    match label.strip_prefix(PROJECT_PREFIX) {
        Some(id) => store.set_open_flag(id, false),
        None => Ok(()),
    }
}

pub fn mark_quitting(app: &AppHandle) {
    app.state::<Quitting>().0.store(true, Ordering::SeqCst);
}

pub fn on_window_destroyed(app: &AppHandle, label: &str) {
    if let Some(id) = label.strip_prefix(PROJECT_PREFIX) {
        crate::fs_tree::unwatch_project(app, id);
        crate::claude::sessions::unwatch_project(app, id);
    }
    app.state::<crate::pty::Ptys>().kill_owner(label);
    let quitting = app.state::<Quitting>().0.load(Ordering::SeqCst);
    let _ = handle_window_closed(&app.state::<Store>(), label, quitting);
}

/// Selettore nativo "Apri cartella": unica via (con menu e recenti) per aprire una cartella come progetto.
/// La cartella scelta qui diventa fidata.
pub fn pick_and_open(app: &AppHandle) {
    let app = app.clone();
    app.dialog().file().pick_folder({
        let app = app.clone();
        move |picked| {
            if let Some(path) = picked.and_then(|p| p.into_path().ok()) {
                let path = path.to_string_lossy();
                let _ = crate::trust::mark_trusted(&app.state::<Store>(), &path);
                let _ = open_path(&app, &path);
            }
        }
    });
}

fn open_path(app: &AppHandle, path: &str) -> Result<Project> {
    let project = open_folder(&app.state::<Store>(), path)?;
    show_project_window(app, &project)?;
    Ok(project)
}

/// Selettore nativo come foglio della finestra, aperto dal core: il percorso non arriva mai dalla WebView.
/// La cartella scelta diventa fidata. `None` = annullato.
async fn pick_folder_for(app: &AppHandle, window: &WebviewWindow, title: &str) -> Result<Option<String>> {
    let dialog = app.dialog().file().set_title(title).set_parent(window);
    let picked = tauri::async_runtime::spawn_blocking(move || dialog.blocking_pick_folder())
        .await
        .map_err(|e| e.to_string())?;
    let Some(path) = picked.and_then(|p| p.into_path().ok()) else { return Ok(None) };
    let path = path.to_string_lossy().into_owned();
    crate::trust::mark_trusted(&app.state::<Store>(), &path)?;
    Ok(Some(path))
}

/// Id del progetto di una finestra a partire dalla sua etichetta (`proj-<id>`); `None` per le altre finestre.
pub fn project_id_of_label(label: &str) -> Option<&str> {
    label.strip_prefix(PROJECT_PREFIX).filter(|id| !id.is_empty())
}

/// Progetto della finestra chiamante (dall'etichetta, mai da un argomento). Unico punto usato dai comandi.
pub(crate) fn window_project_id(window: &WebviewWindow) -> Result<String> {
    project_id_of_label(window.label()).map(String::from).ok_or_else(|| "questa finestra non è un progetto".into())
}

// ---------- menu ----------

pub fn rebuild_menu(app: &AppHandle) -> tauri::Result<()> {
    let recent = Submenu::with_id(app, "recent", "Recenti", true)?;
    if let Ok(list) = app.state::<Store>().list_projects() {
        for p in list.iter().take(10) {
            recent.append(&MenuItem::with_id(app, format!("recent:{}", p.id), &p.name, true, None::<&str>)?)?;
        }
    }
    let file = Submenu::with_items(
        app,
        "File",
        true,
        &[
            &MenuItem::with_id(app, "open_folder", "Apri cartella…", true, Some("CmdOrCtrl+O"))?,
            &recent,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::close_window(app, Some("Chiudi finestra"))?,
        ],
    )?;
    let app_menu = Submenu::with_items(
        app,
        "Code Art",
        true,
        &[
            &PredefinedMenuItem::about(app, Some("Informazioni su Code Art"), None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::hide(app, Some("Nascondi Code Art"))?,
            &PredefinedMenuItem::hide_others(app, Some("Nascondi altri"))?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, "quit", "Esci da Code Art", true, Some("CmdOrCtrl+Q"))?,
        ],
    )?;
    let edit = Submenu::with_items(
        app,
        "Modifica",
        true,
        &[
            &PredefinedMenuItem::undo(app, Some("Annulla"))?,
            &PredefinedMenuItem::redo(app, Some("Ripeti"))?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::cut(app, Some("Taglia"))?,
            &PredefinedMenuItem::copy(app, Some("Copia"))?,
            &PredefinedMenuItem::paste(app, Some("Incolla"))?,
            &PredefinedMenuItem::select_all(app, Some("Seleziona tutto"))?,
        ],
    )?;
    let window = Submenu::with_items(
        app,
        "Finestra",
        true,
        &[&PredefinedMenuItem::minimize(app, Some("Riduci a icona"))?],
    )?;
    app.set_menu(Menu::with_items(app, &[&app_menu, &file, &edit, &window])?)?;
    Ok(())
}

pub fn on_menu_event(app: &AppHandle, id: &str) {
    if id == "quit" {
        crate::lifecycle::request_quit(app);
    } else if id == "open_folder" {
        pick_and_open(app);
    } else if let Some(pid) = id.strip_prefix("recent:") {
        if let Ok(Some(p)) = app.state::<Store>().get_project(pid) {
            let _ = show_project_window(app, &p);
        }
    }
}

// ---------- comandi (involucri sottili) ----------
// Nessun comando accetta un percorso o un id di progetto dalla WebView: il progetto è quello della finestra
// chiamante, le cartelle nuove arrivano solo dal selettore nativo.

#[tauri::command]
pub fn project_current(window: WebviewWindow, store: State<Store>) -> Result<Project> {
    must_get(&store, &window_project_id(&window)?)
}

#[tauri::command]
pub fn project_rename(app: AppHandle, window: WebviewWindow, store: State<Store>, name: String) -> Result<Project> {
    let p = rename(&store, &window_project_id(&window)?, &name)?;
    let _ = window.set_title(&p.name);
    let _ = rebuild_menu(&app);
    Ok(p)
}

/// Aggiunge al progetto della finestra una cartella scelta con il selettore nativo. `None` = annullato.
#[tauri::command]
pub async fn project_pick_and_add_folder(app: AppHandle, window: WebviewWindow) -> Result<Option<Project>> {
    let id = window_project_id(&window)?;
    let Some(path) = pick_folder_for(&app, &window, "Aggiungi cartella al progetto").await? else { return Ok(None) };
    let p = add_folder(&app.state::<Store>(), &id, &path)?;
    crate::fs_tree::watch_project(&app, &p);
    crate::claude::sessions::watch_project(&app, &p);
    Ok(Some(p))
}

/// Sostituisce la cartella `old` del progetto della finestra con una scelta dal selettore nativo. `None` = annullato.
#[tauri::command]
pub async fn project_pick_relocate(app: AppHandle, window: WebviewWindow, old: String) -> Result<Option<Project>> {
    let id = window_project_id(&window)?;
    if !must_get(&app.state::<Store>(), &id)?.folders.contains(&normalize(&old)) {
        return Err("cartella non trovata nel progetto".into());
    }
    let Some(path) = pick_folder_for(&app, &window, "Scegli la nuova posizione").await? else { return Ok(None) };
    let p = relocate(&app.state::<Store>(), &id, &old, &path)?;
    crate::fs_tree::watch_project(&app, &p);
    crate::claude::sessions::watch_project(&app, &p);
    Ok(Some(p))
}

#[tauri::command]
pub fn project_missing_folders(window: WebviewWindow, store: State<Store>) -> Result<Vec<String>> {
    missing_folders(&store, &window_project_id(&window)?)
}

#[tauri::command]
pub fn project_pick_and_open(app: AppHandle) {
    pick_and_open(&app);
}

#[tauri::command]
pub fn project_branch(window: WebviewWindow, store: State<Store>) -> Result<Option<String>> {
    branch(&store, &window_project_id(&window)?)
}

/// Impostazioni modificabili dalla WebView, ciascuna con il suo validatore.
const WEB_SETTINGS: &[(&str, fn(&str) -> bool)] =
    &[("default_permission_mode", crate::claude::engine::valid_permission_mode)];

fn web_setting(key: &str) -> Result<fn(&str) -> bool> {
    WEB_SETTINGS.iter().find(|(k, _)| *k == key).map(|(_, v)| *v).ok_or_else(|| format!("impostazione sconosciuta: {key}"))
}

/// Valida chiave e valore di un'impostazione scritta dalla WebView.
pub fn check_web_setting(key: &str, value: &str) -> Result<()> {
    if web_setting(key)?(value) { Ok(()) } else { Err(format!("valore non valido per {key}")) }
}

#[tauri::command]
pub fn settings_get(store: State<Store>, key: String) -> Result<Option<String>> {
    web_setting(&key)?;
    store.setting_get(&key)
}

#[tauri::command]
pub fn settings_set(store: State<Store>, key: String, value: String) -> Result<()> {
    check_web_setting(&key, &value)?;
    store.setting_set(&key, &value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{Project, Store};

    /// Corpo di `build_window` letto dal sorgente: i test sotto fissano le opzioni della finestra.
    fn build_window_source() -> &'static str {
        let src = include_str!("projects.rs");
        let start = src.find("fn build_window(").unwrap();
        let end = start + src[start..].find("\n}\n").unwrap();
        &src[start..end]
    }

    #[test]
    fn window_is_opaque_without_vibrancy() {
        let body = build_window_source();
        assert!(body.contains(".transparent(false)"), "{body}");
        assert!(!body.contains("vibrancy"), "{body}");
        assert!(!include_str!("../Cargo.toml").contains("window-vibrancy"));
    }

    #[test]
    fn native_title_hidden_but_still_set() {
        // senza hidden_title il titolo appare doppio (nativo + Titlebar.svelte); .title resta per Mission Control
        let body = build_window_source();
        assert!(body.contains(".hidden_title(true)"), "{body}");
        assert!(body.contains(".title(title)"), "{body}");
    }

    fn store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let s = Store::open(&dir.path().join("t.db")).unwrap();
        (dir, s)
    }

    #[test]
    fn navigation_stays_on_the_app_origin() {
        let u = |s: &str| tauri::Url::parse(s).unwrap();
        let dev = u("http://localhost:1420");
        for ok in ["tauri://localhost/", "tauri://localhost/index.html", "http://tauri.localhost/", "https://tauri.localhost/x"] {
            assert!(allow_navigation(&u(ok), None), "{ok}");
        }
        assert!(allow_navigation(&u("http://localhost:1420/src/main.ts"), Some(&dev)));
        for bad in [
            "file:///Users/dev/progetto/a.html",
            "https://example.com/",
            "http://localhost:1420/",
            "tauri://evil/",
            "http://tauri.localhost.example.com/",
            "data:text/html,<script>1</script>",
            "blob:tauri://localhost/123",
            "about:blank",
        ] {
            assert!(!allow_navigation(&u(bad), None), "{bad}");
        }
        assert!(!allow_navigation(&u("http://localhost:1421/"), Some(&dev)));
        assert!(!allow_navigation(&u("https://localhost:1420/"), Some(&dev)));
        assert!(build_window_source().contains(".on_navigation("), "ogni finestra ha la guardia");
    }

    #[test]
    fn project_events_carry_the_project_id_in_the_name() {
        let id = "8f0e2c1a-1111-4111-8111-111111111111";
        for name in ["conv:event", "conv:upsert", "conv:removed", "conv:focus", "fs:changed", "pty:data", "pty:exit"] {
            assert_eq!(project_event(name, id), format!("{name}:{id}"));
        }
        // ogni emissione di progetto passa dall'unico helper: nessun `emit_to` con un nome fisso altrove
        for (file, src) in [
            ("fs_tree.rs", include_str!("fs_tree.rs")),
            ("pty.rs", include_str!("pty.rs")),
            ("lifecycle.rs", include_str!("lifecycle.rs")),
            ("claude/sessions.rs", include_str!("claude/sessions.rs")),
            ("claude/engine.rs", include_str!("claude/engine.rs")),
        ] {
            assert!(!src.contains(".emit_to("), "{file}: usare projects::emit_to_project");
        }
    }

    #[test]
    fn open_folder_names_project_after_last_segment() {
        let (_d, s) = store();
        let p = open_folder(&s, "/tmp/x/demo-api").unwrap();
        assert_eq!(p.name, "demo-api");
        assert_eq!(p.folders, vec!["/tmp/x/demo-api".to_string()]);
    }

    #[test]
    fn open_same_folder_twice_returns_same_project() {
        let (_d, s) = store();
        let a = open_folder(&s, "/tmp/x/demo-api").unwrap();
        let b = open_folder(&s, "/tmp/x/demo-api/").unwrap();
        assert_eq!(a.id, b.id);
        assert_eq!(s.list_projects().unwrap().len(), 1);
    }

    #[test]
    fn rename_persists() {
        let (_d, s) = store();
        let p = open_folder(&s, "/tmp/x/demo-api").unwrap();
        rename(&s, &p.id, "Altro").unwrap();
        assert_eq!(s.get_project(&p.id).unwrap().unwrap().name, "Altro");
    }

    #[test]
    fn open_projects_returns_only_flagged() {
        let (_d, s) = store();
        let a = open_folder(&s, "/tmp/x/a").unwrap();
        let _b = open_folder(&s, "/tmp/x/b").unwrap();
        s.set_open_flag(&a.id, true).unwrap();
        let open = s.open_projects().unwrap();
        assert_eq!(open.len(), 1);
        assert_eq!(open[0].id, a.id);
        s.set_open_flag(&a.id, false).unwrap();
        assert!(s.open_projects().unwrap().is_empty());
    }

    #[test]
    fn missing_folder_is_reported_not_fatal() {
        let (d, s) = store();
        let real = d.path().to_string_lossy().to_string();
        let p = Project {
            id: "p1".into(),
            name: "p".into(),
            folders: vec!["/definitely/not/here".into(), real],
        };
        s.upsert_project(&p).unwrap();
        assert!(s.get_project("p1").unwrap().is_some());
        assert_eq!(missing_folders(&s, "p1").unwrap(), vec!["/definitely/not/here".to_string()]);
    }

    #[test]
    fn closing_window_clears_flag_only_when_not_quitting() {
        assert!(should_clear_open_flag(false));
        assert!(!should_clear_open_flag(true));
        let (_d, s) = store();
        let p = open_folder(&s, "/tmp/x/a").unwrap();
        let label = window_label(&p.id);
        s.set_open_flag(&p.id, true).unwrap();
        handle_window_closed(&s, &label, true).unwrap();
        assert_eq!(s.open_projects().unwrap().len(), 1);
        handle_window_closed(&s, "welcome", false).unwrap();
        assert_eq!(s.open_projects().unwrap().len(), 1);
        handle_window_closed(&s, &label, false).unwrap();
        assert!(s.open_projects().unwrap().is_empty());
    }

    #[test]
    fn add_folder_and_relocate() {
        let (_d, s) = store();
        let p = open_folder(&s, "/tmp/x/a").unwrap();
        add_folder(&s, &p.id, "/tmp/x/b").unwrap();
        add_folder(&s, &p.id, "/tmp/x/b").unwrap();
        assert_eq!(s.get_project(&p.id).unwrap().unwrap().folders.len(), 2);
        relocate(&s, &p.id, "/tmp/x/a", "/tmp/y/a").unwrap();
        assert_eq!(s.get_project(&p.id).unwrap().unwrap().folders[0], "/tmp/y/a");
    }

    #[test]
    fn web_settings_are_whitelisted_and_validated() {
        assert!(check_web_setting("default_permission_mode", "plan").is_ok());
        assert!(check_web_setting("default_permission_mode", "tutto").is_err());
        assert!(check_web_setting("claude_path", "/tmp/x").is_err());
    }

    #[test]
    fn branch_reads_git_head() {
        let (d, s) = store();
        let root = d.path().join("repo");
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::write(root.join(".git/HEAD"), "ref: refs/heads/feature/x\n").unwrap();
        let p = open_folder(&s, root.to_str().unwrap()).unwrap();
        assert_eq!(branch(&s, &p.id).unwrap().as_deref(), Some("feature/x"));
        std::fs::write(root.join(".git/HEAD"), "0123456789abcdef0123\n").unwrap();
        assert_eq!(branch(&s, &p.id).unwrap().as_deref(), Some("0123456"));
        std::fs::remove_dir_all(root.join(".git")).unwrap();
        assert_eq!(branch(&s, &p.id).unwrap(), None);
    }
}
