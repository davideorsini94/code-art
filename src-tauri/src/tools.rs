//! Strumenti di sistema (`git`, `gh`, language server): ricerca nell'ambiente di login ed esecuzione con
//! timeout. Processi sempre con lista di argomenti, in una sessione propria (`stop_group` raggiunge i discendenti).
use crate::claude::locate::{in_new_session, login_env, stop_group, TERM_GRACE};
use serde::Serialize;
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Cartelle d'installazione abituali fuori dal `PATH` (Homebrew su Apple Silicon e Intel).
pub(crate) const SYSTEM_DIRS: &[&str] = &["/opt/homebrew/bin", "/usr/local/bin"];
/// Cartelle d'installazione abituali sotto `HOME`.
const HOME_DIRS: &[&str] = &[".cargo/bin", "go/bin"];
/// Quanto stderr si conserva (il resto viene scartato).
const STDERR_MAX: usize = 8 * 1024;
/// Quanto stdout si accetta: oltre, il processo viene ucciso e `run_tool` restituisce un errore.
const STDOUT_MAX: usize = 16 * 1024 * 1024;
/// Dopo l'uscita del processo, quanto aspettare ancora la chiusura delle pipe.
const DRAIN_GRACE: Duration = Duration::from_millis(300);

/// Errore di un processo fermato (o mai avviato) per l'uscita dall'app.
pub const STOPPED: &str = "Operazione interrotta: l'app si sta chiudendo";
/// Oltre la tolleranza di `stop_group`, quanto aspettare ancora i processi in arresto.
const STOP_MARGIN: Duration = Duration::from_secs(1);

static ENV: OnceLock<HashMap<String, String>> = OnceLock::new();

/// Ambiente di login in cache. Condiviso con l'avvio dell'app (`engine::app_launch` lo usa): `OnceLock`
/// serializza i chiamanti, quindi la shell di login parte una volta sola.
pub fn env() -> &'static HashMap<String, String> {
    ENV.get_or_init(login_env)
}

fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

pub(crate) fn find_tool_in(env: &HashMap<String, String>, name: &str, system_dirs: &[&str]) -> Option<PathBuf> {
    if name.is_empty() || name.contains('/') {
        return None;
    }
    let from_path = env.get("PATH").into_iter().flat_map(|p| p.split(':')).chain(system_dirs.iter().copied());
    let home = env.get("HOME").filter(|h| Path::new(h).is_absolute());
    let from_home = home.into_iter().flat_map(|h| HOME_DIRS.iter().map(move |d| Path::new(h).join(d)));
    from_path
        .filter(|d| Path::new(d).is_absolute())
        .map(|d| Path::new(d).join(name))
        .chain(from_home.map(|d| d.join(name)))
        .find(|p| is_executable(p))
}

/// Cerca `name` nel `PATH` dell'ambiente di login, poi in Homebrew, `~/.cargo/bin` e `~/go/bin`.
pub fn find_tool(env: &HashMap<String, String>, name: &str) -> Option<PathBuf> {
    find_tool_in(env, name, SYSTEM_DIRS)
}

/// Comando per `name` con l'ambiente di login (la sessione propria la crea `run_tool`).
pub fn tool_command(name: &str) -> Result<Command, String> {
    let env = env();
    let path = find_tool(env, name).ok_or_else(|| format!("`{name}` non trovato"))?;
    Ok(command_in(&path, env))
}

/// Comando per il binario `path` con il solo ambiente `env` (quello di login, o uno di prova).
pub fn command_in(path: &Path, env: &HashMap<String, String>) -> Command {
    let mut cmd = Command::new(path);
    cmd.env_clear().envs(env);
    cmd
}

#[derive(Debug)]
pub struct ToolOutput {
    pub status: i32,
    pub stdout: Vec<u8>,
    /// Troncato a 8 KB.
    pub stderr: String,
}

fn spawn_reader(mut r: impl Read + Send + 'static, buf: Arc<Mutex<Vec<u8>>>, cap: usize, over: Option<Arc<AtomicBool>>, done: mpsc::Sender<()>) {
    std::thread::spawn(move || {
        let mut chunk = [0u8; 8192];
        while let Ok(n) = r.read(&mut chunk) {
            if n == 0 {
                break;
            }
            let mut b = buf.lock().unwrap_or_else(|p| p.into_inner());
            let room = cap.saturating_sub(b.len());
            b.extend_from_slice(&chunk[..n.min(room)]);
            if n > room {
                if let Some(o) = &over {
                    o.store(true, Ordering::Relaxed);
                }
            }
        }
        let _ = done.send(());
    });
}

/// Processi in corso (scritture git, `gh`, letture), ciascuno con il suo flag di arresto: all'uscita dall'app
/// `stop_all` li ferma come un timeout (SIGTERM, poi SIGKILL) invece di lasciarli orfani.
#[derive(Default)]
pub struct Running {
    next: AtomicU64,
    flags: Mutex<HashMap<u64, Arc<AtomicBool>>>,
    stopping: AtomicBool,
}

/// Iscrizione nel registro: si cancella da sola quando il processo è finito.
pub struct Ticket<'a> {
    reg: &'a Running,
    id: u64,
    pub stop: Arc<AtomicBool>,
}

impl Drop for Ticket<'_> {
    fn drop(&mut self) {
        crate::lock(&self.reg.flags).remove(&self.id);
    }
}

impl Running {
    /// Iscrive un processo che sta per partire; `Err(STOPPED)` se l'app si sta chiudendo.
    pub fn register(&self) -> Result<Ticket<'_>, String> {
        let mut flags = crate::lock(&self.flags);
        if self.stopping.load(Ordering::SeqCst) {
            return Err(STOPPED.into());
        }
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let stop = Arc::new(AtomicBool::new(false));
        flags.insert(id, stop.clone());
        Ok(Ticket { reg: self, id, stop })
    }

    /// Ferma tutti i processi iscritti e aspetta che abbiano finito (limitato dalla tolleranza di `stop_group`);
    /// da qui in poi non ne parte più nessuno.
    pub fn stop_all(&self) {
        {
            let flags = crate::lock(&self.flags);
            self.stopping.store(true, Ordering::SeqCst);
            for f in flags.values() {
                f.store(true, Ordering::SeqCst);
            }
        }
        let until = Instant::now() + TERM_GRACE + STOP_MARGIN;
        while !crate::lock(&self.flags).is_empty() && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

/// Registro dei processi dell'app (`run_tool`, push sincrono di `gh_pr_create`).
pub static RUNNING: std::sync::LazyLock<Running> = std::sync::LazyLock::new(Running::default);

/// Uscita dall'app: ferma i processi avviati da `run_tool` e dal push sincrono.
pub fn stop_all() {
    RUNNING.stop_all();
}

/// Esegue `cmd` (stdin chiuso o alimentato da `input`); allo scadere di `timeout` uccide il gruppo e
/// restituisce "tempo scaduto". Non resta mai bloccata su pipe tenute aperte da discendenti.
pub fn run_tool(cmd: Command, timeout: Duration, input: Option<Vec<u8>>) -> Result<ToolOutput, String> {
    run_tool_in(&RUNNING, cmd, timeout, input)
}

fn run_tool_in(reg: &Running, mut cmd: Command, timeout: Duration, input: Option<Vec<u8>>) -> Result<ToolOutput, String> {
    let ticket = reg.register()?;
    in_new_session(&mut cmd);
    let stdin = if input.is_some() { Stdio::piped() } else { Stdio::null() };
    let mut child = cmd.stdin(stdin).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().map_err(|e| format!("avvio non riuscito: {e}"))?;
    if let (Some(data), Some(mut si)) = (input, child.stdin.take()) {
        std::thread::spawn(move || {
            let _ = std::io::Write::write_all(&mut si, &data);
        });
    }
    let out_buf = Arc::new(Mutex::new(Vec::new()));
    let err_buf = Arc::new(Mutex::new(Vec::new()));
    let overflow = Arc::new(AtomicBool::new(false));
    let (tx, rx) = mpsc::channel::<()>();
    if let Some(o) = child.stdout.take() {
        spawn_reader(o, out_buf.clone(), STDOUT_MAX, Some(overflow.clone()), tx.clone());
    }
    if let Some(e) = child.stderr.take() {
        spawn_reader(e, err_buf.clone(), STDERR_MAX, None, tx.clone());
    }
    drop(tx);
    let deadline = Instant::now() + timeout;
    let status = loop {
        if overflow.load(Ordering::Relaxed) {
            stop_group(&mut child, TERM_GRACE);
            return Err(format!("output oltre {} MB", STDOUT_MAX / (1024 * 1024)));
        }
        if ticket.stop.load(Ordering::SeqCst) {
            stop_group(&mut child, TERM_GRACE);
            return Err(STOPPED.into());
        }
        match child.try_wait() {
            Ok(Some(st)) => break st,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            Ok(None) => {
                // SIGTERM prima: una scrittura git scaduta non lascia `index.lock` orfani
                stop_group(&mut child, TERM_GRACE);
                return Err(format!("tempo scaduto dopo {} s", timeout.as_secs().max(1)));
            }
            Err(e) => return Err(format!("attesa non riuscita: {e}")),
        }
    };
    // le pipe si chiudono con il processo; un discendente può tenerle aperte: si abbandonano dopo la tolleranza
    let until = Instant::now() + DRAIN_GRACE;
    for _ in 0..2 {
        if rx.recv_timeout(until.saturating_duration_since(Instant::now())).is_err() {
            break;
        }
    }
    if overflow.load(Ordering::Relaxed) {
        return Err(format!("output oltre {} MB", STDOUT_MAX / (1024 * 1024)));
    }
    let stdout = out_buf.lock().unwrap_or_else(|p| p.into_inner()).clone();
    let stderr = String::from_utf8_lossy(&err_buf.lock().unwrap_or_else(|p| p.into_inner())).into_owned();
    Ok(ToolOutput { status: status.code().unwrap_or(-1), stdout, stderr })
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LspAvail {
    pub lang: String,
    pub found: bool,
    pub install_hint: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ToolsStatus {
    pub git: Option<String>,
    pub gh: Option<String>,
    pub lsp: Vec<LspAvail>,
}

fn status_for(env: &HashMap<String, String>, system_dirs: &[&str]) -> ToolsStatus {
    let path = |n: &str| find_tool_in(env, n, system_dirs).map(|p| p.to_string_lossy().into_owned());
    ToolsStatus {
        git: path("git"),
        gh: path("gh"),
        lsp: crate::lsp::servers::SERVERS
            .iter()
            .map(|s| LspAvail {
                lang: s.lang.into(),
                found: s.candidates.iter().any(|c| find_tool_in(env, c.bin, system_dirs).is_some()),
                install_hint: s.install_hint.into(),
            })
            .collect(),
    }
}

#[tauri::command]
pub async fn tools_check() -> Result<ToolsStatus, String> {
    tauri::async_runtime::spawn_blocking(|| status_for(env(), SYSTEM_DIRS)).await.map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn exe(dir: &Path, name: &str, body: &str) -> PathBuf {
        std::fs::create_dir_all(dir).unwrap();
        let p = dir.join(name);
        std::fs::write(&p, body).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p
    }

    fn env_of(path: &str, home: &str) -> HashMap<String, String> {
        [("PATH".to_string(), path.to_string()), ("HOME".to_string(), home.to_string())].into()
    }

    #[test]
    fn find_tool_prefers_login_path() {
        let d = tempfile::tempdir().unwrap();
        let (a, b) = (d.path().join("a"), d.path().join("b"));
        let first = exe(&a, "mytool", "#!/bin/sh\n");
        exe(&b, "mytool", "#!/bin/sh\n");
        // un file non eseguibile in testa viene ignorato
        std::fs::create_dir_all(d.path().join("z")).unwrap();
        std::fs::write(d.path().join("z/mytool"), "x").unwrap();
        let env = env_of(&format!("{}:{}:{}", d.path().join("z").display(), a.display(), b.display()), "/Users/dev");
        assert_eq!(find_tool(&env, "mytool"), Some(first));
        assert_eq!(find_tool(&env, "assente-xyz"), None);
        assert_eq!(find_tool(&env, "../mytool"), None);
    }

    #[test]
    fn find_tool_falls_back_to_homebrew() {
        let d = tempfile::tempdir().unwrap();
        let brew = d.path().join("brew");
        let home = d.path().join("home");
        let from_brew = exe(&brew, "mytool", "#!/bin/sh\n");
        let from_cargo = exe(&home.join(".cargo/bin"), "ra", "#!/bin/sh\n");
        let from_go = exe(&home.join("go/bin"), "gopls", "#!/bin/sh\n");
        let env = env_of("/usr/bin:/bin", home.to_str().unwrap());
        assert_eq!(find_tool_in(&env, "mytool", &[brew.to_str().unwrap()]), Some(from_brew));
        assert_eq!(find_tool_in(&env, "ra", &[]), Some(from_cargo));
        assert_eq!(find_tool_in(&env, "gopls", &[]), Some(from_go));
        assert_eq!(find_tool_in(&env, "mytool", &[]), None);
    }

    #[test]
    fn tools_status_reports_lsp_hints() {
        let d = tempfile::tempdir().unwrap();
        exe(&d.path().join("home/go/bin"), "gopls", "#!/bin/sh\n");
        let env = env_of("/nonexistent", d.path().join("home").to_str().unwrap());
        let s = status_for(&env, &[]);
        assert_eq!(s.git, None);
        let go = s.lsp.iter().find(|l| l.lang == "go").unwrap();
        assert!(go.found);
        let py = s.lsp.iter().find(|l| l.lang == "python").unwrap();
        assert!(!py.found);
        assert!(py.install_hint.contains("pip install python-lsp-server"));
        assert_eq!(s.lsp.len(), 4);
    }

    #[test]
    fn run_tool_times_out() {
        let mut c = Command::new("sleep");
        c.arg("5");
        let t = Instant::now();
        let e = run_tool(c, Duration::from_millis(200), None).unwrap_err();
        assert!(e.contains("tempo scaduto"), "{e}");
        assert!(t.elapsed() < Duration::from_secs(3));
    }

    /// Pid scritto da uno script di prova, quando compare.
    fn read_pid(p: &Path) -> i32 {
        let t0 = Instant::now();
        loop {
            if let Some(n) = std::fs::read_to_string(p).ok().and_then(|s| s.trim().parse().ok()) {
                return n;
            }
            assert!(t0.elapsed() < Duration::from_secs(5), "pid mai scritto");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// `true` se `pid` sparisce entro qualche secondo (uno zombie orfano viene raccolto da launchd).
    fn gone(pid: i32) -> bool {
        let t0 = Instant::now();
        while t0.elapsed() < Duration::from_secs(3) {
            // SAFETY: segnale 0 = solo verifica di esistenza
            if unsafe { libc::kill(pid, 0) } != 0 {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    }

    #[test]
    fn timeout_sends_sigterm_first() {
        let d = tempfile::tempdir().unwrap();
        let marker = d.path().join("term");
        let mut c = Command::new("sh");
        c.args(["-c", &format!("trap 'touch \"{}\"; exit 0' TERM; while :; do sleep 0.05; done", marker.display())]);
        let t = Instant::now();
        assert!(run_tool(c, Duration::from_millis(300), None).unwrap_err().contains("tempo scaduto"));
        assert!(marker.exists(), "il processo non ha ricevuto SIGTERM");
        assert!(t.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn timeout_kills_processes_ignoring_sigterm() {
        let d = tempfile::tempdir().unwrap();
        let pidf = d.path().join("pid");
        let mut c = Command::new("sh");
        c.args(["-c", &format!("trap '' TERM; echo $$ > \"{}\"; while :; do sleep 0.05; done", pidf.display())]);
        let t = Instant::now();
        assert!(run_tool(c, Duration::from_millis(300), None).unwrap_err().contains("tempo scaduto"));
        let el = t.elapsed();
        assert!(el >= TERM_GRACE && el < TERM_GRACE + Duration::from_secs(3), "{el:?}");
        assert!(gone(read_pid(&pidf)));
    }

    #[test]
    fn run_tool_truncates_stderr() {
        let mut c = Command::new("sh");
        c.args(["-c", "head -c 100000 /dev/zero | tr '\\0' x >&2; echo ok"]);
        let o = run_tool(c, Duration::from_secs(10), None).unwrap();
        assert_eq!(o.status, 0);
        assert_eq!(o.stderr.len(), STDERR_MAX);
        assert_eq!(String::from_utf8_lossy(&o.stdout).trim(), "ok");
    }

    #[test]
    fn tool_command_runs_through_run_tool() {
        let mut c = tool_command("sh").expect("sh trovato");
        c.args(["-c", "echo ciao"]);
        let o = run_tool(c, Duration::from_secs(5), None).unwrap();
        assert_eq!(o.status, 0);
        assert_eq!(String::from_utf8_lossy(&o.stdout).trim(), "ciao");
        assert!(tool_command("assente-xyz").unwrap_err().contains("non trovato"));
    }

    #[test]
    fn run_tool_rejects_oversized_stdout() {
        let mut c = Command::new("sh");
        c.args(["-c", "yes | head -c 20000000"]);
        let e = run_tool(c, Duration::from_secs(20), None).unwrap_err();
        assert!(e.contains("output oltre"), "{e}");
    }

    #[test]
    fn stop_all_terminates_running_tools_with_sigterm_first() {
        let d = tempfile::tempdir().unwrap();
        let (pidf, marker) = (d.path().join("pid"), d.path().join("term"));
        let reg = Arc::new(Running::default());
        let mut c = Command::new("sh");
        c.args(["-c", &format!("trap 'touch \"{}\"; exit 0' TERM; echo $$ > \"{}\"; while :; do sleep 0.05; done", marker.display(), pidf.display())]);
        let r = reg.clone();
        let worker = std::thread::spawn(move || run_tool_in(&r, c, Duration::from_secs(60), None));
        let pid = read_pid(&pidf);
        let t = Instant::now();
        reg.stop_all();
        assert!(t.elapsed() < TERM_GRACE + Duration::from_secs(2), "{:?}", t.elapsed());
        // stop_all torna solo quando il gruppo è fermo: SIGTERM arrivato, nessuno sopravvive
        assert!(marker.exists(), "il processo non ha ricevuto SIGTERM");
        // SAFETY: segnale 0 = solo verifica di esistenza
        assert!(unsafe { libc::kill(-pid, 0) } != 0 || gone(pid), "il gruppo è sopravvissuto");
        assert_eq!(worker.join().unwrap().unwrap_err(), STOPPED);
        // dopo l'arresto non parte più nulla
        assert_eq!(run_tool_in(&reg, Command::new("true"), Duration::from_secs(5), None).unwrap_err(), STOPPED);
    }

    #[test]
    fn run_tool_feeds_input_and_reports_status() {
        let mut c = Command::new("sh");
        c.args(["-c", "cat; exit 3"]);
        let o = run_tool(c, Duration::from_secs(5), Some(b"ciao".to_vec())).unwrap();
        assert_eq!(o.status, 3);
        assert_eq!(o.stdout, b"ciao");
    }
}
