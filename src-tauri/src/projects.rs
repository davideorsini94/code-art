//! Progetti: logica pura (testabile con `&Store`), comandi Tauri e gestione finestre.
use crate::store::{Project, Result, Store};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::{AppHandle, Manager, State, TitleBarStyle, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use tauri_plugin_dialog::DialogExt;
use window_vibrancy::{apply_vibrancy, NSVisualEffectMaterial};

const PROJECT_PREFIX: &str = "proj-";
const WELCOME_LABEL: &str = "welcome";

// ---------- logica pura ----------

fn normalize(path: &str) -> String {
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

fn build_window(app: &AppHandle, label: &str, title: &str) -> tauri::Result<WebviewWindow> {
    let window = WebviewWindowBuilder::new(app, label, WebviewUrl::App("index.html".into()))
        .title(title)
        .inner_size(1400.0, 860.0)
        .min_inner_size(900.0, 560.0)
        .transparent(true)
        .title_bar_style(TitleBarStyle::Overlay)
        .hidden_title(false)
        .disable_drag_drop_handler() // serve per il drag & drop HTML5 dell'albero file
        .build()?;
    apply_vibrancy(&window, NSVisualEffectMaterial::HudWindow, None, None)
        .map_err(|e| tauri::Error::Anyhow(e.into()))?;
    Ok(window)
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
    }
    let quitting = app.state::<Quitting>().0.load(Ordering::SeqCst);
    let _ = handle_window_closed(&app.state::<Store>(), label, quitting);
}

pub fn pick_and_open(app: &AppHandle) {
    let app = app.clone();
    app.dialog().file().pick_folder({
        let app = app.clone();
        move |picked| {
            if let Some(path) = picked.and_then(|p| p.into_path().ok()) {
                let _ = open_path(&app, &path.to_string_lossy());
            }
        }
    });
}

fn open_path(app: &AppHandle, path: &str) -> Result<Project> {
    let project = open_folder(&app.state::<Store>(), path)?;
    show_project_window(app, &project)?;
    Ok(project)
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
        mark_quitting(app);
        app.exit(0);
    } else if id == "open_folder" {
        pick_and_open(app);
    } else if let Some(pid) = id.strip_prefix("recent:") {
        if let Ok(Some(p)) = app.state::<Store>().get_project(pid) {
            let _ = show_project_window(app, &p);
        }
    }
}

// ---------- comandi (involucri sottili) ----------

#[tauri::command]
pub fn project_open_folder(app: AppHandle, path: String) -> Result<Project> {
    open_path(&app, &path)
}

#[tauri::command]
pub fn project_current(window: WebviewWindow, store: State<Store>) -> Result<Project> {
    let id = window.label().strip_prefix(PROJECT_PREFIX).ok_or("questa finestra non è un progetto")?;
    must_get(&store, id)
}

#[tauri::command]
pub fn project_rename(app: AppHandle, store: State<Store>, id: String, name: String) -> Result<Project> {
    let p = rename(&store, &id, &name)?;
    if let Some(w) = app.get_webview_window(&window_label(&id)) {
        let _ = w.set_title(&p.name);
    }
    let _ = rebuild_menu(&app);
    Ok(p)
}

#[tauri::command]
pub fn project_add_folder(app: AppHandle, store: State<Store>, id: String, path: String) -> Result<Project> {
    let p = add_folder(&store, &id, &path)?;
    crate::fs_tree::watch_project(&app, &p);
    Ok(p)
}

#[tauri::command]
pub fn project_relocate(app: AppHandle, store: State<Store>, id: String, old: String, new: String) -> Result<Project> {
    let p = relocate(&store, &id, &old, &new)?;
    crate::fs_tree::watch_project(&app, &p);
    Ok(p)
}

#[tauri::command]
pub fn project_missing_folders(store: State<Store>, id: String) -> Result<Vec<String>> {
    missing_folders(&store, &id)
}

#[tauri::command]
pub fn project_pick_and_open(app: AppHandle) {
    pick_and_open(&app);
}

#[tauri::command]
pub fn project_branch(store: State<Store>, id: String) -> Result<Option<String>> {
    branch(&store, &id)
}

#[tauri::command]
pub fn settings_get(store: State<Store>, key: String) -> Result<Option<String>> {
    store.setting_get(&key)
}

#[tauri::command]
pub fn settings_set(store: State<Store>, key: String, value: String) -> Result<()> {
    store.setting_set(&key, &value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{Project, Store};

    fn store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let s = Store::open(&dir.path().join("t.db")).unwrap();
        (dir, s)
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
