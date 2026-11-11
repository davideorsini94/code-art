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

/// Maniglie del PTY, ciascuna con il proprio lock: un'operazione bloccata (es. scrittura su una
/// shell che non legge) non deve tenere fermo il lock globale della mappa né le altre operazioni.
struct Session {
    owner: String,
    master: Arc<Mutex<Box<dyn MasterPty + Send>>>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    killer: Arc<Mutex<Box<dyn ChildKiller + Send + Sync>>>,
}

#[derive(Default, Clone)]
pub struct Ptys {
    next: Arc<AtomicU32>,
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
        let session = Session {
            owner: owner.to_string(), master: Arc::new(Mutex::new(pair.master)),
            writer: Arc::new(Mutex::new(writer)),
            killer: Arc::new(Mutex::new(child.clone_killer())),
        };
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

    /// Verifica il proprietario e restituisce `f` applicata a un clone della sessione: il lock della
    /// mappa è già rilasciato quando `f` (potenzialmente bloccante) viene eseguita.
    fn owned<T>(&self, owner: &str, id: u32, f: impl FnOnce(&Session) -> T) -> Result<T, String> {
        let s = {
            let map = self.sessions.lock().unwrap();
            match map.get(&id) {
                Some(s) if s.owner == owner => Session {
                    owner: s.owner.clone(),
                    master: s.master.clone(),
                    writer: s.writer.clone(),
                    killer: s.killer.clone(),
                },
                _ => return Err("terminale inesistente".into()), // anche se di un'altra finestra: stessa risposta
            }
        };
        Ok(f(&s))
    }

    pub fn write(&self, owner: &str, id: u32, data: &[u8]) -> Result<(), String> {
        if data.len() > MAX_WRITE_BYTES {
            return Err("dati troppo grandi".into());
        }
        self.owned(owner, id, |s| {
            let mut w = s.writer.lock().unwrap();
            w.write_all(data).and_then(|_| w.flush()).map_err(|e| e.to_string())
        })?
    }

    pub fn resize(&self, owner: &str, id: u32, cols: u16, rows: u16) -> Result<(), String> {
        let size = size(cols, rows)?;
        self.owned(owner, id, |s| s.master.lock().unwrap().resize(size).map_err(|e| e.to_string()))?
    }

    pub fn kill(&self, owner: &str, id: u32) -> Result<(), String> {
        self.owned(owner, id, |s| s.killer.lock().unwrap().kill().map_err(|e| e.to_string()))?
    }

    /// Chiude tutti i PTY di una finestra (alla sua distruzione).
    pub fn kill_owner(&self, owner: &str) {
        let killers: Vec<_> =
            self.sessions.lock().unwrap().values().filter(|s| s.owner == owner).map(|s| s.killer.clone()).collect();
        for k in killers {
            let _ = k.lock().unwrap().kill();
        }
    }
}

// ---------- comandi ----------

#[derive(Clone, Serialize)]
struct DataEvent {
    id: u32,
    data: String, // base64 dei byte grezzi: il testo UTF-8 può essere spezzato tra un chunk e l'altro
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpawnInfo {
    id: u32,
    shell: String, // nome della shell (basename), per il titolo della scheda
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
) -> Result<SpawnInfo, String> {
    let roots = roots_of(&window, &store).map_err(|e| e.message)?;
    let cwd: PathBuf = ensure_inside(&roots, Path::new(&cwd)).map_err(|e| e.message)?;
    if !cwd.is_dir() {
        return Err("cartella inesistente".into());
    }
    let label = window.label().to_string();
    let (a1, l1) = (app.clone(), label.clone());
    let (a2, l2) = (app, label.clone());
    let shell = login_shell();
    let id = ptys.spawn(
        &label,
        &shell,
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
    )?;
    let shell = Path::new(&shell).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "shell".into());
    Ok(SpawnInfo { id, shell })
}

/// Esegue un'operazione potenzialmente bloccante fuori dai worker async.
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn pty_write(window: WebviewWindow, ptys: State<'_, Ptys>, id: u32, data: String) -> Result<(), String> {
    let (p, l) = (ptys.inner().clone(), window.label().to_string());
    blocking(move || p.write(&l, id, data.as_bytes())).await
}

#[tauri::command]
pub async fn pty_resize(window: WebviewWindow, ptys: State<'_, Ptys>, id: u32, cols: u16, rows: u16) -> Result<(), String> {
    let (p, l) = (ptys.inner().clone(), window.label().to_string());
    blocking(move || p.resize(&l, id, cols, rows)).await
}

#[tauri::command]
pub async fn pty_kill(window: WebviewWindow, ptys: State<'_, Ptys>, id: u32) -> Result<(), String> {
    let (p, l) = (ptys.inner().clone(), window.label().to_string());
    blocking(move || p.kill(&l, id)).await
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
    fn blocked_write_does_not_block_kill_or_other_ptys() {
        let p = Ptys::default();
        let (tx, rx) = channel();
        // un figlio che non legge mai dal tty: una scrittura grande si blocca
        let stuck = p
            .spawn("w1", "/bin/sh", &["-c", "stty raw -echo; sleep 30"], Path::new("/tmp"), 80, 24, |_, _| {}, move |id| {
                let _ = tx.send(id);
            })
            .unwrap();
        let (other, other_out, _rx2) = start(&p, "w1");
        std::thread::sleep(Duration::from_millis(300)); // lascia applicare stty
        let p2 = p.clone();
        let writer = std::thread::spawn(move || {
            let _ = p2.write("w1", stuck, &vec![b'x'; 1024 * 1024]);
        });
        std::thread::sleep(Duration::from_millis(300));
        assert!(!writer.is_finished(), "la scrittura doveva essere bloccata");
        let t = Instant::now();
        p.write("w1", other, b"echo vivo-$((1+1))\n").unwrap();
        assert!(wait_for(&other_out, "vivo-2"));
        p.resize("w1", other, 90, 30).unwrap();
        p.kill("w1", stuck).unwrap();
        assert_eq!(rx.recv_timeout(Duration::from_secs(2)).unwrap(), stuck);
        assert!(t.elapsed() < Duration::from_secs(2));
        p.kill_owner("w1");
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
