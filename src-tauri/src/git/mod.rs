//! Git in lettura: repository del progetto, stato, diff, branch e cronologia. Tutto passa dal processo `git`
//! (che esegue hook e `core.fsmonitor` del repository), quindi solo in progetti fidati.
pub mod cmd;
pub mod remote;
pub mod status;
#[cfg(test)]
pub(crate) mod testutil;
pub mod write;

use crate::store::{Project, Result, Store};
use cmd::{git, git_ok, READ_TIMEOUT};
use serde::Serialize;
use status::{parse_porcelain_v2, RepoStatus, MAX_ENTRIES};
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager, WebviewWindow};

/// Oltre questa dimensione un file nel diff è trattato come binario.
const MAX_DIFF_BYTES: u64 = 2 * 1024 * 1024;
const MAX_LOG: usize = 200;

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffText {
    pub before: String,
    pub after: String,
    pub binary: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Branch {
    pub name: String,
    pub remote: bool,
    pub current: bool,
    pub upstream: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Commit {
    pub hash: String,
    pub short: String,
    pub subject: String,
    pub author: String,
    pub date_iso: String,
}

// ---------- progetto e repository ----------

/// Progetto fidato (git esegue codice del repository).
fn trusted_project(store: &Store, project_id: &str) -> Result<Project> {
    crate::trust::ensure_project_trusted(store, project_id)?;
    store.get_project(project_id)?.ok_or_else(|| crate::trust::UNTRUSTED.to_string())
}

/// `folder` stessa è la radice di un repository (`.git` cartella, o file per i worktree).
fn is_repo_root(folder: &str) -> bool {
    Path::new(folder).join(".git").exists()
}

/// Profondità massima, sotto la cartella del progetto, a cui si cercano repository annidati.
const NESTED_DEPTH: usize = 3;
/// Tetto ai repository di un progetto (radici e annidati insieme).
const MAX_REPOS: usize = 50;
/// Cartelle mai attraversate nella ricerca (oltre a quelle nascoste).
const SKIP_DIRS: &[&str] = &["node_modules", "target", "vendor", "venv", "dist", "build"];
/// Voci di cartella lette al massimo, per cartella del progetto: una cartella enorme (`~`) non si visita tutta.
const MAX_VISITED: usize = 2000;
/// Quanto vale il risultato della ricerca di una cartella.
const NESTED_TTL: Duration = Duration::from_secs(2);

/// Repository sotto `dir` (esclusa), in ordine di percorso; non scende nei repository trovati né nei link simbolici.
fn scan_nested(dir: &Path, depth: usize, budget: &mut usize, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let entries: Vec<_> = rd.flatten().take(*budget).collect();
    *budget = budget.saturating_sub(entries.len().max(1));
    let mut subs: Vec<(String, PathBuf)> = entries
        .into_iter()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir())) // `DirEntry::file_type` non segue i link
        .filter_map(|e| Some((e.file_name().into_string().ok()?, e.path())))
        .filter(|(n, _)| !n.starts_with('.') && !SKIP_DIRS.contains(&n.as_str()))
        .collect();
    subs.sort();
    for (_, p) in subs {
        if out.len() >= MAX_REPOS {
            return;
        }
        if p.join(".git").exists() {
            out.push(p);
        } else if depth < NESTED_DEPTH && *budget > 0 {
            scan_nested(&p, depth + 1, budget, out);
        }
    }
}

type NestedCache = HashMap<String, (Instant, Vec<String>)>;
static NESTED_CACHE: OnceLock<Mutex<NestedCache>> = OnceLock::new();

/// Repository annidati in `folder` (che non è essa stessa un repository), con una cache breve:
/// la ricerca gira a ogni comando git e a ogni evento del watcher. Con `stale_ok` un risultato scaduto si
/// riusa così com'è (il watcher non rifà mai la visita se ne ha una già fatta).
fn nested_repos(folder: &str, stale_ok: bool) -> Vec<String> {
    let cache = NESTED_CACHE.get_or_init(Default::default);
    if let Some((at, v)) = cache.lock().ok().and_then(|c| c.get(folder).cloned()) {
        if stale_ok || at.elapsed() < NESTED_TTL {
            return v;
        }
    }
    let mut found = Vec::new();
    scan_nested(Path::new(folder), 1, &mut MAX_VISITED.clone(), &mut found);
    let v: Vec<String> = found.into_iter().map(|p| p.to_string_lossy().into_owned()).collect();
    if let Ok(mut c) = cache.lock() {
        c.insert(folder.to_string(), (Instant::now(), v.clone()));
    }
    v
}

/// Dimentica le ricerche in cache (dopo `git init`).
pub(crate) fn forget_nested() {
    if let Some(c) = NESTED_CACHE.get() {
        if let Ok(mut c) = c.lock() {
            c.clear();
        }
    }
}

/// Repository di un insieme di cartelle senza controllo di fiducia: prima le cartelle che sono radici, poi gli
/// annidati (dentro cartelle che non lo sono) in ordine di percorso; senza doppioni (percorso canonico, vale il
/// primo); al massimo `MAX_REPOS`.
pub(crate) fn repos_in(folders: &[String]) -> Vec<String> {
    collect_repos(folders, false)
}

/// Come `repos_in`, ma riusa anche i risultati scaduti: per il thread del watcher, che non deve fermarsi su una visita.
pub(crate) fn repos_in_cached(folders: &[String]) -> Vec<String> {
    collect_repos(folders, true)
}

fn collect_repos(folders: &[String], stale_ok: bool) -> Vec<String> {
    let roots = folders.iter().filter(|f| is_repo_root(f)).cloned();
    let mut nested: Vec<String> = folders.iter().filter(|f| !is_repo_root(f)).flat_map(|f| nested_repos(f, stale_ok)).collect();
    nested.sort();
    let mut seen = std::collections::HashSet::new();
    roots
        .chain(nested)
        .filter(|r| seen.insert(Path::new(r).canonicalize().unwrap_or_else(|_| PathBuf::from(r))))
        .take(MAX_REPOS)
        .collect()
}

/// Repository del progetto: le cartelle che sono radici, poi quelli annidati (fino a 3 livelli).
pub fn repos_of(store: &Store, project_id: &str) -> Result<Vec<String>> {
    let p = trusted_project(store, project_id)?;
    Ok(repos_in(&p.folders))
}

/// `repo` deve essere una delle radici di `repos_of` (confronto sui percorsi canonici).
pub(crate) fn resolve_repo(store: &Store, project_id: &str, repo: &str) -> Result<PathBuf> {
    // prima la fiducia (altrimenti l'interfaccia non mostra "Mi fido"), poi l'argomento della pagina
    let repos = repos_of(store, project_id)?;
    let want = Path::new(repo).canonicalize().map_err(|_| "repository non valido".to_string())?;
    repos
        .into_iter()
        .find(|r| Path::new(r).canonicalize().is_ok_and(|c| c == want))
        .map(PathBuf::from)
        .ok_or_else(|| "il repository non appartiene al progetto".to_string())
}

/// Percorso relativo al repository, senza `..`, `.` né radice.
fn check_rel(path: &str) -> Result<()> {
    let p = Path::new(path);
    if path.is_empty() || path.contains('\0') || !p.components().all(|c| matches!(c, Component::Normal(_))) {
        return Err("percorso non valido".into());
    }
    Ok(())
}

// ---------- operazioni ----------

/// Errori di `run_tool` che il fallback a meno dettaglio può risolvere.
fn needs_fallback(e: &str) -> bool {
    e.contains("output oltre") || e.contains("tempo scaduto")
}

pub fn status(store: &Store, project_id: &str, repo: &str) -> Result<RepoStatus> {
    let dir = resolve_repo(store, project_id, repo)?;
    // output troppo grande o troppo lento (decine di migliaia di file non tracciati): si degrada a meno dettaglio
    let mut out = None;
    let mut degraded = false;
    for mode in ["--untracked-files=all", "--untracked-files=normal", "--untracked-files=no"] {
        match git_ok(&dir, &["status", "--porcelain=v2", "-z", "--branch", mode], READ_TIMEOUT) {
            Ok(o) => {
                out = Some(o);
                break;
            }
            Err(e) if needs_fallback(&e) => degraded = true,
            Err(e) => return Err(e),
        }
    }
    let out = out.ok_or_else(|| "git: stato troppo grande".to_string())?;
    let (h, mut entries) = parse_porcelain_v2(&out);
    let truncated = degraded || entries.len() > MAX_ENTRIES;
    entries.truncate(MAX_ENTRIES);
    let merge_head = git_ok(&dir, &["rev-parse", "--git-path", "MERGE_HEAD"], READ_TIMEOUT)?;
    let merge_head = String::from_utf8_lossy(&merge_head);
    let merging = dir.join(merge_head.trim_end_matches('\n')).exists();
    Ok(RepoStatus {
        root: repo.to_string(),
        branch: h.branch,
        upstream: h.upstream,
        ahead: h.ahead,
        behind: h.behind,
        detached: h.detached,
        merging,
        entries,
        truncated,
    })
}

enum Side {
    Text(String),
    Binary,
}

fn decode(bytes: Vec<u8>) -> Side {
    if bytes.len() as u64 > MAX_DIFF_BYTES || bytes.contains(&0) {
        return Side::Binary;
    }
    String::from_utf8(bytes).map_or(Side::Binary, Side::Text)
}

/// Contenuto di `spec` (`HEAD:percorso`, `:0:percorso`) o vuoto se non esiste.
/// Messaggi di git (con `LC_ALL=C`) per "questo oggetto non esiste": percorso assente in HEAD o nell'indice,
/// oppure HEAD non nato (repository senza commit). Qualsiasi altro errore è un errore vero.
fn is_missing(stderr: &str) -> bool {
    stderr.contains("does not exist in 'HEAD'")
        || stderr.contains("does not exist (neither on disk nor in the index)")
        || stderr.contains("exists on disk, but not in")
        || stderr.contains("invalid object name 'HEAD'")
}

/// Contenuto di `spec` (`HEAD:percorso`, `:0:percorso`): vuoto solo se l'oggetto non esiste; prima si legge la
/// dimensione, così un blob oltre `MAX_DIFF_BYTES` è `Binary` senza essere caricato.
fn show(dir: &Path, spec: &str) -> Result<Side> {
    let sz = git(dir, &["cat-file", "-s", spec], READ_TIMEOUT)?;
    if sz.status != 0 {
        return if is_missing(&sz.stderr) { Ok(Side::Text(String::new())) } else { Err(format!("git: {}", sz.stderr.trim())) };
    }
    let size: u64 = String::from_utf8_lossy(&sz.stdout).trim().parse().map_err(|_| "git: dimensione illeggibile".to_string())?;
    if size > MAX_DIFF_BYTES {
        return Ok(Side::Binary);
    }
    Ok(decode(git_ok(dir, &["show", spec], READ_TIMEOUT)?))
}

fn disk(dir: &Path, rel: &str) -> Result<Side> {
    let full = crate::fs_tree::ensure_inside(&[dir.to_path_buf()], &dir.join(rel)).map_err(|e| e.message)?;
    match std::fs::metadata(&full) {
        Ok(m) if m.is_file() && m.len() > MAX_DIFF_BYTES => Ok(Side::Binary),
        Ok(m) if m.is_file() => Ok(decode(std::fs::read(&full).map_err(|e| e.to_string())?)),
        _ => Ok(Side::Text(String::new())),
    }
}

/// Prima = `HEAD` (staged) o indice (non staged); dopo = indice (staged) o disco (non staged).
#[cfg(test)]
pub fn diff(store: &Store, project_id: &str, repo: &str, path: &str, staged: bool) -> Result<DiffText> {
    diff_with_orig(store, project_id, repo, path, None, staged)
}

/// Come `diff`; per una rinomina in stage `orig_path` (il nome vecchio) dà il lato prima da `HEAD`.
pub fn diff_with_orig(store: &Store, project_id: &str, repo: &str, path: &str, orig_path: Option<&str>, staged: bool) -> Result<DiffText> {
    check_rel(path)?;
    if let Some(o) = orig_path {
        check_rel(o)?;
    }
    let dir = resolve_repo(store, project_id, repo)?;
    let (before, after) = if staged {
        let old = orig_path.unwrap_or(path);
        (show(&dir, &format!("HEAD:{old}"))?, show(&dir, &format!(":0:{path}"))?)
    } else {
        (show(&dir, &format!(":0:{path}"))?, disk(&dir, path)?)
    };
    Ok(match (before, after) {
        (Side::Text(before), Side::Text(after)) => DiffText { before, after, binary: false },
        _ => DiffText { before: String::new(), after: String::new(), binary: true },
    })
}

pub fn branches(store: &Store, project_id: &str, repo: &str) -> Result<Vec<Branch>> {
    let dir = resolve_repo(store, project_id, repo)?;
    let out = git_ok(
        &dir,
        &["for-each-ref", "--format=%(refname)%00%(HEAD)%00%(upstream:short)", "refs/heads", "refs/remotes"],
        READ_TIMEOUT,
    )?;
    let mut list = Vec::new();
    for line in String::from_utf8_lossy(&out).lines() {
        let mut f = line.split('\0');
        let (Some(full), Some(head), Some(up)) = (f.next(), f.next(), f.next()) else { continue };
        let (name, remote) = match (full.strip_prefix("refs/heads/"), full.strip_prefix("refs/remotes/")) {
            (Some(n), _) => (n, false),
            (_, Some(n)) => (n, true),
            _ => continue,
        };
        if remote && name.ends_with("/HEAD") {
            continue;
        }
        list.push(Branch { name: name.into(), remote, current: head == "*", upstream: (!up.is_empty()).then(|| up.into()) });
    }
    Ok(list)
}

pub fn log(store: &Store, project_id: &str, repo: &str, limit: usize) -> Result<Vec<Commit>> {
    let dir = resolve_repo(store, project_id, repo)?;
    let n = limit.clamp(1, MAX_LOG).to_string();
    let out = git(&dir, &["log", "-n", &n, "--format=%H%x1f%h%x1f%s%x1f%an%x1f%aI%x1e"], READ_TIMEOUT)?;
    if out.status != 0 {
        // repository senza commit
        if out.stderr.contains("does not have any commits") {
            return Ok(Vec::new());
        }
        return Err(format!("git: {}", out.stderr.trim()));
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .split('\x1e')
        .filter_map(|rec| {
            let mut f = rec.trim_start_matches('\n').split('\x1f');
            Some(Commit { hash: f.next()?.into(), short: f.next()?.into(), subject: f.next()?.into(), author: f.next()?.into(), date_iso: f.next()?.into() })
        })
        .collect())
}

// ---------- comandi ----------

#[tauri::command]
pub async fn git_repos(app: AppHandle, window: WebviewWindow) -> Result<Vec<String>> {
    let id = crate::projects::window_project_id(&window)?;
    crate::projects::off_main(move || repos_of(&app.state::<Store>(), &id)).await
}

#[tauri::command]
pub async fn git_status(app: AppHandle, window: WebviewWindow, repo: String) -> Result<RepoStatus> {
    let id = crate::projects::window_project_id(&window)?;
    crate::projects::off_main(move || status(&app.state::<Store>(), &id, &repo)).await
}

#[tauri::command]
pub async fn git_diff(app: AppHandle, window: WebviewWindow, repo: String, path: String, orig_path: Option<String>, staged: bool) -> Result<DiffText> {
    let id = crate::projects::window_project_id(&window)?;
    crate::projects::off_main(move || diff_with_orig(&app.state::<Store>(), &id, &repo, &path, orig_path.as_deref(), staged)).await
}

#[tauri::command]
pub async fn git_branches(app: AppHandle, window: WebviewWindow, repo: String) -> Result<Vec<Branch>> {
    let id = crate::projects::window_project_id(&window)?;
    crate::projects::off_main(move || branches(&app.state::<Store>(), &id, &repo)).await
}

#[tauri::command]
pub async fn git_log(app: AppHandle, window: WebviewWindow, repo: String, limit: usize) -> Result<Vec<Commit>> {
    let id = crate::projects::window_project_id(&window)?;
    crate::projects::off_main(move || log(&app.state::<Store>(), &id, &repo, limit)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::testutil::*;
    use std::process::Command;

    #[test]
    fn names_with_spaces_accents_newline() {
        let d = repo();
        let names = ["con spazio.txt", "perché è così.txt", "a\nb.txt"];
        for n in names {
            std::fs::write(d.path().join(n), "x").unwrap();
        }
        let (_s, st) = project(&[d.path()]);
        let r = status(&st, "p", &root(&d)).unwrap();
        let mut got: Vec<_> = r.entries.iter().map(|e| e.path.clone()).collect();
        got.sort();
        let mut want: Vec<_> = names.iter().map(|n| n.to_string()).collect();
        want.sort();
        assert_eq!(got, want);
        assert!(r.entries.iter().all(|e| e.untracked));
        assert_eq!(r.branch.as_deref(), Some("main"));
    }

    #[test]
    fn status_reports_staged_unstaged_and_rename() {
        let d = repo();
        std::fs::write(d.path().join("a.txt"), "one\n").unwrap();
        std::fs::write(d.path().join("old.txt"), "keep me around\nplease\n").unwrap();
        commit_all(d.path(), "init");
        std::fs::write(d.path().join("a.txt"), "two\n").unwrap();
        g(d.path(), &["mv", "old.txt", "new.txt"]);
        let (_s, st) = project(&[d.path()]);
        let r = status(&st, "p", &root(&d)).unwrap();
        let a = r.entries.iter().find(|e| e.path == "a.txt").unwrap();
        assert_eq!((a.staged, a.unstaged), (None, Some('M')));
        let n = r.entries.iter().find(|e| e.path == "new.txt").unwrap();
        assert_eq!((n.staged, n.orig_path.as_deref()), (Some('R'), Some("old.txt")));
        assert!(!r.merging && !r.detached && !r.truncated);
    }

    #[test]
    fn detached_head() {
        let d = repo();
        std::fs::write(d.path().join("a.txt"), "1").unwrap();
        commit_all(d.path(), "c1");
        g(d.path(), &["checkout", "-q", "--detach"]);
        let (_s, st) = project(&[d.path()]);
        let r = status(&st, "p", &root(&d)).unwrap();
        assert!(r.detached && r.branch.is_none());
    }

    #[test]
    fn merging_detected() {
        let d = repo();
        std::fs::write(d.path().join("a.txt"), "1").unwrap();
        commit_all(d.path(), "c1");
        let head = Command::new("git").current_dir(d.path()).args(["rev-parse", "HEAD"]).output().unwrap().stdout;
        let (_s, st) = project(&[d.path()]);
        assert!(!status(&st, "p", &root(&d)).unwrap().merging);
        std::fs::write(d.path().join(".git/MERGE_HEAD"), head).unwrap();
        assert!(status(&st, "p", &root(&d)).unwrap().merging);
    }

    #[test]
    fn ahead_behind_from_real_upstream() {
        let upstream = repo();
        std::fs::write(upstream.path().join("a.txt"), "1").unwrap();
        commit_all(upstream.path(), "c1");
        let work = tempfile::tempdir().unwrap();
        g(work.path(), &["clone", "-q", upstream.path().to_str().unwrap(), "."]);
        for (k, v) in [("user.name", "test"), ("user.email", "test@example.invalid"), ("commit.gpgsign", "false")] {
            g(work.path(), &["config", k, v]);
        }
        std::fs::write(work.path().join("b.txt"), "2").unwrap();
        commit_all(work.path(), "c2");
        let (_s, st) = project(&[work.path()]);
        let r = status(&st, "p", &root(&work)).unwrap();
        assert_eq!((r.ahead, r.behind), (1, 0));
        assert_eq!(r.upstream.as_deref(), Some("origin/main"));
    }

    #[test]
    fn status_truncated_over_5000() {
        let d = repo();
        for i in 0..5001 {
            std::fs::write(d.path().join(format!("f{i}.txt")), "").unwrap();
        }
        let (_s, st) = project(&[d.path()]);
        let r = status(&st, "p", &root(&d)).unwrap();
        assert!(r.truncated);
        assert_eq!(r.entries.len(), 5000);
    }

    #[test]
    fn diff_staged_vs_unstaged() {
        let d = repo();
        std::fs::write(d.path().join("a.txt"), "v1\n").unwrap();
        commit_all(d.path(), "c1");
        std::fs::write(d.path().join("a.txt"), "v2\n").unwrap();
        g(d.path(), &["add", "a.txt"]);
        std::fs::write(d.path().join("a.txt"), "v3\n").unwrap();
        let (_s, st) = project(&[d.path()]);
        let staged = diff(&st, "p", &root(&d), "a.txt", true).unwrap();
        assert_eq!((staged.before.as_str(), staged.after.as_str()), ("v1\n", "v2\n"));
        let unstaged = diff(&st, "p", &root(&d), "a.txt", false).unwrap();
        assert_eq!((unstaged.before.as_str(), unstaged.after.as_str()), ("v2\n", "v3\n"));
        assert!(!staged.binary && !unstaged.binary);
    }

    #[test]
    fn diff_staged_rename_reads_before_from_orig_path() {
        let d = repo();
        std::fs::write(d.path().join("old.txt"), "keep me around\nplease\n").unwrap();
        commit_all(d.path(), "c1");
        g(d.path(), &["mv", "old.txt", "new.txt"]);
        std::fs::write(d.path().join("new.txt"), "keep me around\nplease\nmore\n").unwrap();
        g(d.path(), &["add", "new.txt"]);
        let (_s, st) = project(&[d.path()]);
        let r = diff_with_orig(&st, "p", &root(&d), "new.txt", Some("old.txt"), true).unwrap();
        assert_eq!((r.before.as_str(), r.after.as_str()), ("keep me around\nplease\n", "keep me around\nplease\nmore\n"));
        assert!(diff_with_orig(&st, "p", &root(&d), "new.txt", Some("../x"), true).is_err());
    }

    #[test]
    fn diff_new_file_has_empty_before() {
        let d = repo();
        std::fs::write(d.path().join("seed.txt"), "s").unwrap();
        commit_all(d.path(), "c1");
        std::fs::write(d.path().join("new.txt"), "fresh\n").unwrap();
        let (_s, st) = project(&[d.path()]);
        let untracked = diff(&st, "p", &root(&d), "new.txt", false).unwrap();
        assert_eq!((untracked.before.as_str(), untracked.after.as_str()), ("", "fresh\n"));
        g(d.path(), &["add", "new.txt"]);
        let staged = diff(&st, "p", &root(&d), "new.txt", true).unwrap();
        assert_eq!((staged.before.as_str(), staged.after.as_str()), ("", "fresh\n"));
    }

    #[test]
    fn diff_binary_and_large_files_are_flagged() {
        let d = repo();
        std::fs::write(d.path().join("bin.dat"), [0u8, 1, 2]).unwrap();
        std::fs::write(d.path().join("big.txt"), vec![b'a'; 2 * 1024 * 1024 + 1]).unwrap();
        let (_s, st) = project(&[d.path()]);
        for p in ["bin.dat", "big.txt"] {
            let r = diff(&st, "p", &root(&d), p, false).unwrap();
            assert!(r.binary && r.before.is_empty() && r.after.is_empty(), "{p}");
        }
    }

    #[test]
    fn missing_vs_real_git_errors() {
        let d = repo();
        // HEAD non nato: file nuovo in indice => prima vuoto, non errore
        std::fs::write(d.path().join("n.txt"), "new\n").unwrap();
        g(d.path(), &["add", "n.txt"]);
        let (_s, st) = project(&[d.path()]);
        let r = diff(&st, "p", &root(&d), "n.txt", true).unwrap();
        assert_eq!((r.before.as_str(), r.after.as_str()), ("", "new\n"));
        commit_all(d.path(), "c1");
        // percorso assente in HEAD, nell'indice e su disco: tutto vuoto
        let r = diff(&st, "p", &root(&d), "nulla.txt", true).unwrap();
        assert_eq!((r.before.as_str(), r.after.as_str()), ("", ""));
        let r = diff(&st, "p", &root(&d), "nulla.txt", false).unwrap();
        assert!(r.before.is_empty() && r.after.is_empty());
        // file solo su disco, non in indice
        std::fs::write(d.path().join("solo.txt"), "x").unwrap();
        assert_eq!(diff(&st, "p", &root(&d), "solo.txt", false).unwrap().before, "");
        // repository rotto: errore leggibile, non un diff "tutto aggiunto"
        std::fs::write(d.path().join(".git/HEAD"), "garbage\n").unwrap();
        let e = diff(&st, "p", &root(&d), "n.txt", true).unwrap_err();
        assert!(e.starts_with("git: "), "{e}");
        assert!(diff(&st, "p", &root(&d), "n.txt", false).is_err());
    }

    #[test]
    fn huge_blob_is_binary_without_reading_it() {
        let d = repo();
        // oltre il tetto di 16 MB di run_tool: se fosse letto sarebbe un errore
        std::fs::write(d.path().join("huge.txt"), vec![b'a'; 17 * 1024 * 1024]).unwrap();
        commit_all(d.path(), "big");
        let (_s, st) = project(&[d.path()]);
        let r = diff(&st, "p", &root(&d), "huge.txt", true).unwrap();
        assert!(r.binary && r.before.is_empty() && r.after.is_empty());
        std::fs::write(d.path().join("huge.txt"), vec![b'b'; 17 * 1024 * 1024]).unwrap();
        g(d.path(), &["add", "huge.txt"]);
        assert!(diff(&st, "p", &root(&d), "huge.txt", true).unwrap().binary);
        assert!(diff(&st, "p", &root(&d), "huge.txt", false).unwrap().binary);
    }

    #[test]
    fn fallback_only_for_overflow_and_timeout() {
        assert!(needs_fallback("output oltre 16 MB"));
        assert!(needs_fallback("tempo scaduto dopo 15 s"));
        assert!(!needs_fallback("git: fatal: not a git repository"));
    }

    #[test]
    fn nfd_names_round_trip_to_diff() {
        let d = repo();
        let name = "e\u{301}.txt";
        std::fs::write(d.path().join(name), "uno\n").unwrap();
        commit_all(d.path(), "c1");
        std::fs::write(d.path().join(name), "due\n").unwrap();
        let (_s, st) = project(&[d.path()]);
        let r = status(&st, "p", &root(&d)).unwrap();
        assert_eq!(r.entries.len(), 1);
        let p = r.entries[0].path.clone();
        let df = diff(&st, "p", &root(&d), &p, false).unwrap();
        assert_eq!((df.before.as_str(), df.after.as_str()), ("uno\n", "due\n"));
    }

    #[test]
    fn untrusted_beats_bad_repo_argument() {
        let d = repo();
        let dir = tempfile::tempdir().unwrap();
        let s = Store::open(&dir.path().join("t.db")).unwrap();
        s.upsert_project(&Project { id: "p".into(), name: "p".into(), folders: vec![root(&d)] }).unwrap();
        assert_eq!(status(&s, "p", "/Users/dev/non-esiste").unwrap_err(), crate::trust::UNTRUSTED);
    }

    #[test]
    fn diff_rejects_paths_escaping_the_repo() {
        let d = repo();
        let (_s, st) = project(&[d.path()]);
        for p in ["../x", "/etc/passwd", "", "a/../../b", "./a"] {
            assert!(diff(&st, "p", &root(&d), p, false).is_err(), "{p}");
        }
    }

    #[test]
    fn branches_lists_local_and_remote() {
        let upstream = repo();
        std::fs::write(upstream.path().join("a.txt"), "1").unwrap();
        commit_all(upstream.path(), "c1");
        g(upstream.path(), &["branch", "feature"]);
        let work = tempfile::tempdir().unwrap();
        g(work.path(), &["clone", "-q", upstream.path().to_str().unwrap(), "."]);
        g(work.path(), &["branch", "local-only"]);
        let (_s, st) = project(&[work.path()]);
        let list = branches(&st, "p", &root(&work)).unwrap();
        let find = |n: &str| list.iter().find(|b| b.name == n).unwrap_or_else(|| panic!("manca {n}: {list:?}"));
        assert!(find("main").current && !find("main").remote);
        assert_eq!(find("main").upstream.as_deref(), Some("origin/main"));
        assert!(!find("local-only").current && find("local-only").upstream.is_none());
        assert!(find("origin/feature").remote);
        assert!(list.iter().all(|b| !b.name.ends_with("/HEAD")));
    }

    #[test]
    fn log_respects_limit() {
        let d = repo();
        for i in 0..5 {
            std::fs::write(d.path().join("a.txt"), i.to_string()).unwrap();
            commit_all(d.path(), &format!("commit {i}"));
        }
        let (_s, st) = project(&[d.path()]);
        let l = log(&st, "p", &root(&d), 3).unwrap();
        assert_eq!(l.len(), 3);
        assert_eq!(l[0].subject, "commit 4");
        assert_eq!(l[0].author, "test");
        assert_eq!(l[0].hash.len(), 40);
        assert!(l[0].hash.starts_with(&l[0].short));
        assert_eq!(log(&st, "p", &root(&d), 1000).unwrap().len(), 5);
    }

    #[test]
    fn log_of_empty_repo_is_empty() {
        let d = repo();
        let (_s, st) = project(&[d.path()]);
        assert!(log(&st, "p", &root(&d), 10).unwrap().is_empty());
    }

    #[test]
    fn repos_are_project_folders_with_git_root() {
        let (a, plain, b) = (repo(), tempfile::tempdir().unwrap(), repo());
        let (_s, st) = project(&[a.path(), plain.path(), b.path()]);
        assert_eq!(repos_of(&st, "p").unwrap(), vec![root(&a), root(&b)]);
    }

    /// `<base>/<rel>/.git` come cartella (o file, per i worktree).
    fn mk_repo(base: &Path, rel: &str) {
        std::fs::create_dir_all(base.join(rel).join(".git")).unwrap();
    }

    fn rels(base: &tempfile::TempDir, v: Vec<String>) -> Vec<String> {
        let b = base.path().canonicalize().unwrap();
        v.iter().map(|r| Path::new(r).strip_prefix(&b).unwrap().to_string_lossy().into_owned()).collect()
    }

    #[test]
    fn nested_repos_are_found_down_to_depth_3() {
        let d = tempfile::tempdir().unwrap();
        for r in ["b", "a/x", "c/d/e", "f/g/h/i"] {
            mk_repo(d.path(), r);
        }
        // `.git` file (worktree)
        std::fs::create_dir_all(d.path().join("w")).unwrap();
        std::fs::write(d.path().join("w/.git"), "gitdir: /Users/dev/x\n").unwrap();
        let (_s, st) = project(&[d.path()]);
        assert_eq!(rels(&d, repos_of(&st, "p").unwrap()), vec!["a/x", "b", "c/d/e", "w"]);
    }

    #[test]
    fn nested_scan_skips_hidden_and_build_dirs_and_symlinks() {
        let d = tempfile::tempdir().unwrap();
        for r in [".hidden/r", "node_modules/r", "target/r", "vendor/r", ".venv/r", "venv/r", "dist/r", "build/r", "ok/r"] {
            mk_repo(d.path(), r);
        }
        let out = tempfile::tempdir().unwrap();
        mk_repo(out.path(), "esterno");
        std::os::unix::fs::symlink(out.path().join("esterno"), d.path().join("link")).unwrap();
        let (_s, st) = project(&[d.path()]);
        assert_eq!(rels(&d, repos_of(&st, "p").unwrap()), vec!["ok/r"]);
    }

    #[test]
    fn nested_scan_does_not_descend_into_a_repo() {
        let d = tempfile::tempdir().unwrap();
        mk_repo(d.path(), "outer");
        mk_repo(d.path(), "outer/inner");
        let (_s, st) = project(&[d.path()]);
        assert_eq!(rels(&d, repos_of(&st, "p").unwrap()), vec!["outer"]);
        // la cartella del progetto che è già un repository non cerca sotto di sé
        mk_repo(d.path(), "outer/.git/../sub");
        let root = tempfile::tempdir().unwrap();
        mk_repo(root.path(), ".");
        mk_repo(root.path(), "sub");
        let (_s2, st2) = project(&[root.path()]);
        assert_eq!(repos_of(&st2, "p").unwrap(), vec![root.path().canonicalize().unwrap().to_string_lossy().into_owned()]);
    }

    #[test]
    fn repos_are_capped_and_roots_come_first() {
        let d = tempfile::tempdir().unwrap();
        for i in 0..60 {
            mk_repo(d.path(), &format!("r{i:02}"));
        }
        let top = repo();
        let (_s, st) = project(&[d.path(), top.path()]);
        let v = repos_of(&st, "p").unwrap();
        assert_eq!(v.len(), MAX_REPOS);
        assert_eq!(v[0], root(&top));
        assert!(v[1..].windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn overlapping_project_folders_do_not_duplicate_repos() {
        let d = tempfile::tempdir().unwrap();
        mk_repo(d.path(), "api");
        mk_repo(d.path(), "web");
        let (a, b) = (root_of(d.path()), root_of(&d.path().join("api")));
        let v = repos_in(&[a.clone(), b.clone(), a]);
        assert_eq!(v, vec![b, format!("{}/web", root_of(d.path()))]);
    }

    #[test]
    fn scan_stops_at_the_visit_budget() {
        let d = tempfile::tempdir().unwrap();
        for i in 0..30 {
            mk_repo(d.path(), &format!("r{i:02}"));
        }
        let mut all = Vec::new();
        scan_nested(d.path(), 1, &mut 1000, &mut all);
        assert_eq!(all.len(), 30);
        let mut few = Vec::new();
        scan_nested(d.path(), 1, &mut 10, &mut few);
        assert!(few.len() <= 10 && !few.is_empty());
        // senza budget non si legge nulla
        let mut none = Vec::new();
        scan_nested(d.path(), 1, &mut 0, &mut none);
        assert!(none.is_empty());
    }

    #[test]
    fn nested_repo_is_accepted_by_resolve_repo() {
        let d = tempfile::tempdir().unwrap();
        let sub = d.path().join("api");
        std::fs::create_dir(&sub).unwrap();
        g(&sub, &["init", "-q", "-b", "main"]);
        configure(&sub);
        std::fs::create_dir_all(d.path().join("api/deep")).unwrap();
        let (_s, st) = project(&[d.path()]);
        let nested = sub.canonicalize().unwrap().to_string_lossy().into_owned();
        assert_eq!(resolve_repo(&st, "p", &nested).unwrap(), PathBuf::from(&nested));
        assert_eq!(status(&st, "p", &nested).unwrap().branch.as_deref(), Some("main"));
        // sottocartella del repository, radice del progetto non repository, percorso esterno: rifiutati
        assert!(resolve_repo(&st, "p", &format!("{nested}/deep")).is_err());
        assert!(resolve_repo(&st, "p", &root_of(d.path())).is_err());
        let other = repo();
        assert!(resolve_repo(&st, "p", &root(&other)).is_err());
    }

    fn root_of(p: &Path) -> String {
        p.canonicalize().unwrap().to_string_lossy().into_owned()
    }

    #[test]
    fn nested_repos_refused_when_untrusted() {
        let d = tempfile::tempdir().unwrap();
        mk_repo(d.path(), "a");
        let dir = tempfile::tempdir().unwrap();
        let s = Store::open(&dir.path().join("t.db")).unwrap();
        let f = root_of(d.path());
        s.upsert_project(&Project { id: "p".into(), name: "p".into(), folders: vec![f.clone()] }).unwrap();
        assert_eq!(repos_of(&s, "p").unwrap_err(), crate::trust::UNTRUSTED);
        assert_eq!(resolve_repo(&s, "p", &format!("{f}/a")).unwrap_err(), crate::trust::UNTRUSTED);
    }

    #[test]
    fn repo_arg_must_be_project_repo() {
        let (mine, other) = (repo(), repo());
        let (_s, st) = project(&[mine.path()]);
        assert!(status(&st, "p", &root(&mine)).is_ok());
        assert!(status(&st, "p", &root(&other)).is_err());
        // sottocartella del repository, o percorso inesistente
        std::fs::create_dir(mine.path().join("sub")).unwrap();
        assert!(status(&st, "p", &format!("{}/sub", root(&mine))).is_err());
        assert!(status(&st, "p", "/Users/dev/non-esiste").is_err());
    }

    #[test]
    fn refused_when_untrusted() {
        let d = repo();
        let dir = tempfile::tempdir().unwrap();
        let s = Store::open(&dir.path().join("t.db")).unwrap();
        let f = root(&d);
        s.upsert_project(&Project { id: "p".into(), name: "p".into(), folders: vec![f.clone()] }).unwrap();
        assert_eq!(repos_of(&s, "p").unwrap_err(), crate::trust::UNTRUSTED);
        assert_eq!(status(&s, "p", &f).unwrap_err(), crate::trust::UNTRUSTED);
        assert_eq!(diff(&s, "p", &f, "a", false).unwrap_err(), crate::trust::UNTRUSTED);
        assert_eq!(branches(&s, "p", &f).unwrap_err(), crate::trust::UNTRUSTED);
        assert_eq!(log(&s, "p", &f, 5).unwrap_err(), crate::trust::UNTRUSTED);
    }
}
