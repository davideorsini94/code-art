//! Git in scrittura: stage, unstage, scarto, commit, branch, merge e `init`. Tutto in progetti fidati, `repo` tra
//! le radici di `git_repos`, percorsi relativi validati e passati come pathspec letterali da stdin, nomi di branch
//! validati con `git check-ref-format --branch`. Una sola scrittura alla volta per repository (`lock_repo`).
use super::cmd::{git, git_write, git_write_ok, READ_TIMEOUT};
use super::{check_rel, resolve_repo};
pub use crate::claude::delete::{SystemTrasher, Trasher};
use crate::store::{Result, Store};
use serde::Serialize;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use tauri::{AppHandle, Manager, WebviewWindow};

pub const BUSY: &str = "Un'altra operazione git è in corso";
pub const BTN_CANCEL: &str = "Annulla";
const MAX_PATHS: usize = 10_000;
const MAX_MESSAGE_BYTES: usize = 64 * 1024;
const MAX_BRANCH_BYTES: usize = 255;

/// Conferma prima di un'azione distruttiva: dialogo nativo nell'app, finto nei test.
pub trait Confirm: Send + Sync {
    /// `true` solo se l'utente sceglie `action` (il pulsante predefinito è «Annulla»).
    fn confirm(&self, title: &str, message: &str, action: &str) -> bool;
}

/// Dialogo nativo con «Annulla» predefinito, come `conv_delete`.
pub struct NativeConfirm {
    pub app: AppHandle,
    pub window: WebviewWindow,
}

impl Confirm for NativeConfirm {
    fn confirm(&self, title: &str, message: &str, action: &str) -> bool {
        use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind, MessageDialogResult};
        let answer = self
            .app
            .dialog()
            .message(message)
            .title(title)
            .kind(MessageDialogKind::Warning)
            .buttons(MessageDialogButtons::OkCancelCustom(BTN_CANCEL.into(), action.into()))
            .parent(&self.window)
            .blocking_show_with_result();
        matches!(answer, MessageDialogResult::Custom(l) if l == action)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MergeKind {
    FastForward,
    Merged,
    Conflicts,
    UpToDate,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeOutcome {
    pub kind: MergeKind,
    pub conflicts: Vec<String>,
}

// ---------- un'operazione alla volta per repository ----------

fn locks() -> &'static Mutex<HashSet<PathBuf>> {
    static LOCKS: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();
    LOCKS.get_or_init(Default::default)
}

/// Occupa il repository finché vive (anche se passato a un altro thread, come le operazioni remote).
pub struct RepoGuard(PathBuf);

impl Drop for RepoGuard {
    fn drop(&mut self) {
        crate::lock(locks()).remove(&self.0);
    }
}

/// Occupa `dir` (chiave: percorso canonico, così due progetti sullo stesso repository si escludono) o `BUSY`.
pub fn lock_repo(dir: &Path) -> Result<RepoGuard> {
    let key = dir.canonicalize().map_err(|_| "repository non valido".to_string())?;
    if crate::lock(locks()).insert(key.clone()) { Ok(RepoGuard(key)) } else { Err(BUSY.into()) }
}

// ---------- validazione ----------

/// Percorsi relativi al repository: non vuoti, senza `..`/`.`/radice, mai con `-` iniziale, e la cartella che li
/// contiene, se esiste, sta dentro il repository (un link simbolico a una cartella esterna è rifiutato).
fn check_paths(dir: &Path, paths: &[String]) -> Result<()> {
    if paths.is_empty() || paths.len() > MAX_PATHS {
        return Err("elenco di percorsi non valido".into());
    }
    let base = dir.canonicalize().map_err(|_| "repository non valido".to_string())?;
    for p in paths {
        check_rel(p)?;
        if p.starts_with('-') {
            return Err("percorso non valido".into());
        }
        let parent = base.join(p).parent().map(Path::to_path_buf).unwrap_or_else(|| base.clone());
        if parent.canonicalize().is_ok_and(|real| !real.starts_with(&base)) {
            return Err("percorso fuori dal repository".into());
        }
    }
    Ok(())
}

/// Pathspec per `--pathspec-from-file=- --pathspec-file-nul` (letterali grazie a `GIT_LITERAL_PATHSPECS`).
fn pathspec(paths: &[String]) -> Option<Vec<u8>> {
    Some(paths.join("\0").into_bytes())
}

const FROM_STDIN: [&str; 2] = ["--pathspec-from-file=-", "--pathspec-file-nul"];

/// Nome di branch dalla pagina: mai un'opzione, mai la sintassi `@{…}`, e `check-ref-format --branch` deve
/// restituirlo identico (niente espansioni).
fn check_branch(dir: &Path, name: &str) -> Result<()> {
    let bad = || "nome di branch non valido".to_string();
    if name.is_empty() || name.len() > MAX_BRANCH_BYTES || name.starts_with('-') || name.contains("@{") || name.contains('\0') || name == "HEAD" {
        return Err(bad());
    }
    let out = git(dir, &["check-ref-format", "--branch", name], READ_TIMEOUT)?;
    if out.status != 0 || String::from_utf8_lossy(&out.stdout).trim_end_matches('\n') != name {
        return Err(bad());
    }
    Ok(())
}

fn ref_exists(dir: &Path, full: &str) -> Result<bool> {
    Ok(git(dir, &["show-ref", "--verify", "--quiet", full], READ_TIMEOUT)?.status == 0)
}

fn current_branch(dir: &Path) -> Result<Option<String>> {
    let out = git(dir, &["symbolic-ref", "--quiet", "--short", "HEAD"], READ_TIMEOUT)?;
    Ok((out.status == 0).then(|| String::from_utf8_lossy(&out.stdout).trim_end_matches('\n').to_string()))
}

fn head_exists(dir: &Path) -> Result<bool> {
    Ok(git(dir, &["rev-parse", "--verify", "--quiet", "HEAD"], READ_TIMEOUT)?.status == 0)
}

/// Repository del progetto, già occupato per questa operazione.
fn open_for_write(store: &Store, project_id: &str, repo: &str) -> Result<(PathBuf, RepoGuard)> {
    let dir = resolve_repo(store, project_id, repo)?;
    let guard = lock_repo(&dir)?;
    Ok((dir, guard))
}

fn nul_list(bytes: &[u8]) -> Vec<String> {
    bytes.split(|b| *b == 0).filter(|s| !s.is_empty()).map(|s| String::from_utf8_lossy(s).into_owned()).collect()
}

/// `ls-files -z <extra> -- <percorsi>` a blocchi (`ls-files` non legge i pathspec da stdin; i blocchi restano sotto
/// il limite della riga di comando). I percorsi sono già validati e letterali (`GIT_LITERAL_PATHSPECS`).
fn ls_files(dir: &Path, extra: &[&str], paths: &[String]) -> Result<HashSet<String>> {
    let mut found = HashSet::new();
    for chunk in paths.chunks(500) {
        let mut args = vec!["ls-files", "-z"];
        args.extend_from_slice(extra);
        args.push("--");
        args.extend(chunk.iter().map(String::as_str));
        found.extend(nul_list(&git_write_ok(dir, &args, None)?));
    }
    Ok(found)
}

// ---------- operazioni ----------

pub fn stage(store: &Store, project_id: &str, repo: &str, paths: &[String]) -> Result<()> {
    let dir = resolve_repo(store, project_id, repo)?;
    check_paths(&dir, paths)?;
    let _g = lock_repo(&dir)?;
    // `-A`: anche le cancellazioni
    git_write_ok(&dir, &["add", "-A", FROM_STDIN[0], FROM_STDIN[1]], pathspec(paths)).map(drop)
}

pub fn unstage(store: &Store, project_id: &str, repo: &str, paths: &[String]) -> Result<()> {
    let dir = resolve_repo(store, project_id, repo)?;
    check_paths(&dir, paths)?;
    let _g = lock_repo(&dir)?;
    if head_exists(&dir)? {
        git_write_ok(&dir, &["restore", "--staged", FROM_STDIN[0], FROM_STDIN[1]], pathspec(paths)).map(drop)
    } else {
        // repository senza commit: non c'è HEAD da cui ripristinare, si toglie dall'indice
        git_write_ok(&dir, &["rm", "--cached", "-q", "-r", FROM_STDIN[0], FROM_STDIN[1]], pathspec(paths)).map(drop)
    }
}

/// Testo della conferma di `discard`.
pub fn discard_message(n: usize) -> String {
    format!("Scartare le modifiche a {n} file? Non si possono recuperare.")
}

/// Scarta le modifiche non in stage: i file tracciati tornano come nell'indice, quelli non tracciati vanno nel
/// Cestino (mai cancellati). `Ok(false)` = l'utente ha annullato, niente toccato.
pub fn discard(store: &Store, project_id: &str, repo: &str, paths: &[String], confirm: &dyn Confirm, trasher: &dyn Trasher) -> Result<bool> {
    let dir = resolve_repo(store, project_id, repo)?;
    check_paths(&dir, paths)?;
    let _g = lock_repo(&dir)?;
    let tracked = ls_files(&dir, &[], paths)?;
    let untracked = ls_files(&dir, &["--others", "--exclude-standard"], paths)?;
    let (keep, loose): (Vec<String>, Vec<String>) = paths.iter().cloned().partition(|p| tracked.contains(p));
    if let Some(p) = loose.iter().find(|p| !untracked.contains(*p)) {
        return Err(format!("nessuna modifica da scartare: {p}"));
    }
    if !confirm.confirm("Scartare le modifiche?", &discard_message(paths.len()), "Scarta") {
        return Ok(false);
    }
    if !keep.is_empty() {
        git_write_ok(&dir, &["restore", "--worktree", FROM_STDIN[0], FROM_STDIN[1]], pathspec(&keep))?;
    }
    let base = dir.canonicalize().map_err(|e| e.to_string())?;
    for p in &loose {
        trasher.trash(&base.join(p))?;
    }
    Ok(true)
}

/// Commit con il messaggio da stdin (`-F -`); gli hook del repository girano (progetto fidato). Hash breve.
pub fn commit(store: &Store, project_id: &str, repo: &str, message: &str, amend: bool) -> Result<String> {
    let (dir, _g) = open_for_write(store, project_id, repo)?;
    if message.trim().is_empty() {
        return Err("Il messaggio di commit è vuoto".into());
    }
    if message.len() > MAX_MESSAGE_BYTES || message.contains('\0') {
        return Err("Messaggio di commit non valido".into());
    }
    let mut args = vec!["commit", "-F", "-"];
    if amend {
        args.push("--amend");
    }
    let out = git_write(&dir, &args, Some(message.as_bytes().to_vec()))?;
    if out.status != 0 {
        let mut why = out.stderr.trim().to_string();
        if why.is_empty() {
            why = String::from_utf8_lossy(&out.stdout).trim().to_string();
        }
        return Err(format!("git: {why}"));
    }
    let short = git_write_ok(&dir, &["rev-parse", "--short", "HEAD"], None)?;
    Ok(String::from_utf8_lossy(&short).trim().to_string())
}

/// Passa a `branch`: locale, oppure remoto (`origin/x`) creando il branch locale che lo segue.
pub fn checkout(store: &Store, project_id: &str, repo: &str, branch: &str) -> Result<()> {
    let dir = resolve_repo(store, project_id, repo)?;
    check_branch(&dir, branch)?;
    let _g = lock_repo(&dir)?;
    if ref_exists(&dir, &format!("refs/heads/{branch}"))? {
        git_write_ok(&dir, &["switch", "--no-guess", branch], None).map(drop)
    } else if ref_exists(&dir, &format!("refs/remotes/{branch}"))? {
        git_write_ok(&dir, &["switch", "--track", branch], None).map(drop)
    } else {
        Err(format!("Il branch «{branch}» non esiste"))
    }
}

pub fn create_branch(store: &Store, project_id: &str, repo: &str, name: &str, from: Option<&str>, checkout: bool) -> Result<()> {
    let dir = resolve_repo(store, project_id, repo)?;
    check_branch(&dir, name)?;
    if let Some(f) = from {
        check_branch(&dir, f)?;
    }
    let _g = lock_repo(&dir)?;
    let mut args = if checkout { vec!["switch", "--no-guess", "-c", name] } else { vec!["branch", name] };
    args.extend(from);
    git_write_ok(&dir, &args, None).map(drop)
}

/// Elimina un branch locale con `-d` (mai `-D`), dopo la conferma; mai il branch corrente.
/// `Ok(false)` = annullato.
pub fn delete_branch(store: &Store, project_id: &str, repo: &str, name: &str, confirm: &dyn Confirm) -> Result<bool> {
    let dir = resolve_repo(store, project_id, repo)?;
    check_branch(&dir, name)?;
    let _g = lock_repo(&dir)?;
    if current_branch(&dir)?.as_deref() == Some(name) {
        return Err("Non si può eliminare il branch corrente".into());
    }
    if !ref_exists(&dir, &format!("refs/heads/{name}"))? {
        return Err(format!("Il branch «{name}» non esiste"));
    }
    if !confirm.confirm("Eliminare il branch?", &format!("«{name}»\n\nIl branch locale verrà eliminato."), "Elimina") {
        return Ok(false);
    }
    let out = git_write(&dir, &["branch", "-d", name], None)?;
    if out.status == 0 {
        Ok(true)
    } else if out.stderr.contains("not fully merged") {
        Err(format!("Il branch «{name}» non è unito: non è stato eliminato"))
    } else {
        Err(format!("git: {}", out.stderr.trim()))
    }
}

pub fn merge(store: &Store, project_id: &str, repo: &str, branch: &str) -> Result<MergeOutcome> {
    let dir = resolve_repo(store, project_id, repo)?;
    check_branch(&dir, branch)?;
    let _g = lock_repo(&dir)?;
    let out = git_write(&dir, &["merge", "--no-edit", branch], None)?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    if out.status == 0 {
        let kind = if stdout.contains("Already up to date") {
            MergeKind::UpToDate
        } else if stdout.contains("Fast-forward") {
            MergeKind::FastForward
        } else {
            MergeKind::Merged
        };
        return Ok(MergeOutcome { kind, conflicts: Vec::new() });
    }
    let conflicts = nul_list(&git_write_ok(&dir, &["diff", "--name-only", "--diff-filter=U", "-z"], None)?);
    if conflicts.is_empty() {
        let why = if out.stderr.trim().is_empty() { stdout.trim().to_string() } else { out.stderr.trim().to_string() };
        return Err(format!("git: {why}"));
    }
    Ok(MergeOutcome { kind: MergeKind::Conflicts, conflicts })
}

pub fn merge_abort(store: &Store, project_id: &str, repo: &str) -> Result<()> {
    let (dir, _g) = open_for_write(store, project_id, repo)?;
    git_write_ok(&dir, &["merge", "--abort"], None).map(drop)
}

/// `git init` in una cartella del progetto (fidato) che non è ancora un repository.
pub fn init(store: &Store, project_id: &str, folder: &str) -> Result<()> {
    let p = super::trusted_project(store, project_id)?;
    let want = Path::new(folder).canonicalize().map_err(|_| "cartella non valida".to_string())?;
    let dir = p
        .folders
        .iter()
        .find(|f| Path::new(f).canonicalize().is_ok_and(|c| c == want))
        .ok_or_else(|| "la cartella non appartiene al progetto".to_string())?;
    if super::is_repo_root(dir) {
        return Err("la cartella è già un repository git".into());
    }
    let _g = lock_repo(&want)?;
    git_write_ok(Path::new(dir), &["init", "-q"], None).map(drop)
}

// ---------- comandi ----------

use crate::projects::{off_main, window_project_id};

#[tauri::command]
pub async fn git_stage(app: AppHandle, window: WebviewWindow, repo: String, paths: Vec<String>) -> Result<()> {
    let id = window_project_id(&window)?;
    off_main(move || stage(&app.state::<Store>(), &id, &repo, &paths)).await
}

#[tauri::command]
pub async fn git_unstage(app: AppHandle, window: WebviewWindow, repo: String, paths: Vec<String>) -> Result<()> {
    let id = window_project_id(&window)?;
    off_main(move || unstage(&app.state::<Store>(), &id, &repo, &paths)).await
}

/// `false` = annullato dall'utente.
#[tauri::command]
pub async fn git_discard(app: AppHandle, window: WebviewWindow, repo: String, paths: Vec<String>) -> Result<bool> {
    let id = window_project_id(&window)?;
    off_main(move || {
        let c = NativeConfirm { app: app.clone(), window };
        discard(&app.state::<Store>(), &id, &repo, &paths, &c, &SystemTrasher)
    })
    .await
}

#[tauri::command]
pub async fn git_commit(app: AppHandle, window: WebviewWindow, repo: String, message: String, amend: bool) -> Result<String> {
    let id = window_project_id(&window)?;
    off_main(move || commit(&app.state::<Store>(), &id, &repo, &message, amend)).await
}

#[tauri::command]
pub async fn git_checkout(app: AppHandle, window: WebviewWindow, repo: String, branch: String) -> Result<()> {
    let id = window_project_id(&window)?;
    off_main(move || checkout(&app.state::<Store>(), &id, &repo, &branch)).await
}

#[tauri::command]
pub async fn git_create_branch(app: AppHandle, window: WebviewWindow, repo: String, name: String, from: Option<String>, checkout: bool) -> Result<()> {
    let id = window_project_id(&window)?;
    off_main(move || create_branch(&app.state::<Store>(), &id, &repo, &name, from.as_deref(), checkout)).await
}

/// `false` = annullato dall'utente.
#[tauri::command]
pub async fn git_delete_branch(app: AppHandle, window: WebviewWindow, repo: String, name: String) -> Result<bool> {
    let id = window_project_id(&window)?;
    off_main(move || {
        let c = NativeConfirm { app: app.clone(), window };
        delete_branch(&app.state::<Store>(), &id, &repo, &name, &c)
    })
    .await
}

#[tauri::command]
pub async fn git_merge(app: AppHandle, window: WebviewWindow, repo: String, branch: String) -> Result<MergeOutcome> {
    let id = window_project_id(&window)?;
    off_main(move || merge(&app.state::<Store>(), &id, &repo, &branch)).await
}

#[tauri::command]
pub async fn git_merge_abort(app: AppHandle, window: WebviewWindow, repo: String) -> Result<()> {
    let id = window_project_id(&window)?;
    off_main(move || merge_abort(&app.state::<Store>(), &id, &repo)).await
}

#[tauri::command]
pub async fn git_init(app: AppHandle, window: WebviewWindow, folder: String) -> Result<()> {
    let id = window_project_id(&window)?;
    off_main(move || init(&app.state::<Store>(), &id, &folder)).await
}

#[cfg(test)]
mod tests;
