//! Cartelle fidate: Claude (in bypass permissions) e il terminale eseguono codice, quindi partono solo in
//! cartelle che l'utente ha scelto con il selettore nativo o confermato con la domanda nativa.
//! La conferma la mostra il core (dialogo nativo), non la WebView: una pagina compromessa non può
//! dichiarare fidata una cartella da sola.
use crate::store::{Project, Result, Store};
use tauri::{AppHandle, Manager, State, WebviewWindow};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind, MessageDialogResult};

/// Codice d'errore dei comandi che eseguono codice in un progetto non fidato.
pub const UNTRUSTED: &str = "untrusted_folder";

pub const PROMPT: &str = "Ti fidi di questa cartella? Claude potrà eseguire comandi al suo interno senza chiedere";
const BTN_TRUST: &str = "Mi fido";
const BTN_READ_ONLY: &str = "Apri in sola consultazione";

pub fn is_trusted(store: &Store, folder: &str) -> bool {
    store.folder_trusted(&crate::projects::normalize(folder)).unwrap_or(false)
}

pub fn mark_trusted(store: &Store, folder: &str) -> Result<()> {
    store.trust_folder(&crate::projects::normalize(folder))
}

/// Cartelle del progetto non ancora fidate (nell'ordine del progetto).
pub fn untrusted_folders(store: &Store, project: &Project) -> Vec<String> {
    project.folders.iter().filter(|f| !is_trusted(store, f)).cloned().collect()
}

/// `Err(UNTRUSTED)` se il progetto non esiste o ha almeno una cartella non fidata.
pub fn ensure_project_trusted(store: &Store, project_id: &str) -> Result<()> {
    match store.get_project(project_id)? {
        Some(p) if !p.folders.is_empty() && untrusted_folders(store, &p).is_empty() => Ok(()),
        _ => Err(UNTRUSTED.into()),
    }
}

fn window_project(window: &WebviewWindow, store: &Store) -> Result<Project> {
    let id = window.label().strip_prefix(crate::projects::PROJECT_PREFIX).ok_or("questa finestra non è un progetto")?;
    store.get_project(id)?.ok_or_else(|| "progetto inesistente".into())
}

// ---------- comandi ----------

/// Cartelle del progetto della finestra chiamante non ancora fidate.
#[tauri::command]
pub fn trust_check(window: WebviewWindow, store: State<Store>) -> Result<Vec<String>> {
    Ok(untrusted_folders(&store, &window_project(&window, &store)?))
}

/// Chiede con un dialogo nativo (foglio della finestra) se fidarsi delle cartelle non fidate del progetto
/// della finestra chiamante. Vero se ora sono tutte fidate.
#[tauri::command]
pub async fn trust_accept(app: AppHandle, window: WebviewWindow) -> Result<bool> {
    let store = app.state::<Store>().inner().clone();
    let project = window_project(&window, &store)?;
    let pending = untrusted_folders(&store, &project);
    if pending.is_empty() {
        return Ok(true);
    }
    let message = format!("{PROMPT}\n\n{}", pending.join("\n"));
    let dialog = app
        .dialog()
        .message(message)
        .title(project.name.clone())
        .kind(MessageDialogKind::Warning)
        .buttons(MessageDialogButtons::OkCancelCustom(BTN_TRUST.into(), BTN_READ_ONLY.into()))
        .parent(&window);
    let answer = tauri::async_runtime::spawn_blocking(move || dialog.blocking_show_with_result())
        .await
        .map_err(|e| e.to_string())?;
    let trusted = matches!(&answer, MessageDialogResult::Ok | MessageDialogResult::Yes)
        || matches!(&answer, MessageDialogResult::Custom(l) if l == BTN_TRUST);
    if !trusted {
        return Ok(false);
    }
    for f in &pending {
        mark_trusted(&store, f)?;
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let s = Store::open(&dir.path().join("t.db")).unwrap();
        (dir, s)
    }

    #[test]
    fn unknown_folder_is_untrusted_until_marked() {
        let (_d, s) = store();
        assert!(!is_trusted(&s, "/Users/dev/progetto"));
        mark_trusted(&s, "/Users/dev/progetto/").unwrap();
        assert!(is_trusted(&s, "/Users/dev/progetto"));
        assert!(is_trusted(&s, "/Users/dev/progetto/"));
        // né i genitori né le sottocartelle ereditano la fiducia
        assert!(!is_trusted(&s, "/Users/dev"));
        assert!(!is_trusted(&s, "/Users/dev/progetto/sotto"));
        mark_trusted(&s, "/Users/dev/progetto").unwrap(); // idempotente
    }

    #[test]
    fn project_is_trusted_only_when_every_folder_is() {
        let (_d, s) = store();
        let p = Project { id: "p1".into(), name: "p".into(), folders: vec!["/Users/dev/a".into(), "/Users/dev/b".into()] };
        s.upsert_project(&p).unwrap();
        assert_eq!(ensure_project_trusted(&s, "p1").unwrap_err(), UNTRUSTED);
        mark_trusted(&s, "/Users/dev/a").unwrap();
        assert_eq!(untrusted_folders(&s, &p), vec!["/Users/dev/b".to_string()]);
        assert_eq!(ensure_project_trusted(&s, "p1").unwrap_err(), UNTRUSTED);
        mark_trusted(&s, "/Users/dev/b").unwrap();
        assert!(ensure_project_trusted(&s, "p1").is_ok());
        assert_eq!(ensure_project_trusted(&s, "nessuno").unwrap_err(), UNTRUSTED);
    }
}
