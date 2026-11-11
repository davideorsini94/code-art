//! Cartelle fidate: Claude (in bypass permissions) e il terminale eseguono codice, quindi partono solo in
//! cartelle che l'utente ha scelto con il selettore nativo o confermato con la domanda nativa.
//! La conferma la mostra il core (dialogo nativo), non la WebView: una pagina compromessa non può
//! dichiarare fidata una cartella da sola.
use crate::store::{Project, Result, Store};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager, State, WebviewWindow};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind, MessageDialogResult};

/// Codice d'errore dei comandi che eseguono codice in un progetto non fidato.
pub const UNTRUSTED: &str = "untrusted_folder";

pub const PROMPT: &str = "Ti fidi di questa cartella? Claude potrà eseguire comandi al suo interno senza chiedere";
/// Titolo fisso del dialogo: non il nome del progetto, che viene dal nome della cartella (non fidato).
const TITLE: &str = "Cartella non fidata";
const BTN_TRUST: &str = "Mi fido";
const BTN_READ_ONLY: &str = "Apri in sola consultazione";

/// Solo la scelta esplicita "Mi fido" vale come fiducia: pulsante predefinito (Invio), Esc, errore o etichetta
/// sconosciuta non la concedono.
fn is_trust_answer(r: &MessageDialogResult) -> bool {
    matches!(r, MessageDialogResult::Custom(l) if l == BTN_TRUST)
}

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

/// Dopo un rifiuto, per quanto tempo anche il "Mi fido" del banner viene ignorato.
pub const REFUSAL_COOLDOWN: Duration = Duration::from_secs(10);

/// Se mostrare la domanda di fiducia. `refused_at`: ultimo rifiuto per quel progetto in questo avvio;
/// `user_click`: richiesta dal "Mi fido" del banner (altrimenti è la domanda automatica all'apertura).
/// Dopo un rifiuto la domanda automatica non torna più in questo avvio, e il banner la riapre solo dopo
/// `REFUSAL_COOLDOWN`: una pagina compromessa non può sommergere l'utente di domande fino al "sì".
pub fn may_prompt(refused_at: Option<Instant>, now: Instant, user_click: bool) -> bool {
    match refused_at {
        None => true,
        Some(_) if !user_click => false,
        Some(t) => now.saturating_duration_since(t) >= REFUSAL_COOLDOWN,
    }
}

#[derive(Default)]
struct PromptState {
    refused_at: Option<Instant>,
    open: bool,
}

/// Stato delle domande di fiducia per progetto, solo in memoria (dura un avvio dell'app).
#[derive(Default)]
pub struct TrustPrompts(Mutex<HashMap<String, PromptState>>);

use crate::projects::window_project;

// ---------- comandi ----------

/// Cartelle del progetto della finestra chiamante non ancora fidate.
#[tauri::command]
pub fn trust_check(window: WebviewWindow, store: State<Store>) -> Result<Vec<String>> {
    Ok(untrusted_folders(&store, &window_project(&window, &store)?))
}

/// Chiede con un dialogo nativo (foglio della finestra) se fidarsi delle cartelle non fidate del progetto
/// della finestra chiamante. Vero se ora sono tutte fidate. `from_click`: richiesta dal banner (vedi `may_prompt`).
#[tauri::command]
pub async fn trust_accept(app: AppHandle, window: WebviewWindow, from_click: Option<bool>) -> Result<bool> {
    let store = app.state::<Store>().inner().clone();
    let project = window_project(&window, &store)?;
    let pending = untrusted_folders(&store, &project);
    if pending.is_empty() {
        return Ok(true);
    }
    {
        let prompts = app.state::<TrustPrompts>();
        let mut map = prompts.0.lock().unwrap_or_else(|p| p.into_inner());
        let st = map.entry(project.id.clone()).or_default();
        if st.open || !may_prompt(st.refused_at, Instant::now(), from_click.unwrap_or(false)) {
            return Ok(false);
        }
        st.open = true;
    }
    let message = format!("{PROMPT}\n\n{}", pending.join("\n"));
    let dialog = app
        .dialog()
        .message(message)
        .title(TITLE)
        .kind(MessageDialogKind::Warning)
        .buttons(MessageDialogButtons::OkCancelCustom(BTN_READ_ONLY.into(), BTN_TRUST.into()))
        .parent(&window);
    let answer = tauri::async_runtime::spawn_blocking(move || dialog.blocking_show_with_result()).await;
    // "Apri in sola consultazione" è il primo pulsante, quindi quello predefinito (Invio) di NSAlert.
    let trusted = matches!(&answer, Ok(r) if is_trust_answer(r));
    {
        let prompts = app.state::<TrustPrompts>();
        let mut map = prompts.0.lock().unwrap_or_else(|p| p.into_inner());
        let st = map.entry(project.id.clone()).or_default();
        st.open = false;
        st.refused_at = if trusted { None } else { Some(Instant::now()) };
    }
    answer.map_err(|e| e.to_string())?;
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
    fn only_the_explicit_trust_button_grants_trust() {
        use MessageDialogResult::*;
        for (r, want) in [
            (Custom(BTN_TRUST.into()), true),
            (Custom(BTN_READ_ONLY.into()), false),
            (Ok, false),
            (Yes, false),
            (No, false),
            (Cancel, false),
            (Custom("altro".into()), false),
            (Custom(String::new()), false),
        ] {
            assert_eq!(is_trust_answer(&r), want, "{r:?}");
        }
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
    fn refusal_stops_automatic_prompts_and_cools_down_the_banner() {
        let t0 = Instant::now();
        assert!(may_prompt(None, t0, false));
        assert!(may_prompt(None, t0, true));
        // dopo un rifiuto: mai più la domanda automatica in questo avvio
        assert!(!may_prompt(Some(t0), t0 + Duration::from_secs(3600), false));
        // il banner: ignorato entro 10 s, poi di nuovo ammesso
        assert!(!may_prompt(Some(t0), t0, true));
        assert!(!may_prompt(Some(t0), t0 + Duration::from_millis(9_999), true));
        assert!(may_prompt(Some(t0), t0 + REFUSAL_COOLDOWN, true));
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
