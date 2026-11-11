//! Strumenti di sistema (`git`, `gh`, language server): ricerca nell'ambiente di login ed esecuzione con
//! timeout. Processi sempre con lista di argomenti, in una sessione propria (`kill_group` raggiunge i discendenti).
// `find_tool`, `tool_command` e `run_tool` servono ai moduli git/gh/lsp dei task successivi.
#![allow(dead_code)]
use crate::claude::locate::{in_new_session, kill_group, login_env};
use serde::Serialize;
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Cartelle d'installazione abituali fuori dal `PATH` (Homebrew su Apple Silicon e Intel).
const SYSTEM_DIRS: &[&str] = &["/opt/homebrew/bin", "/usr/local/bin"];
/// Cartelle d'installazione abituali sotto `HOME`.
const HOME_DIRS: &[&str] = &[".cargo/bin", "go/bin"];
/// Quanto stderr si conserva (il resto viene scartato).
const STDERR_MAX: usize = 8 * 1024;
/// Dopo l'uscita del processo, quanto aspettare ancora la chiusura delle pipe.
const DRAIN_GRACE: Duration = Duration::from_millis(300);

/// Language server noti: (linguaggio, binari candidati in ordine di preferenza, come installarlo).
/// Il client LSP li importa da qui.
pub const LSP_SERVERS: &[(&str, &[&str], &str)] = &[
    ("python", &["pyright-langserver", "pylsp"], "npm i -g pyright  oppure  pip install python-lsp-server"),
    ("java", &["jdtls"], "brew install jdtls"),
    ("go", &["gopls"], "go install golang.org/x/tools/gopls@latest"),
    ("rust", &["rust-analyzer"], "rustup component add rust-analyzer"),
];

static ENV: OnceLock<HashMap<String, String>> = OnceLock::new();

/// Registra l'ambiente di login già calcolato all'avvio (evita una seconda shell di login).
pub fn set_env(env: HashMap<String, String>) {
    let _ = ENV.set(env);
}

/// Ambiente di login in cache; se nessuno l'ha ancora registrato lo calcola.
pub fn env() -> &'static HashMap<String, String> {
    ENV.get_or_init(login_env)
}

fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

fn find_tool_in(env: &HashMap<String, String>, name: &str, system_dirs: &[&str]) -> Option<PathBuf> {
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

/// Comando per `name` con l'ambiente di login e in una sessione propria.
pub fn tool_command(name: &str) -> Result<Command, String> {
    let env = env();
    let path = find_tool(env, name).ok_or_else(|| format!("`{name}` non trovato"))?;
    let mut cmd = Command::new(path);
    cmd.env_clear().envs(env);
    in_new_session(&mut cmd);
    Ok(cmd)
}

#[derive(Debug)]
pub struct ToolOutput {
    pub status: i32,
    pub stdout: Vec<u8>,
    /// Troncato a 8 KB.
    pub stderr: String,
}

fn spawn_reader(mut r: impl Read + Send + 'static, buf: Arc<Mutex<Vec<u8>>>, cap: usize, done: mpsc::Sender<()>) {
    std::thread::spawn(move || {
        let mut chunk = [0u8; 8192];
        while let Ok(n) = r.read(&mut chunk) {
            if n == 0 {
                break;
            }
            let mut b = buf.lock().unwrap_or_else(|p| p.into_inner());
            let room = cap.saturating_sub(b.len());
            b.extend_from_slice(&chunk[..n.min(room)]);
        }
        let _ = done.send(());
    });
}

/// Esegue `cmd` (stdin chiuso o alimentato da `input`); allo scadere di `timeout` uccide il gruppo e
/// restituisce "tempo scaduto". Non resta mai bloccata su pipe tenute aperte da discendenti.
pub fn run_tool(mut cmd: Command, timeout: Duration, input: Option<Vec<u8>>) -> Result<ToolOutput, String> {
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
    let (tx, rx) = mpsc::channel::<()>();
    if let Some(o) = child.stdout.take() {
        spawn_reader(o, out_buf.clone(), usize::MAX, tx.clone());
    }
    if let Some(e) = child.stderr.take() {
        spawn_reader(e, err_buf.clone(), STDERR_MAX, tx.clone());
    }
    drop(tx);
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break st,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            Ok(None) => {
                kill_group(child.id());
                let _ = child.wait();
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
        lsp: LSP_SERVERS
            .iter()
            .map(|(lang, bins, hint)| LspAvail {
                lang: (*lang).into(),
                found: bins.iter().any(|b| find_tool_in(env, b, system_dirs).is_some()),
                install_hint: (*hint).into(),
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
    fn run_tool_feeds_input_and_reports_status() {
        let mut c = Command::new("sh");
        c.args(["-c", "cat; exit 3"]);
        let o = run_tool(c, Duration::from_secs(5), Some(b"ciao".to_vec())).unwrap();
        assert_eq!(o.status, 3);
        assert_eq!(o.stdout, b"ciao");
    }
}
