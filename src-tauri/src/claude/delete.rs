//! Eliminazione di una conversazione: sposta nel Cestino il file di sessione di Claude Code
//! (`~/.claude/projects/<cartella>/<sessionId>.jsonl`) e l'eventuale cartella `<sessionId>/` accanto.
//! È l'UNICA eccezione voluta alla regola "l'app non modifica `~/.claude`" (richiesta del proprietario):
//! mai cancellazione definitiva, sempre dopo una conferma nativa mostrata dal core.
//! Qui la logica pura e testabile: individuazione dei file, esecutore del Cestino iniettabile, testo del dialogo.
use super::sessions;
use crate::store::Result;
use serde::Serialize;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

pub const DIALOG_TITLE: &str = "Eliminare la conversazione?";
pub const DIALOG_NOTE: &str = "Verrà spostata nel Cestino. Sparirà anche da Claude Code.";
pub const BTN_TRASH: &str = "Sposta nel Cestino";
pub const BTN_CANCEL: &str = "Annulla";
const UNTITLED: &str = "Conversazione senza titolo";
const TITLE_SHOWN_MAX: usize = 100;

/// Esito di `conv_delete`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DeleteOutcome {
    Deleted,
    Cancelled,
}

/// Chi sposta un percorso nel Cestino; nei test un finto (mai il vero Cestino).
pub trait Trasher: Send + Sync {
    fn trash(&self, path: &Path) -> Result<()>;
}

/// Il Cestino di macOS.
pub struct SystemTrasher;

impl Trasher for SystemTrasher {
    fn trash(&self, path: &Path) -> Result<()> {
        trash::delete(path).map_err(|e| e.to_string())
    }
}

/// File da spostare: il file di sessione e, se c'è, la cartella con lo stesso id (dati dei sotto-agenti).
#[derive(Debug, PartialEq, Eq)]
pub struct SessionFiles {
    pub file: PathBuf,
    pub dir: Option<PathBuf>,
}

/// Trova i file della sessione SOLO unendo le cartelle di sessione note del progetto con `<id>.jsonl`.
/// Rifiuta id non validi, un file che è un link simbolico o che, canonicalizzato, esce dalla radice delle
/// sessioni. `Ok(None)`: il file non c'è (già sparito dal disco).
pub fn locate(root: &Path, folders: &[String], session_id: &str) -> Result<Option<SessionFiles>> {
    if !sessions::valid_session_id(session_id) {
        return Err("sessione non valida".into());
    }
    let root_real = root.canonicalize().map_err(|e| e.to_string())?;
    for d in sessions::session_dirs_for(root, folders) {
        let file = d.join(format!("{session_id}.jsonl"));
        let Ok(md) = std::fs::symlink_metadata(&file) else { continue };
        if md.file_type().is_symlink() {
            return Err("il file della sessione è un link simbolico".into());
        }
        if !md.is_file() {
            continue;
        }
        let real = file.canonicalize().map_err(|e| e.to_string())?;
        if !real.starts_with(&root_real) {
            return Err("il file della sessione è fuori dalla cartella delle sessioni".into());
        }
        // la cartella accanto solo se è davvero una cartella dentro la radice (un link simbolico non si tocca)
        let dir = d.join(session_id);
        let dir = std::fs::symlink_metadata(&dir)
            .ok()
            .filter(|m| m.is_dir())
            .and_then(|_| dir.canonicalize().ok())
            .filter(|r| r.starts_with(&root_real))
            .map(|_| dir);
        return Ok(Some(SessionFiles { file, dir }));
    }
    Ok(None)
}

/// Sposta nel Cestino il file (errore = niente è cambiato) e poi, al meglio, la cartella accanto.
pub fn trash_files(t: &dyn Trasher, files: &SessionFiles) -> Result<()> {
    t.trash(&files.file)?;
    if let Some(d) = &files.dir {
        let _ = t.trash(d);
    }
    Ok(())
}

/// Testo del dialogo: titolo come testo semplice, una riga, senza caratteri di controllo, accorciato.
pub fn dialog_message(title: &str) -> String {
    let t: String = title.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    let t = t.split_whitespace().collect::<Vec<_>>().join(" ");
    let t = if t.is_empty() { UNTITLED.to_string() } else { t.chars().take(TITLE_SHOWN_MAX).collect() };
    format!("{t}\n\n{DIALOG_NOTE}")
}

/// Un solo dialogo di eliminazione alla volta per finestra (chiave: id del progetto).
#[derive(Default)]
pub struct DeletePrompts(Arc<Mutex<HashSet<String>>>);

pub struct PromptGuard {
    set: Arc<Mutex<HashSet<String>>>,
    key: String,
}

impl Drop for PromptGuard {
    fn drop(&mut self) {
        self.set.lock().unwrap_or_else(|p| p.into_inner()).remove(&self.key);
    }
}

impl DeletePrompts {
    pub fn acquire(&self, key: &str) -> Option<PromptGuard> {
        let mut s = self.0.lock().unwrap_or_else(|p| p.into_inner());
        s.insert(key.to_string()).then(|| PromptGuard { set: self.0.clone(), key: key.to_string() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SID: &str = "11111111-1111-4111-8111-111111111111";

    fn setup() -> (tempfile::TempDir, PathBuf, String) {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().canonicalize().unwrap();
        let root = base.join("projects");
        let folder = base.join("proj");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::create_dir_all(root.join(sessions::encode_project_dir(&folder.to_string_lossy()))).unwrap();
        (dir, root, folder.to_string_lossy().into_owned())
    }

    fn sdir(root: &Path, folder: &str) -> PathBuf {
        root.join(sessions::encode_project_dir(folder))
    }

    #[test]
    fn finds_file_and_sibling_dir() {
        let (_d, root, folder) = setup();
        let sd = sdir(&root, &folder);
        std::fs::write(sd.join(format!("{SID}.jsonl")), "{}\n").unwrap();
        std::fs::create_dir_all(sd.join(SID).join("subagents")).unwrap();
        let f = locate(&root, &[folder.clone()], SID).unwrap().unwrap();
        assert_eq!(f.file, sd.join(format!("{SID}.jsonl")));
        assert_eq!(f.dir, Some(sd.join(SID)));
    }

    #[test]
    fn missing_file_is_none_and_no_dir_is_fine() {
        let (_d, root, folder) = setup();
        assert_eq!(locate(&root, &[folder.clone()], SID).unwrap(), None);
        std::fs::write(sdir(&root, &folder).join(format!("{SID}.jsonl")), "{}\n").unwrap();
        assert_eq!(locate(&root, &[folder], SID).unwrap().unwrap().dir, None);
    }

    #[test]
    fn invalid_ids_are_refused() {
        let (_d, root, folder) = setup();
        for bad in ["../x", "a/b", "", "..", "x.jsonl/../../y"] {
            assert!(locate(&root, &[folder.clone()], bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn symlinked_session_file_is_refused() {
        let (d, root, folder) = setup();
        let outside = d.path().join("fuori.jsonl");
        std::fs::write(&outside, "{}\n").unwrap();
        std::os::unix::fs::symlink(&outside, sdir(&root, &folder).join(format!("{SID}.jsonl"))).unwrap();
        assert!(locate(&root, &[folder], SID).is_err());
        assert!(outside.exists());
    }

    #[test]
    fn symlinked_sibling_dir_is_left_alone() {
        let (d, root, folder) = setup();
        let sd = sdir(&root, &folder);
        std::fs::write(sd.join(format!("{SID}.jsonl")), "{}\n").unwrap();
        let outside = d.path().join("fuori");
        std::fs::create_dir_all(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, sd.join(SID)).unwrap();
        assert_eq!(locate(&root, &[folder], SID).unwrap().unwrap().dir, None);
    }

    #[test]
    fn session_dir_that_is_a_link_out_of_the_root_is_refused() {
        let (d, root, folder) = setup();
        let sd = sdir(&root, &folder);
        let outside = d.path().join("fuori");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join(format!("{SID}.jsonl")), "{}\n").unwrap();
        std::fs::remove_dir_all(&sd).unwrap();
        std::os::unix::fs::symlink(&outside, &sd).unwrap();
        assert!(locate(&root, &[folder], SID).is_err());
    }

    #[test]
    fn dialog_message_is_plain_single_line_and_bounded() {
        let m = dialog_message("  Titolo\n<b>x</b>\u{7}  con   spazi ");
        assert_eq!(m, format!("Titolo <b>x</b> con spazi\n\n{DIALOG_NOTE}"));
        assert!(dialog_message("").starts_with(UNTITLED));
        let long = dialog_message(&"a".repeat(500));
        assert_eq!(long.lines().next().unwrap().chars().count(), TITLE_SHOWN_MAX);
    }

    #[test]
    fn one_prompt_per_key_at_a_time() {
        let p = DeletePrompts::default();
        let g = p.acquire("p1").unwrap();
        assert!(p.acquire("p1").is_none());
        assert!(p.acquire("p2").is_some());
        drop(g);
        assert!(p.acquire("p1").is_some());
    }
}
