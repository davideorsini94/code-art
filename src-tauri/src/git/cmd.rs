//! Esecuzione di `git` con lista di argomenti, ambiente di login e timeout.
use crate::tools::{run_tool, tool_command, ToolOutput};
use std::path::Path;
use std::process::Command;
use std::time::Duration;

/// Timeout delle letture.
pub const READ_TIMEOUT: Duration = Duration::from_secs(15);
/// Timeout delle scritture locali (commit con hook, merge, checkout di alberi grandi).
pub const WRITE_TIMEOUT: Duration = Duration::from_secs(120);
/// Timeout di fetch/pull/push (annullabili).
pub const REMOTE_TIMEOUT: Duration = Duration::from_secs(600);

/// Comando `git` comune a letture e scritture: login + `GIT_TERMINAL_PROMPT=0` (nessuna richiesta di credenziali),
/// nessun editor (`GIT_EDITOR=true`: i messaggi arrivano sempre da `-F`/`--no-edit`). `LC_ALL=C` solo dove l'output
/// viene analizzato (le variabili passano anche agli hook del repository).
pub fn base_command(repo: &Path) -> Result<Command, String> {
    let mut cmd = tool_command("git")?;
    cmd.current_dir(repo)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_EDITOR", "true")
        .env("GIT_MERGE_AUTOEDIT", "no")
        .args(["-c", "core.quotepath=off", "-c", "color.ui=never"]);
    // nei test il codice sotto esame non dipende dalla configurazione git dell'utente (firma, status.*, fsmonitor)
    #[cfg(test)]
    cmd.env("GIT_CONFIG_GLOBAL", "/dev/null").env("GIT_CONFIG_NOSYSTEM", "1");
    Ok(cmd)
}

/// Esegue `git <args>` nella cartella `repo` per una LETTURA. Argomenti sempre come lista (mai una riga di comando);
/// i percorsi e i nomi che arrivano dalla pagina vanno dopo `--` o in forma `rev:percorso` (mai nudi).
/// In più `GIT_OPTIONAL_LOCKS=0`: le letture non toccano l'indice, quindi non risvegliano il watcher.
pub fn git(repo: &Path, args: &[&str], timeout: Duration) -> Result<ToolOutput, String> {
    let mut cmd = base_command(repo)?;
    cmd.env("GIT_OPTIONAL_LOCKS", "0").env("LC_ALL", "C").args(args);
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

/// Tipo di scrittura: decide quali variabili d'ambiente vedono git e gli hook che esegue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Write {
    /// Output analizzato (`merge`, `branch -d`, `rev-parse`): `LC_ALL=C`.
    Parsed,
    /// Percorsi della pagina come pathspec (`add`, `restore`, `rm --cached`, `ls-files`): `LC_ALL=C` e
    /// `GIT_LITERAL_PATHSPECS=1` (`:/` o `*` non diventano pattern). Solo qui: negli hook di commit/merge/switch
    /// renderebbe letterali anche i glob degli hook (`-- '*.rs'` non troverebbe nulla).
    Paths,
    /// Comandi che eseguono hook e il cui output non si analizza (`commit`, `switch`): l'ambiente dell'utente.
    Hooks,
}

/// Esegue `git <args>` per una SCRITTURA: senza `GIT_OPTIONAL_LOCKS=0` (l'indice va aggiornato e bloccato
/// normalmente); `input` alimenta lo stdin (messaggio di commit, elenco di percorsi).
pub fn git_write(repo: &Path, mode: Write, args: &[&str], input: Option<Vec<u8>>) -> Result<ToolOutput, String> {
    let mut cmd = base_command(repo)?;
    if mode != Write::Hooks {
        cmd.env("LC_ALL", "C");
    }
    if mode == Write::Paths {
        cmd.env("GIT_LITERAL_PATHSPECS", "1");
    }
    cmd.args(args);
    run_tool(cmd, WRITE_TIMEOUT, input)
}

/// Come `git_write`, ma un'uscita diversa da 0 è un errore con il messaggio di git.
pub fn git_write_ok(repo: &Path, mode: Write, args: &[&str], input: Option<Vec<u8>>) -> Result<Vec<u8>, String> {
    let out = git_write(repo, mode, args, input)?;
    if out.status == 0 {
        Ok(out.stdout)
    } else {
        Err(format!("git: {}", out.stderr.trim()))
    }
}
