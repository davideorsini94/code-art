//! Comandi Tauri del motore delle conversazioni: involucri sottili attorno a `Engine`, eseguiti fuori dal
//! thread principale e confinati al progetto della finestra chiamante.
use super::engine::{ClaudeCheck, Engine, StartOpts};
use super::protocol::ClaudeEvent;
use crate::store::Result;
use tauri::{AppHandle, Manager, WebviewWindow};

fn window_project_id(window: &WebviewWindow) -> Result<String> {
    window
        .label()
        .strip_prefix(crate::projects::PROJECT_PREFIX)
        .map(String::from)
        .ok_or_else(|| "questa finestra non è un progetto".into())
}

/// Esegue `f` fuori dal thread principale con il motore e il progetto della finestra chiamante.
async fn with_engine<T: Send + 'static>(
    app: AppHandle,
    window: WebviewWindow,
    f: impl FnOnce(&Engine, &str) -> Result<T> + Send + 'static,
) -> Result<T> {
    let project_id = window_project_id(&window)?;
    let engine = app.state::<Engine>().inner().clone();
    tauri::async_runtime::spawn_blocking(move || f(&engine, &project_id)).await.map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn claude_check(app: AppHandle) -> Result<ClaudeCheck> {
    let engine = app.state::<Engine>().inner().clone();
    tauri::async_runtime::spawn_blocking(move || engine.claude_check()).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn conv_start(app: AppHandle, window: WebviewWindow, project_id: String, opts: Option<StartOpts>) -> Result<String> {
    with_engine(app, window, move |e, pid| {
        if pid != project_id {
            return Err("progetto non della finestra chiamante".into());
        }
        e.start(pid, opts.unwrap_or_default())
    })
    .await
}

#[tauri::command]
pub async fn conv_resume(app: AppHandle, window: WebviewWindow, session_id: String) -> Result<()> {
    with_engine(app, window, move |e, pid| e.resume(pid, &session_id)).await
}

#[tauri::command]
pub async fn conv_send(app: AppHandle, window: WebviewWindow, session_id: String, text: String, image_paths: Vec<String>) -> Result<()> {
    with_engine(app, window, move |e, pid| e.send(pid, &session_id, &text, &image_paths)).await
}

#[tauri::command]
pub async fn conv_answer(app: AppHandle, window: WebviewWindow, session_id: String, request_id: String, choice: String) -> Result<()> {
    with_engine(app, window, move |e, pid| e.answer(pid, &session_id, &request_id, &choice)).await
}

#[tauri::command]
pub async fn conv_interrupt(app: AppHandle, window: WebviewWindow, session_id: String) -> Result<()> {
    with_engine(app, window, move |e, pid| e.interrupt(pid, &session_id)).await
}

#[tauri::command]
pub async fn conv_set_model(app: AppHandle, window: WebviewWindow, session_id: String, model: String) -> Result<()> {
    with_engine(app, window, move |e, pid| e.set_model(pid, &session_id, &model)).await
}

#[tauri::command]
pub async fn conv_set_permission_mode(app: AppHandle, window: WebviewWindow, session_id: String, mode: String) -> Result<()> {
    with_engine(app, window, move |e, pid| e.set_permission_mode(pid, &session_id, &mode)).await
}

#[tauri::command]
pub async fn conv_mark_viewed(app: AppHandle, window: WebviewWindow, session_id: String) -> Result<()> {
    with_engine(app, window, move |e, pid| e.mark_viewed(pid, &session_id)).await
}

#[tauri::command]
pub async fn conv_archive(app: AppHandle, window: WebviewWindow, session_id: String, archived: bool) -> Result<()> {
    with_engine(app, window, move |e, pid| e.archive(pid, &session_id, archived)).await
}

#[tauri::command]
pub async fn conv_running(app: AppHandle, window: WebviewWindow, project_id: String) -> Result<Vec<String>> {
    with_engine(app, window, move |e, pid| {
        if pid != project_id {
            return Err("progetto non della finestra chiamante".into());
        }
        Ok(e.running(pid))
    })
    .await
}

/// Richieste di permesso/domanda/piano ancora aperte (eventi `Ask`, dalla più vecchia).
#[tauri::command]
pub async fn conv_pending(app: AppHandle, window: WebviewWindow, session_id: String) -> Result<Vec<ClaudeEvent>> {
    with_engine(app, window, move |e, pid| e.pending_asks(pid, &session_id)).await
}

#[tauri::command]
pub async fn conv_stop(app: AppHandle, window: WebviewWindow, session_id: String) -> Result<()> {
    with_engine(app, window, move |e, pid| e.stop_owned(pid, &session_id)).await
}
