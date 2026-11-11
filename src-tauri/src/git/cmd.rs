//! Esecuzione di `git` con lista di argomenti, ambiente di login e timeout.
use crate::tools::{run_tool, tool_command, ToolOutput};
use std::path::Path;
use std::time::Duration;

/// Timeout delle letture.
pub const READ_TIMEOUT: Duration = Duration::from_secs(15);

/// Esegue `git <args>` nella cartella `repo`. Argomenti sempre come lista (mai una riga di comando); i
/// percorsi e i nomi che arrivano dalla pagina vanno dopo `--` o in forma `rev:percorso` (mai nudi).
/// Ambiente: login + `GIT_TERMINAL_PROMPT=0` (nessuna richiesta di credenziali), `GIT_OPTIONAL_LOCKS=0`
/// (le letture non toccano l'indice, quindi non risvegliano il watcher), `LC_ALL=C` (output analizzabile).
pub fn git(repo: &Path, args: &[&str], timeout: Duration) -> Result<ToolOutput, String> {
    let mut cmd = tool_command("git")?;
    cmd.current_dir(repo)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("LC_ALL", "C")
        .args(["-c", "core.quotepath=off", "-c", "color.ui=never"])
        .args(args);
    run_tool(cmd, timeout, None)
}

/// Come `git`, ma un'uscita diversa da 0 è un errore con il messaggio di git.
pub fn git_ok(repo: &Path, args: &[&str], timeout: Duration) -> Result<Vec<u8>, String> {
    let out = git(repo, args, timeout)?;
    if out.status == 0 {
        Ok(out.stdout)
    } else {
        Err(format!("git: {}", out.stderr.trim()))
    }
}
