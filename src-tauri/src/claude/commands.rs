//! Comandi Tauri del motore delle conversazioni: involucri sottili attorno a `Engine`, eseguiti fuori dal
//! thread principale e confinati al progetto della finestra chiamante.
use super::engine::{ClaudeCheck, Engine, StartOpts};
use super::protocol::ClaudeEvent;
use crate::projects::window_project_id;
use crate::store::Result;
use tauri::{AppHandle, Manager, WebviewWindow};

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

/// Controlla che i primi byte siano davvero quelli del formato dichiarato.
fn image_magic_ok(ext: &str, b: &[u8]) -> bool {
    match ext {
        "png" => b.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]),
        "jpg" | "jpeg" => b.starts_with(&[0xFF, 0xD8, 0xFF]),
        "gif" => b.starts_with(b"GIF87a") || b.starts_with(b"GIF89a"),
        "webp" => b.len() >= 12 && &b[..4] == b"RIFF" && &b[8..12] == b"WEBP",
        _ => false,
    }
}

/// Valida e scrive un'immagine incollata in `dir` (creata con 0700) con nome casuale; ritorna il percorso.
/// Estensioni accettate: le stesse di `Engine::load_image`. Cancella (best effort) i file più vecchi di 24 ore.
fn save_image_in(dir: &std::path::Path, bytes: &[u8], ext: &str) -> Result<String> {
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
    let ext = ext.trim_start_matches('.').to_ascii_lowercase();
    if !matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "gif" | "webp") {
        return Err(format!("formato immagine non supportato: {ext}"));
    }
    if bytes.is_empty() || bytes.len() as u64 > super::engine::IMAGE_MAX_BYTES {
        return Err(format!("immagine vuota o troppo grande (al massimo {} MB)", super::engine::IMAGE_MAX_BYTES / (1024 * 1024)));
    }
    if !image_magic_ok(&ext, bytes) {
        return Err("il contenuto non corrisponde al formato dichiarato".into());
    }
    std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir).map_err(|e| e.to_string())?;
    let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
    if let Ok(rd) = std::fs::read_dir(dir) {
        let limit = std::time::Duration::from_secs(24 * 3600);
        for e in rd.flatten() {
            let old = e.metadata().ok().filter(|m| m.is_file()).and_then(|m| m.modified().ok()).and_then(|t| t.elapsed().ok()).is_some_and(|age| age > limit);
            if old {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
    let path = dir.join(format!("{}.{ext}", uuid::Uuid::new_v4()));
    let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&path).map_err(|e| e.to_string())?;
    std::io::Write::write_all(&mut f, bytes).map_err(|e| e.to_string())?;
    Ok(path.to_string_lossy().into_owned())
}

/// Salva un'immagine incollata/trascinata nella cartella temporanea dell'app; ritorna il percorso da passare a `conv_send`.
/// I byte arrivano come corpo binario della richiesta (niente array JSON); l'estensione nell'header `ext`.
#[tauri::command]
pub async fn conv_save_image(window: WebviewWindow, request: tauri::ipc::Request<'_>) -> Result<String> {
    window_project_id(&window)?; // solo le finestre di progetto
    let tauri::ipc::InvokeBody::Raw(bytes) = request.body() else {
        return Err("corpo della richiesta non binario".into());
    };
    if bytes.len() as u64 > super::engine::IMAGE_MAX_BYTES {
        return Err("immagine troppo grande".into());
    }
    let bytes = bytes.clone();
    let ext = request.headers().get("ext").and_then(|v| v.to_str().ok()).unwrap_or_default().to_string();
    tauri::async_runtime::spawn_blocking(move || save_image_in(&super::engine::image_dir(), &bytes, &ext)).await.map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    const PNG: &[u8] = &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 0];

    #[test]
    fn valid_png_is_saved_with_random_name_and_private_dir() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("images");
        let p = save_image_in(&dir, PNG, "PNG").unwrap();
        assert!(p.ends_with(".png") && p.starts_with(dir.to_str().unwrap()));
        assert_eq!(std::fs::read(&p).unwrap(), PNG);
        assert_eq!(std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777, 0o700);
        assert_ne!(p, save_image_in(&dir, PNG, "png").unwrap());
    }

    #[test]
    fn wrong_magic_bad_extension_and_oversize_are_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(save_image_in(tmp.path(), b"not an image at all", "png").is_err());
        assert!(save_image_in(tmp.path(), PNG, "jpg").is_err());
        assert!(save_image_in(tmp.path(), PNG, "svg").is_err());
        assert!(save_image_in(tmp.path(), PNG, "../x").is_err());
        let mut big = PNG.to_vec();
        big.resize(super::super::engine::IMAGE_MAX_BYTES as usize + 1, 0);
        assert!(save_image_in(tmp.path(), &big, "png").is_err());
        assert_eq!(std::fs::read_dir(tmp.path()).unwrap().count(), 0);
    }

    #[test]
    fn old_files_are_removed() {
        let tmp = tempfile::tempdir().unwrap();
        let old = tmp.path().join("old.png");
        std::fs::write(&old, PNG).unwrap();
        let t = std::time::SystemTime::now() - std::time::Duration::from_secs(25 * 3600);
        std::fs::File::options().write(true).open(&old).unwrap().set_modified(t).unwrap();
        save_image_in(tmp.path(), PNG, "png").unwrap();
        assert!(!old.exists());
    }
}
