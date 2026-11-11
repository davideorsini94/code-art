//! Ciclo di vita: chiusura finestre e uscita con conversazioni al lavoro, notifiche macOS e numero sul Dock.
//!
//! La logica di decisione è in funzioni pure (testate); il resto collega Tauri. Ogni cambio di stato delle
//! conversazioni passa da `LifecycleSink`, che avvolge il sink degli eventi del motore.
use crate::claude::engine::{Engine, EventSink};
use crate::claude::{ConvStatus, Conversation};
use crate::projects::{self, Quitting, PROJECT_PREFIX};
use crate::store::Store;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use tauri::{AppHandle, Emitter, Manager, Window};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind, MessageDialogResult};
use tauri_plugin_notification::{NotificationExt, PermissionState};

const BTN_BACKGROUND: &str = "Lascia finire in sottofondo";
const BTN_INTERRUPT: &str = "Interrompi";
const BTN_CANCEL: &str = "Annulla";
const BTN_QUIT: &str = "Esci";
const QUIT_MESSAGE: &str = "Ci sono conversazioni al lavoro. Uscire le interromperà.";
const CLOSE_MESSAGE: &str = "In questo progetto ci sono conversazioni al lavoro.";
/// Notifiche ricordate per il click (le più vecchie si scartano).
const NOTIFICATIONS_KEPT: usize = 64;

// ---------- logica pura ----------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseAction {
    CloseNow,
    Ask,
}

/// Chiudere/uscire: si chiede conferma solo se qualche conversazione sta lavorando.
pub fn decide_close(running: usize) -> CloseAction {
    if running == 0 { CloseAction::CloseNow } else { CloseAction::Ask }
}

/// Testo del numero sul Dock: nessun numero a zero.
pub fn badge_text(waiting_total: usize) -> Option<String> {
    (waiting_total > 0).then(|| waiting_total.to_string())
}

/// Notifica solo quando si passa a `Waiting` da un altro stato e l'utente non sta guardando la finestra.
pub fn should_notify(prev: ConvStatus, next: ConvStatus, window_focused: bool) -> bool {
    next == ConvStatus::Waiting && prev != ConvStatus::Waiting && !window_focused
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClosePlan {
    /// Processi vivi senza turno in corso: si fermano comunque alla chiusura della finestra.
    pub stop: Vec<String>,
    /// Processi vivi con un turno in corso (`working`).
    pub working: Vec<String>,
    /// Serve la domanda "sottofondo / interrompi / annulla".
    pub ask: bool,
}

/// Piano di chiusura di una finestra a partire da `(sessionId, stato, processo vivo)` delle conversazioni
/// del suo progetto. Le conversazioni senza processo non contano.
pub fn plan_window_close(convs: &[(String, ConvStatus, bool)]) -> ClosePlan {
    let mut stop = Vec::new();
    let mut working = Vec::new();
    for (id, status, running) in convs {
        if !running {
            continue;
        }
        if *status == ConvStatus::Working { working.push(id.clone()) } else { stop.push(id.clone()) }
    }
    let ask = decide_close(working.len()) == CloseAction::Ask;
    ClosePlan { stop, working, ask }
}

fn status_of(s: &str) -> ConvStatus {
    match s {
        "working" => ConvStatus::Working,
        "waiting" => ConvStatus::Waiting,
        _ => ConvStatus::Idle,
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

// ---------- stato ----------

/// Cosa è cambiato rispetto all'ultimo upsert visto della stessa conversazione.
#[derive(Debug, PartialEq, Eq)]
pub struct Observed {
    /// Stato o archiviazione diversi (o conversazione mai vista).
    pub changed: bool,
    /// Stato precedente, se la conversazione era già stata vista in questa esecuzione.
    pub prev: Option<ConvStatus>,
}

#[derive(Default)]
pub struct Lifecycle {
    seen: Mutex<HashMap<String, (ConvStatus, bool)>>,
    /// Conversazioni notificate e non ancora viste: sessionId -> projectId.
    pending: Mutex<HashMap<String, String>>,
    /// id notifica -> (sessionId, projectId), per il click.
    notifications: Mutex<Vec<(i32, String, String)>>,
    /// Sessioni che stiamo fermando noi: il passaggio a `waiting` non va notificato.
    silenced: Mutex<HashSet<String>>,
    next_id: AtomicI32,
    permission: Mutex<Option<bool>>,
    /// Serializza calcolo e invio del numero sul Dock.
    badge: Mutex<()>,
}

impl Lifecycle {
    pub fn observe(&self, c: &Conversation) -> Observed {
        let st = status_of(&c.status);
        let old = lock(&self.seen).insert(c.session_id.clone(), (st, c.archived));
        if st != ConvStatus::Waiting {
            lock(&self.pending).remove(&c.session_id);
        }
        Observed { changed: old.is_none_or(|(s, a)| s != st || a != c.archived), prev: old.map(|(s, _)| s) }
    }

    fn remember_notification(&self, session_id: &str, project_id: &str) -> i32 {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst) + 1;
        let mut list = lock(&self.notifications);
        list.push((id, session_id.into(), project_id.into()));
        if list.len() > NOTIFICATIONS_KEPT {
            list.remove(0);
        }
        lock(&self.pending).insert(session_id.into(), project_id.into());
        id
    }

    fn notification_target(&self, id: i32) -> Option<(String, String)> {
        lock(&self.notifications).iter().find(|(n, ..)| *n == id).map(|(_, s, p)| (s.clone(), p.clone()))
    }

    /// L'utente ha portato in primo piano la finestra del progetto: le sue notifiche sono viste.
    pub fn clear_project(&self, project_id: &str) {
        lock(&self.pending).retain(|_, p| p != project_id);
    }

    /// L'unica conversazione notificata e ancora in attesa, se ce n'è esattamente una.
    pub fn single_pending(&self) -> Option<(String, String)> {
        let p = lock(&self.pending);
        if p.len() == 1 { p.iter().next().map(|(s, pr)| (s.clone(), pr.clone())) } else { None }
    }

    fn silence(&self, ids: &[String]) {
        lock(&self.silenced).extend(ids.iter().cloned());
    }

    fn unsilence(&self, ids: &[String]) {
        let mut s = lock(&self.silenced);
        for id in ids {
            s.remove(id);
        }
    }

    fn is_silenced(&self, id: &str) -> bool {
        lock(&self.silenced).contains(id)
    }

    /// Permesso notifiche: richiesto una volta, al primo bisogno. Se negato, niente notifiche (il numero sul
    /// Dock resta).
    fn permission_granted(&self, app: &AppHandle) -> bool {
        let mut p = lock(&self.permission);
        *p.get_or_insert_with(|| matches!(app.notification().request_permission(), Ok(PermissionState::Granted)))
    }
}

// ---------- integrazione con gli eventi del motore ----------

/// Avvolge il sink degli eventi: tutto passa all'interno, poi ogni `conv:upsert` aggiorna Dock e notifiche.
pub struct LifecycleSink {
    pub inner: Arc<dyn EventSink>,
    pub app: AppHandle,
    pub life: Arc<Lifecycle>,
    pub store: Store,
}

impl EventSink for LifecycleSink {
    fn emit(&self, project_id: &str, event: &str, payload: Value) {
        if event != "conv:upsert" {
            return self.inner.emit(project_id, event, payload);
        }
        let conv = serde_json::from_value::<Conversation>(payload.clone());
        self.inner.emit(project_id, event, payload);
        let Ok(conv) = conv else { return };
        let obs = self.life.observe(&conv);
        if !obs.changed {
            return;
        }
        let (app, life, store) = (self.app.clone(), self.life.clone(), self.store.clone());
        // fuori dal thread del motore: la finestra e il Dock si interrogano altrove
        std::thread::spawn(move || {
            update_badge(&app, &life, &store);
            if let Some(prev) = obs.prev {
                maybe_notify(&app, &life, &store, &conv, prev);
            }
        });
    }
}

fn window_focused(app: &AppHandle, project_id: &str) -> bool {
    app.get_webview_window(&projects::window_label(project_id)).and_then(|w| w.is_focused().ok()).unwrap_or(false)
}

fn quitting(app: &AppHandle) -> bool {
    app.state::<Quitting>().0.load(Ordering::SeqCst)
}

fn maybe_notify(app: &AppHandle, life: &Lifecycle, store: &Store, c: &Conversation, prev: ConvStatus) {
    if c.archived || quitting(app) || life.is_silenced(&c.session_id) {
        return;
    }
    if !should_notify(prev, status_of(&c.status), window_focused(app, &c.project_id)) {
        return;
    }
    let Ok(Some(project)) = store.get_project(&c.project_id) else { return };
    if !life.permission_granted(app) {
        return;
    }
    let title = if c.title.trim().is_empty() { "Conversazione" } else { c.title.trim() };
    let id = life.remember_notification(&c.session_id, &c.project_id);
    // solo testo semplice dal nostro archivio: titolo della conversazione e nome del progetto
    let _ = app.notification().builder().id(id).title(title).body(&project.name).show();
}

/// Registra il click sulle notifiche (`tap`): apre il progetto e seleziona la conversazione.
pub fn register_notification_clicks(app: &AppHandle, life: Arc<Lifecycle>) {
    let handle = app.clone();
    let _ = app.notification().on_action(move |performed| {
        if performed.action_id() != "tap" {
            return;
        }
        let target = performed.notification().and_then(|n| life.notification_target(n.id()));
        if let Some((session_id, project_id)) = target {
            focus_conversation(&handle, &life, &session_id, &project_id);
        }
    });
}

/// Apre o porta in primo piano la finestra del progetto e le chiede di selezionare la conversazione.
pub fn focus_conversation(app: &AppHandle, life: &Lifecycle, session_id: &str, project_id: &str) {
    let Ok(Some(project)) = app.state::<Store>().get_project(project_id) else { return };
    let label = projects::window_label(project_id);
    let existed = app.get_webview_window(&label).is_some();
    if projects::show_project_window(app, &project).is_err() {
        return;
    }
    lock(&life.pending).remove(session_id);
    let payload = json!({ "sessionId": session_id });
    let _ = app.emit_to(label.as_str(), "conv:focus", payload.clone());
    if !existed {
        // finestra appena creata: l'interfaccia non è ancora in ascolto. La selezione è idempotente.
        let app = app.clone();
        std::thread::spawn(move || {
            for ms in [800u64, 2000] {
                std::thread::sleep(std::time::Duration::from_millis(ms));
                let _ = app.emit_to(label.as_str(), "conv:focus", payload.clone());
            }
        });
    }
}

/// Icona del Dock cliccata (`RunEvent::Reopen`) o app riattivata: se una sola conversazione notificata aspetta,
/// si va a lei (ripiego quando il click sulla notifica non arriva); altrimenti, senza finestre, si riapre il
/// progetto usato più di recente (o la finestra di benvenuto).
pub fn on_reopen(app: &AppHandle, has_visible_windows: bool) {
    let life = app.state::<Arc<Lifecycle>>();
    if let Some((session_id, project_id)) = life.single_pending() {
        return focus_conversation(app, &life, &session_id, &project_id);
    }
    if has_visible_windows {
        return;
    }
    match app.state::<Store>().list_projects().ok().and_then(|l| l.into_iter().next()) {
        Some(p) => {
            let _ = projects::show_project_window(app, &p);
        }
        None => {
            let _ = projects::show_welcome_window(app);
        }
    }
}

/// Un'altra finestra del progetto è passata in primo piano.
pub fn on_window_focused(app: &AppHandle, label: &str) {
    if let Some(id) = label.strip_prefix(PROJECT_PREFIX) {
        app.state::<Arc<Lifecycle>>().clear_project(id);
    }
}

// ---------- Dock ----------

/// Numero sul Dock = conversazioni `waiting` non archiviate di tutti i progetti.
///
/// Si usa direttamente `NSApp.dockTile` (sul thread principale): l'API di Tauri (`set_badge_label`) passa da
/// una finestra, che quando l'ultima è chiusa non c'è.
pub fn update_badge(app: &AppHandle, life: &Lifecycle, store: &Store) {
    let _order = lock(&life.badge);
    let label = badge_text(store.waiting_total().unwrap_or(0));
    let _ = app.run_on_main_thread(move || {
        use objc2::MainThreadMarker;
        use objc2_app_kit::NSApplication;
        use objc2_foundation::NSString;
        if let Some(mtm) = MainThreadMarker::new() {
            let tile = NSApplication::sharedApplication(mtm).dockTile();
            let text = label.as_deref().map(NSString::from_str);
            tile.setBadgeLabel(text.as_deref());
        }
    });
}

// ---------- chiusura finestra ----------

/// Ferma in sottofondo (mai sul thread dell'interfaccia) i processi indicati, senza notificare il passaggio a
/// `waiting` che ne deriva.
fn stop_in_background(app: &AppHandle, ids: Vec<String>) {
    if ids.is_empty() {
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        let life = app.state::<Arc<Lifecycle>>();
        life.silence(&ids);
        let engine = app.state::<Engine>();
        let engine = &*engine;
        std::thread::scope(|s| {
            for id in &ids {
                s.spawn(move || engine.stop(id));
            }
        });
        life.unsilence(&ids);
    });
}

fn project_plan(app: &AppHandle, project_id: &str) -> ClosePlan {
    let live: HashSet<String> = app.state::<Engine>().running(project_id).into_iter().collect();
    let rows: Vec<(String, ConvStatus, bool)> = app
        .state::<Store>()
        .list_conversations(project_id)
        .unwrap_or_default()
        .into_iter()
        .map(|c| (c.session_id.clone(), status_of(&c.status), live.contains(&c.session_id)))
        .collect();
    plan_window_close(&rows)
}

/// `CloseRequested` di una finestra progetto. Senza lavoro in corso la finestra si chiude e i processi si
/// fermano in sottofondo; altrimenti si chiede cosa fare. Durante l'uscita dall'app non si interviene.
pub fn on_close_requested(window: &Window, api: &tauri::CloseRequestApi) {
    let Some(project_id) = window.label().strip_prefix(PROJECT_PREFIX) else { return };
    let app = window.app_handle();
    if quitting(app) {
        return;
    }
    let plan = project_plan(app, project_id);
    if !plan.ask {
        return stop_in_background(app, plan.stop);
    }
    api.prevent_close();
    let (win, handle) = (window.clone(), app.clone());
    app.dialog()
        .message(CLOSE_MESSAGE)
        .title("Conversazioni al lavoro")
        .kind(MessageDialogKind::Warning)
        .buttons(MessageDialogButtons::YesNoCancelCustom(BTN_BACKGROUND.into(), BTN_INTERRUPT.into(), BTN_CANCEL.into()))
        .show_with_result(move |r| {
            let label = match &r {
                MessageDialogResult::Custom(l) => l.as_str(),
                MessageDialogResult::Yes => BTN_BACKGROUND,
                MessageDialogResult::No => BTN_INTERRUPT,
                _ => BTN_CANCEL,
            };
            match label {
                BTN_BACKGROUND => {
                    let _ = win.destroy();
                    stop_in_background(&handle, plan.stop);
                }
                BTN_INTERRUPT => {
                    let _ = win.destroy();
                    stop_in_background(&handle, [plan.stop, plan.working].concat());
                }
                _ => {}
            }
        });
}

// ---------- uscita ----------

fn working_total(app: &AppHandle) -> usize {
    let store = app.state::<Store>();
    app.state::<Engine>()
        .live_ids()
        .iter()
        .filter(|id| matches!(store.get_conversation(id), Ok(Some(c)) if status_of(&c.status) == ConvStatus::Working))
        .count()
}

/// Esce davvero: da qui in poi le finestre si chiudono senza domande e `was_open` resta com'è.
fn quit_now(app: &AppHandle) {
    projects::mark_quitting(app);
    let app = app.clone();
    std::thread::spawn(move || {
        app.state::<Engine>().stop_all();
        app.exit(0);
    });
}

/// ⌘Q / "Esci da Code Art": con conversazioni al lavoro in qualunque progetto chiede conferma.
pub fn request_quit(app: &AppHandle) {
    if decide_close(working_total(app)) == CloseAction::CloseNow {
        return quit_now(app);
    }
    let handle = app.clone();
    app.dialog()
        .message(QUIT_MESSAGE)
        .title("Uscire da Code Art?")
        .kind(MessageDialogKind::Warning)
        .buttons(MessageDialogButtons::OkCancelCustom(BTN_QUIT.into(), BTN_CANCEL.into()))
        .show_with_result(move |r| {
            if matches!(&r, MessageDialogResult::Ok | MessageDialogResult::Yes)
                || matches!(&r, MessageDialogResult::Custom(l) if l == BTN_QUIT)
            {
                quit_now(&handle);
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use ConvStatus::*;

    #[test]
    fn decide_close_asks_only_with_running_turns() {
        assert_eq!(decide_close(0), CloseAction::CloseNow);
        assert_eq!(decide_close(1), CloseAction::Ask);
        assert_eq!(decide_close(2), CloseAction::Ask);
    }

    #[test]
    fn badge_hidden_at_zero() {
        assert_eq!(badge_text(0), None);
        assert_eq!(badge_text(3), Some("3".to_string()));
    }

    #[test]
    fn notify_only_on_transition_to_waiting_with_window_not_focused() {
        for prev in [Working, Waiting, Idle] {
            for next in [Working, Waiting, Idle] {
                for focused in [false, true] {
                    let expected = next == Waiting && prev != Waiting && !focused;
                    assert_eq!(should_notify(prev, next, focused), expected, "{prev:?} -> {next:?} focused={focused}");
                }
            }
        }
        assert!(should_notify(Working, Waiting, false));
        assert!(!should_notify(Working, Waiting, true));
        assert!(!should_notify(Waiting, Waiting, false));
    }

    fn row(id: &str, s: ConvStatus, running: bool) -> (String, ConvStatus, bool) {
        (id.into(), s, running)
    }

    #[test]
    fn close_plan_without_work_stops_idle_processes_and_closes() {
        let plan = plan_window_close(&[row("a", Idle, true), row("b", Waiting, true), row("c", Idle, false)]);
        assert_eq!(plan, ClosePlan { stop: vec!["a".into(), "b".into()], working: vec![], ask: false });
    }

    #[test]
    fn close_plan_asks_when_a_turn_is_running() {
        let plan = plan_window_close(&[row("a", Working, true), row("b", Idle, true), row("c", Waiting, true)]);
        assert!(plan.ask);
        assert_eq!(plan.working, vec!["a".to_string()]);
        assert_eq!(plan.stop, vec!["b".to_string(), "c".to_string()]);
    }

    #[test]
    fn close_plan_ignores_conversations_without_a_process() {
        // `working` salvato ma senza processo (non dovrebbe succedere): niente domanda
        let plan = plan_window_close(&[row("a", Working, false)]);
        assert_eq!(plan, ClosePlan { stop: vec![], working: vec![], ask: false });
        assert_eq!(plan_window_close(&[]).ask, false);
    }

    fn conv(id: &str, status: &str, archived: bool) -> Conversation {
        Conversation {
            session_id: id.into(),
            project_id: "p".into(),
            cwd: "/Users/dev/x".into(),
            title: "t".into(),
            prev_title: None,
            title_changed_at: None,
            title_locked: false,
            summary: String::new(),
            status: status.into(),
            owned: true,
            archived,
            last_activity: 0,
            turn_count: 0,
            model: None,
            permission_mode: "default".into(),
            context_used_pct: None,
            maybe_open_elsewhere: false,
        }
    }

    #[test]
    fn observe_reports_transitions_and_ignores_metadata_only_changes() {
        let l = Lifecycle::default();
        assert_eq!(l.observe(&conv("a", "idle", false)), Observed { changed: true, prev: None });
        assert_eq!(l.observe(&conv("a", "working", false)), Observed { changed: true, prev: Some(Idle) });
        assert_eq!(l.observe(&conv("a", "working", false)), Observed { changed: false, prev: Some(Working) });
        assert_eq!(l.observe(&conv("a", "waiting", false)), Observed { changed: true, prev: Some(Working) });
        // archiviare cambia il numero sul Dock anche a stato invariato
        assert_eq!(l.observe(&conv("a", "waiting", true)), Observed { changed: true, prev: Some(Waiting) });
    }

    #[test]
    fn pending_notification_is_single_and_cleared_when_seen_or_resolved() {
        let l = Lifecycle::default();
        assert_eq!(l.single_pending(), None);
        let id = l.remember_notification("a", "p1");
        assert_eq!(l.notification_target(id), Some(("a".into(), "p1".into())));
        assert_eq!(l.single_pending(), Some(("a".into(), "p1".into())));
        l.remember_notification("b", "p2");
        assert_eq!(l.single_pending(), None, "due in attesa: nessuna scelta automatica");
        l.clear_project("p2");
        assert_eq!(l.single_pending(), Some(("a".into(), "p1".into())));
        l.observe(&conv("a", "idle", false));
        assert_eq!(l.single_pending(), None, "uscita da waiting: non più in attesa");
    }

    #[test]
    fn remembered_notifications_are_bounded() {
        let l = Lifecycle::default();
        let first = l.remember_notification("a", "p");
        for _ in 0..NOTIFICATIONS_KEPT {
            l.remember_notification("a", "p");
        }
        assert_eq!(l.notification_target(first), None);
    }
}
