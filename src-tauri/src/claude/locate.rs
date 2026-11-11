//! Ambiente della shell di login e ricerca dell'eseguibile `claude`.
//! I processi si avviano sempre con una lista di argomenti, mai con una riga di comando composta.
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const SHELL_TIMEOUT: Duration = Duration::from_secs(5);
const VERSION_TIMEOUT: Duration = Duration::from_secs(5);

/// Variabili che non vanno passate a `claude` (avviando l'app da una sessione Claude Code la CLI
/// rifiuterebbe di partire come sessione annidata).
const STRIPPED_VARS: &[&str] = &["CLAUDECODE"];

/// Esegue `cmd` con stdin chiuso e restituisce lo stdout se il processo termina entro `timeout`
/// con successo; altrimenti lo termina e restituisce `None`.
fn stdout_with_timeout(mut cmd: Command, timeout: Duration) -> Option<String> {
    let mut child = cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().ok()?;
    let mut out = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = out.read_to_end(&mut buf);
        buf
    });
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break Some(st),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };
    let buf = reader.join().ok()?;
    status.filter(|s| s.success())?;
    Some(String::from_utf8_lossy(&buf).into_owned())
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

    #[test]
    fn slow_command_times_out() {
        let mut cmd = Command::new("/bin/sleep");
        cmd.arg("5");
        let t = Instant::now();
        assert_eq!(stdout_with_timeout(cmd, Duration::from_millis(200)), None);
        assert!(t.elapsed() < Duration::from_secs(2));
    }
}
