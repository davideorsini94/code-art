//! Terminale integrato: PTY per finestra. Ogni PTY appartiene alla finestra che l'ha creata.
use crate::fs_tree::{ensure_inside, roots_of};
use crate::store::Store;
use base64::Engine;
use portable_pty::{native_pty_system, ChildKiller, CommandBuilder, MasterPty, PtySize};
use serde::Serialize;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, State, WebviewWindow};

const MAX_PER_WINDOW: usize = 16;
const MAX_WRITE_BYTES: usize = 1024 * 1024;
const MAX_DIM: u16 = 1000;

struct Session {
    owner: String,
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    killer: Box<dyn ChildKiller + Send + Sync>,
}

#[derive(Default)]
pub struct Ptys {
    next: AtomicU32,
    sessions: Arc<Mutex<HashMap<u32, Session>>>,
}

fn size(cols: u16, rows: u16) -> Result<PtySize, String> {
    if cols == 0 || rows == 0 || cols > MAX_DIM || rows > MAX_DIM {
        return Err("dimensioni del terminale non valide".into());
    }
    Ok(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })
}

/// Shell di login dell'utente: `$SHELL` se è un file assoluto esistente, altrimenti `/bin/zsh`.
fn login_shell() -> String {
    match std::env::var("SHELL") {
        Ok(s) if Path::new(&s).is_absolute() && Path::new(&s).is_file() => s,
        _ => "/bin/zsh".to_string(),
    }
}

impl Ptys {
    /// Avvia `program args` in `cwd`. `on_data` riceve i byte grezzi; `on_exit` viene chiamata una sola volta.
    #[allow(clippy::too_many_arguments)]
    pub fn spawn(
        &self,
        owner: &str,
        program: &str,
        args: &[&str],
        cwd: &Path,
        cols: u16,
        rows: u16,
        on_data: impl Fn(u32, &[u8]) + Send + 'static,
        on_exit: impl FnOnce(u32) + Send + 'static,
    ) -> Result<u32, String> {
        let size = size(cols, rows)?;
        if self.sessions.lock().unwrap().values().filter(|s| s.owner == owner).count() >= MAX_PER_WINDOW {
            return Err("troppi terminali aperti".into());
        }
        let pair = native_pty_system().openpty(size).map_err(|e| e.to_string())?;
        let mut cmd = CommandBuilder::new(program);
        cmd.args(args);
        cmd.cwd(cwd);
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        let mut child = pair.slave.spawn_command(cmd).map_err(|e| e.to_string())?;
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().map_err(|e| e.to_string())?;
        let writer = pair.master.take_writer().map_err(|e| e.to_string())?;
        let id = self.next.fetch_add(1, Ordering::SeqCst) + 1;
        let session = Session { owner: owner.to_string(), master: pair.master, writer, killer: child.clone_killer() };
        self.sessions.lock().unwrap().insert(id, session);

        let sessions = self.sessions.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 8192];
            while let Ok(n) = reader.read(&mut buf) {
                if n == 0 {
                    break;
                }
                on_data(id, &buf[..n]);
            }
            let _ = child.wait();
            sessions.lock().unwrap().remove(&id);
            on_exit(id);
        });
        Ok(id)
    }

    fn with_owned<T>(&self, owner: &str, id: u32, f: impl FnOnce(&mut Session) -> T) -> Result<T, String> {
        let mut map = self.sessions.lock().unwrap();
        match map.get_mut(&id) {
            Some(s) if s.owner == owner => Ok(f(s)),
            _ => Err("terminale inesistente".into()), // anche se di un'altra finestra: stessa risposta
        }
    }

    pub fn write(&self, owner: &str, id: u32, data: &[u8]) -> Result<(), String> {
        if data.len() > MAX_WRITE_BYTES {
            return Err("dati troppo grandi".into());
        }
        self.with_owned(owner, id, |s| s.writer.write_all(data).and_then(|_| s.writer.flush()).map_err(|e| e.to_string()))?
    }

    pub fn resize(&self, owner: &str, id: u32, cols: u16, rows: u16) -> Result<(), String> {
        let size = size(cols, rows)?;
        self.with_owned(owner, id, |s| s.master.resize(size).map_err(|e| e.to_string()))?
    }

    pub fn kill(&self, owner: &str, id: u32) -> Result<(), String> {
        self.with_owned(owner, id, |s| s.killer.kill().map_err(|e| e.to_string()))?
    }

    /// Chiude tutti i PTY di una finestra (alla sua distruzione).
    pub fn kill_owner(&self, owner: &str) {
        for s in self.sessions.lock().unwrap().values_mut().filter(|s| s.owner == owner) {
            let _ = s.killer.kill();
        }
    }
}

// ---------- comandi ----------

#[derive(Clone, Serialize)]
struct DataEvent {
    id: u32,
    data: String, // base64 dei byte grezzi: il testo UTF-8 può essere spezzato tra un chunk e l'altro
}

#[derive(Clone, Serialize)]
struct ExitEvent {
    id: u32,
}

#[tauri::command]
pub async fn pty_spawn(
    app: AppHandle,
    window: WebviewWindow,
    store: State<'_, Store>,
    ptys: State<'_, Ptys>,
    cwd: String,
    cols: u16,
    rows: u16,
) -> Result<u32, String> {
    let roots = roots_of(&window, &store).map_err(|e| e.message)?;
    let cwd: PathBuf = ensure_inside(&roots, Path::new(&cwd)).map_err(|e| e.message)?;
    if !cwd.is_dir() {
        return Err("cartella inesistente".into());
    }
    let label = window.label().to_string();
    let (a1, l1) = (app.clone(), label.clone());
    let (a2, l2) = (app, label.clone());
    ptys.spawn(
        &label,
        &login_shell(),
        &["-l"],
        &cwd,
        cols,
        rows,
        move |id, bytes| {
            let data = base64::engine::general_purpose::STANDARD.encode(bytes);
            let _ = a1.emit_to(l1.as_str(), "pty:data", DataEvent { id, data });
        },
        move |id| {
            let _ = a2.emit_to(l2.as_str(), "pty:exit", ExitEvent { id });
        },
    )
}

#[tauri::command]
pub async fn pty_write(window: WebviewWindow, ptys: State<'_, Ptys>, id: u32, data: String) -> Result<(), String> {
    ptys.write(window.label(), id, data.as_bytes())
}

#[tauri::command]
pub async fn pty_resize(window: WebviewWindow, ptys: State<'_, Ptys>, id: u32, cols: u16, rows: u16) -> Result<(), String> {
    ptys.resize(window.label(), id, cols, rows)
}

#[tauri::command]
pub async fn pty_kill(window: WebviewWindow, ptys: State<'_, Ptys>, id: u32) -> Result<(), String> {
    ptys.kill(window.label(), id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::{channel, Receiver};
    use std::time::{Duration, Instant};

    fn start(p: &Ptys, owner: &str) -> (u32, Arc<Mutex<Vec<u8>>>, Receiver<u32>) {
        let out = Arc::new(Mutex::new(Vec::new()));
        let (tx, rx) = channel();
        let o = out.clone();
        let id = p
            .spawn(owner, "/bin/sh", &[], Path::new("/tmp"), 80, 24, move |_, b| o.lock().unwrap().extend_from_slice(b), move |id| {
                let _ = tx.send(id);
            })
            .unwrap();
        (id, out, rx)
    }

    fn wait_for(out: &Arc<Mutex<Vec<u8>>>, needle: &str) -> bool {
        let end = Instant::now() + Duration::from_secs(2);
        while Instant::now() < end {
            if String::from_utf8_lossy(&out.lock().unwrap()).contains(needle) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    }

    #[test]
    fn spawn_echo_roundtrip() {
        let p = Ptys::default();
        let (id, out, _rx) = start(&p, "w1");
        p.write("w1", id, b"echo ok-$((40+2))\n").unwrap();
        assert!(wait_for(&out, "ok-42"), "output: {:?}", String::from_utf8_lossy(&out.lock().unwrap()));
        p.kill("w1", id).unwrap();
    }

    #[test]
    fn kill_emits_exit() {
        let p = Ptys::default();
        let (id, _out, rx) = start(&p, "w1");
        p.kill("w1", id).unwrap();
        assert_eq!(rx.recv_timeout(Duration::from_secs(2)).unwrap(), id);
    }

    #[test]
    fn other_window_cannot_touch_pty() {
        let p = Ptys::default();
        let (id, out, rx) = start(&p, "w1");
        assert!(p.write("w2", id, b"echo intruso\n").is_err());
        assert!(p.resize("w2", id, 100, 30).is_err());
        assert!(p.kill("w2", id).is_err());
        assert!(rx.recv_timeout(Duration::from_millis(300)).is_err(), "il PTY non doveva morire");
        assert!(!String::from_utf8_lossy(&out.lock().unwrap()).contains("intruso"));
        p.kill_owner("w1");
        assert_eq!(rx.recv_timeout(Duration::from_secs(2)).unwrap(), id);
    }

    #[test]
    fn rejects_invalid_size() {
        let p = Ptys::default();
        let r = p.spawn("w1", "/bin/sh", &[], Path::new("/tmp"), 0, 24, |_, _| {}, |_| {});
        assert!(r.is_err());
        let (id, _o, _rx) = start(&p, "w1");
        assert!(p.resize("w1", id, 80, 0).is_err());
        p.kill_owner("w1");
    }
}
