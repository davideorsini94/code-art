//! Ambiente della shell di login e ricerca dell'eseguibile `claude`.
//! I processi si avviano sempre con una lista di argomenti, mai con una riga di comando composta.
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

const SHELL_TIMEOUT: Duration = Duration::from_secs(5);
const VERSION_TIMEOUT: Duration = Duration::from_secs(5);

/// Variabili che non vanno passate a `claude` (avviando l'app da una sessione Claude Code la CLI
/// rifiuterebbe di partire come sessione annidata).
const STRIPPED_VARS: &[&str] = &["CLAUDECODE"];

/// Il figlio parte in una sessione nuova (= gruppo di processi proprio, senza terminale di controllo):
/// `kill_group` raggiunge anche i suoi discendenti, e una shell interattiva non si ferma su un terminale.
pub fn in_new_session(cmd: &mut Command) {
    use std::os::unix::process::CommandExt;
    // SAFETY: `setsid` è async-signal-safe e non tocca memoria del processo padre.
    unsafe {
        cmd.pre_exec(|| if libc::setsid() == -1 { Err(std::io::Error::last_os_error()) } else { Ok(()) });
    }
}

/// SIGKILL a tutto il gruppo di un figlio avviato con `in_new_session` (pgid = pid). Va chiamata solo
/// prima che il figlio sia stato raccolto con `wait`, altrimenti il pid potrebbe essere già riusato.
pub fn kill_group(pid: u32) {
    if let Ok(pid) = i32::try_from(pid) {
        if pid > 0 {
            // SAFETY: semplice chiamata di sistema; pid negativo = gruppo.
            unsafe {
                libc::kill(-pid, libc::SIGKILL);
            }
        }
    }
}

/// Dopo l'uscita del processo, quanto aspettare ancora la fine dello stdout (un discendente in
/// background può tenere aperta la pipe all'infinito: quello che serve è già stato scritto).
const DRAIN_GRACE: Duration = Duration::from_millis(300);

/// Esegue `cmd` con stdin chiuso e restituisce lo stdout se il processo termina entro `timeout`
/// con successo; altrimenti uccide il suo gruppo e restituisce `None`. Non resta mai bloccata:
/// lo stdout si legge in un buffer condiviso, senza aspettare la chiusura della pipe.
fn stdout_with_timeout(mut cmd: Command, timeout: Duration) -> Option<String> {
    in_new_session(&mut cmd);
    let mut child = cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().ok()?;
    let mut out = child.stdout.take()?;
    let buf = Arc::new(Mutex::new(Vec::new()));
    let (done_tx, done_rx) = mpsc::channel::<()>();
    {
        let buf = buf.clone();
        std::thread::spawn(move || {
            let mut chunk = [0u8; 8192];
            while let Ok(n) = out.read(&mut chunk) {
                if n == 0 {
                    break;
                }
                buf.lock().unwrap_or_else(|p| p.into_inner()).extend_from_slice(&chunk[..n]);
            }
            let _ = done_tx.send(());
        });
    }
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break Some(st),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                kill_group(child.id());
                let _ = child.wait();
                break None;
            }
        }
    };
    status.filter(|s| s.success())?;
    // il lettore è un thread a parte: se la pipe resta aperta lo si abbandona
    let _ = done_rx.recv_timeout(DRAIN_GRACE.min(deadline.saturating_duration_since(Instant::now()).max(Duration::from_millis(50))));
    let bytes = buf.lock().unwrap_or_else(|p| p.into_inner()).clone();
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

fn is_env_key(k: &str) -> bool {
    let mut chars = k.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_') && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Interpreta l'output di `env`: righe `CHIAVE=valore`; le righe che non iniziano con una chiave valida
/// continuano il valore precedente (valori su più righe) o, in testa, vengono scartate (rumore della shell).
pub fn parse_env(text: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let mut last: Option<String> = None;
    for line in text.lines() {
        match line.split_once('=') {
            Some((k, v)) if is_env_key(k) => {
                map.insert(k.to_string(), v.to_string());
                last = Some(k.to_string());
            }
            _ => {
                if let Some(v) = last.as_ref().and_then(|k| map.get_mut(k)) {
                    v.push('\n');
                    v.push_str(line);
                }
            }
        }
    }
    map
}

/// Ambiente della shell di login dell'utente (`$SHELL -l -i -c env`, 5 s al massimo);
/// in caso di problemi, l'ambiente del processo corrente.
pub fn login_env() -> HashMap<String, String> {
    let shell = std::env::var("SHELL").ok().filter(|s| Path::new(s).is_absolute()).unwrap_or_else(|| "/bin/zsh".into());
    let mut cmd = Command::new(shell);
    cmd.args(["-l", "-i", "-c", "env"]);
    let mut env = stdout_with_timeout(cmd, SHELL_TIMEOUT)
        .map(|t| parse_env(&t))
        .filter(|m| m.contains_key("PATH"))
        .unwrap_or_else(|| std::env::vars().collect());
    for k in STRIPPED_VARS {
        env.remove(*k);
    }
    env
}

fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

/// Cerca `claude` nel `PATH` dell'ambiente dato, poi nei percorsi d'installazione abituali sotto `HOME`.
pub fn find_claude(env: &HashMap<String, String>) -> Option<PathBuf> {
    let from_path = env
        .get("PATH")
        .into_iter()
        .flat_map(|p| p.split(':'))
        .filter(|d| Path::new(d).is_absolute())
        .map(|d| Path::new(d).join("claude"));
    let from_home = env
        .get("HOME")
        .into_iter()
        .flat_map(|h| [".local/bin/claude", ".claude/local/claude"].map(|rel| Path::new(h).join(rel)));
    from_path.chain(from_home).find(|p| is_executable(p))
}

/// Prima riga di `claude --version`, se il comando riesce.
pub fn claude_version(claude: &Path, env: &HashMap<String, String>) -> Option<String> {
    let mut cmd = Command::new(claude);
    cmd.arg("--version").env_clear().envs(env);
    let out = stdout_with_timeout(cmd, VERSION_TIMEOUT)?;
    out.lines().next().map(|l| l.trim().to_string()).filter(|l| !l.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn parse_env_skips_noise_and_keeps_multiline_values() {
        let text = "benvenuto nella shell\nPATH=/usr/bin:/bin\nMULTI=a\nb\nHOME=/Users/dev\n1BAD=x\n";
        let env = parse_env(text);
        assert_eq!(env.get("PATH").map(String::as_str), Some("/usr/bin:/bin"));
        assert_eq!(env.get("MULTI").map(String::as_str), Some("a\nb"));
        assert_eq!(env.get("HOME").map(String::as_str), Some("/Users/dev\n1BAD=x"));
        assert!(!env.contains_key("benvenuto nella shell"));
    }

    #[test]
    fn finds_executable_in_path_and_reads_version() {
        let d = tempfile::tempdir().unwrap();
        let empty = d.path().join("vuota");
        let bin = d.path().join("bin");
        std::fs::create_dir_all(&empty).unwrap();
        std::fs::create_dir_all(&bin).unwrap();
        // un file non eseguibile con lo stesso nome viene ignorato
        std::fs::write(empty.join("claude"), "x").unwrap();
        let exe = bin.join("claude");
        std::fs::write(&exe, "#!/bin/sh\necho '9.9.9 (Claude Code)'\n").unwrap();
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        let env: HashMap<String, String> = [(
            "PATH".to_string(),
            format!("{}:relativa:{}", empty.display(), bin.display()),
        )]
        .into();
        assert_eq!(find_claude(&env), Some(exe.clone()));
        assert_eq!(claude_version(&exe, &env).as_deref(), Some("9.9.9 (Claude Code)"));
        assert_eq!(find_claude(&HashMap::new()), None);
    }

    fn script(dir: &Path, body: &str) -> PathBuf {
        let p = dir.join("s.sh");
        std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p
    }

    #[test]
    fn background_job_holding_stdout_does_not_block() {
        // come un demone lanciato da .zshrc: eredita lo stdout e lo tiene aperto
        let d = tempfile::tempdir().unwrap();
        let s = script(d.path(), "sleep 5 &\necho A=1");
        let t = Instant::now();
        assert_eq!(stdout_with_timeout(Command::new(&s), Duration::from_secs(2)).as_deref(), Some("A=1\n"));
        assert!(t.elapsed() < Duration::from_secs(2), "{:?}", t.elapsed());
    }

    #[test]
    fn timeout_kills_the_whole_group() {
        let d = tempfile::tempdir().unwrap();
        let pidfile = d.path().join("bg.pid");
        let s = script(d.path(), &format!("sleep 30 &\necho $! > {}\nsleep 30", pidfile.display()));
        let t = Instant::now();
        assert_eq!(stdout_with_timeout(Command::new(&s), Duration::from_millis(800)), None);
        assert!(t.elapsed() < Duration::from_secs(3), "{:?}", t.elapsed());
        // sotto carico la shell può essere uccisa prima ancora di avviare il processo in background
        let Some(bg) = std::fs::read_to_string(&pidfile).ok().and_then(|p| p.trim().parse::<i32>().ok()) else { return };
        // il processo in background è stato ucciso con il gruppo (eventualmente zombie per un attimo)
        let deadline = Instant::now() + Duration::from_secs(2);
        // SAFETY: kill con segnale 0 verifica solo l'esistenza del processo
        while unsafe { libc::kill(bg, 0) } == 0 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        let alive = unsafe { libc::kill(bg, 0) } == 0;
        let zombie = Command::new("/bin/ps").args(["-o", "stat=", "-p", &bg.to_string()]).output().map(|o| String::from_utf8_lossy(&o.stdout).contains('Z')).unwrap_or(false);
        assert!(!alive || zombie, "processo in background ancora vivo");
    }

    #[test]
    fn slow_command_times_out() {
        let mut cmd = Command::new("/bin/sleep");
        cmd.arg("5");
        let t = Instant::now();
        assert_eq!(stdout_with_timeout(cmd, Duration::from_millis(200)), None);
        assert!(t.elapsed() < Duration::from_secs(2));
    }
}
