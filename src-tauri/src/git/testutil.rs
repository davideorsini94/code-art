//! Aiuti per i test git: repository temporanei con identità fittizia, isolati dalla configurazione dell'utente.
use crate::store::{Project, Store};
use std::path::Path;
use std::process::Command;

/// Esegue git per preparare i repository di prova, isolato dalla configurazione dell'utente.
pub fn g(dir: &Path, args: &[&str]) {
    let out = g_out(dir, args);
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

/// Come `g`, ma restituisce lo stdout (senza a capo finale).
pub fn g_str(dir: &Path, args: &[&str]) -> String {
    let out = g_out(dir, args);
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim_end_matches('\n').to_string()
}

pub fn g_out(dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new("git")
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .args(args)
        .output()
        .unwrap()
}

/// Identità locale fittizia, nessuna firma, nessun hook.
pub fn configure(d: &Path) {
    for (k, v) in [("user.name", "test"), ("user.email", "test@example.invalid"), ("commit.gpgsign", "false"), ("core.hooksPath", "/dev/null")] {
        g(d, &["config", k, v]);
    }
}

/// Repository temporaneo con identità locale fittizia.
pub fn repo() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    g(d.path(), &["init", "-q", "-b", "main"]);
    configure(d.path());
    d
}

pub fn commit_all(d: &Path, msg: &str) {
    g(d, &["add", "-A"]);
    g(d, &["commit", "-q", "-m", msg]);
}

/// Store con un progetto "p" fidato che contiene `folders`.
pub fn project(folders: &[&Path]) -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let s = Store::open(&dir.path().join("t.db")).unwrap();
    let folders: Vec<String> = folders.iter().map(|f| f.canonicalize().unwrap().to_string_lossy().into_owned()).collect();
    s.upsert_project(&Project { id: "p".into(), name: "p".into(), folders: folders.clone() }).unwrap();
    for f in &folders {
        crate::trust::mark_trusted(&s, f).unwrap();
    }
    (dir, s)
}

pub fn root(d: &tempfile::TempDir) -> String {
    d.path().canonicalize().unwrap().to_string_lossy().into_owned()
}
