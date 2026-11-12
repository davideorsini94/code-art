use super::*;
use crate::git::testutil::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

/// Conferma finta: risponde sempre `answer` e conta le domande.
struct FakeConfirm {
    answer: bool,
    asked: AtomicUsize,
    last: Mutex<String>,
}

fn confirm(answer: bool) -> FakeConfirm {
    FakeConfirm { answer, asked: AtomicUsize::new(0), last: Mutex::new(String::new()) }
}

impl Confirm for FakeConfirm {
    fn confirm(&self, _title: &str, message: &str, _action: &str) -> bool {
        self.asked.fetch_add(1, Ordering::SeqCst);
        *self.last.lock().unwrap() = message.to_string();
        self.answer
    }
}

/// Cestino finto: annota i percorsi e li sposta in una cartella temporanea (mai il vero Cestino).
struct FakeTrash {
    bin: tempfile::TempDir,
    got: Mutex<Vec<PathBuf>>,
}

impl Trasher for FakeTrash {
    fn trash(&self, path: &Path) -> Result<()> {
        let n = self.got.lock().unwrap().len();
        std::fs::rename(path, self.bin.path().join(n.to_string())).map_err(|e| e.to_string())?;
        self.got.lock().unwrap().push(path.to_path_buf());
        Ok(())
    }
}

fn s(v: &[&str]) -> Vec<String> {
    v.iter().map(|x| x.to_string()).collect()
}

/// Repository con un commit iniziale (`a.txt`) e il progetto fidato che lo contiene.
fn seeded() -> (tempfile::TempDir, tempfile::TempDir, Store, String) {
    let d = repo();
    std::fs::write(d.path().join("a.txt"), "uno\n").unwrap();
    commit_all(d.path(), "init");
    let (sd, st) = project(&[d.path()]);
    let r = root(&d);
    (d, sd, st, r)
}

fn staged_names(d: &Path) -> String {
    g_str(d, &["diff", "--cached", "--name-only"])
}

#[test]
fn stage_unstage_roundtrip() {
    let (d, _sd, st, r) = seeded();
    std::fs::write(d.path().join("a.txt"), "due\n").unwrap();
    std::fs::write(d.path().join("nuovo file.txt"), "x\n").unwrap();
    std::fs::write(d.path().join("altro.txt"), "y\n").unwrap();
    stage(&st, "p", &r, &s(&["a.txt", "nuovo file.txt"])).unwrap();
    assert_eq!(staged_names(d.path()), "a.txt\nnuovo file.txt");
    unstage(&st, "p", &r, &s(&["a.txt"])).unwrap();
    assert_eq!(staged_names(d.path()), "nuovo file.txt");
    // un file cancellato si mette in stage come cancellazione
    std::fs::remove_file(d.path().join("a.txt")).unwrap();
    stage(&st, "p", &r, &s(&["a.txt"])).unwrap();
    assert_eq!(g_str(d.path(), &["diff", "--cached", "--name-status", "--", "a.txt"]), "D\ta.txt");
}

#[test]
fn unstage_in_repo_without_commits() {
    let d = repo();
    std::fs::write(d.path().join("n.txt"), "x").unwrap();
    let (_sd, st) = project(&[d.path()]);
    stage(&st, "p", &root(&d), &s(&["n.txt"])).unwrap();
    assert_eq!(staged_names(d.path()), "n.txt");
    unstage(&st, "p", &root(&d), &s(&["n.txt"])).unwrap();
    assert_eq!(staged_names(d.path()), "");
}

#[test]
fn pathspec_magic_is_literal() {
    let (d, _sd, st, r) = seeded();
    std::fs::write(d.path().join("b.txt"), "b").unwrap();
    // `:/` e `*` sarebbero "tutto il repository" come pathspec: qui sono nomi di file inesistenti
    for p in [":/", "*", "*.txt"] {
        assert!(stage(&st, "p", &r, &s(&[p])).is_err(), "{p}");
    }
    assert_eq!(staged_names(d.path()), "");
}

#[test]
fn commit_creates_commit_with_message_from_stdin() {
    let (d, _sd, st, r) = seeded();
    std::fs::write(d.path().join("a.txt"), "due\n").unwrap();
    stage(&st, "p", &r, &s(&["a.txt"])).unwrap();
    let msg = "Titolo con «accenti» -m --amend\n\nCorpo\nsu più righe";
    let short = commit(&st, "p", &r, msg, false).unwrap();
    assert_eq!(short, g_str(d.path(), &["rev-parse", "--short", "HEAD"]));
    assert_eq!(g_str(d.path(), &["log", "-1", "--format=%B"]), msg);
    assert_eq!(g_str(d.path(), &["rev-list", "--count", "HEAD"]), "2");
}

#[test]
fn commit_empty_message_refused() {
    let (d, _sd, st, r) = seeded();
    std::fs::write(d.path().join("a.txt"), "due\n").unwrap();
    stage(&st, "p", &r, &s(&["a.txt"])).unwrap();
    for m in ["", "   ", "\n\t\n"] {
        assert!(commit(&st, "p", &r, m, false).is_err(), "{m:?}");
    }
    assert_eq!(g_str(d.path(), &["rev-list", "--count", "HEAD"]), "1");
}

#[test]
fn amend_changes_last_commit() {
    let (d, _sd, st, r) = seeded();
    std::fs::write(d.path().join("b.txt"), "b\n").unwrap();
    stage(&st, "p", &r, &s(&["b.txt"])).unwrap();
    commit(&st, "p", &r, "secondo", false).unwrap();
    std::fs::write(d.path().join("c.txt"), "c\n").unwrap();
    stage(&st, "p", &r, &s(&["c.txt"])).unwrap();
    commit(&st, "p", &r, "secondo corretto", true).unwrap();
    assert_eq!(g_str(d.path(), &["rev-list", "--count", "HEAD"]), "2");
    assert_eq!(g_str(d.path(), &["log", "-1", "--format=%s"]), "secondo corretto");
    assert_eq!(g_str(d.path(), &["show", "--name-only", "--format=", "HEAD"]), "b.txt\nc.txt");
}

#[test]
fn commit_runs_repository_hooks() {
    let (d, _sd, st, r) = seeded();
    let hooks = d.path().join("hooks");
    std::fs::create_dir(&hooks).unwrap();
    let hook = hooks.join("pre-commit");
    std::fs::write(&hook, "#!/bin/sh\necho 'bloccato dal hook' >&2\nexit 1\n").unwrap();
    std::fs::set_permissions(&hook, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    g(d.path(), &["config", "core.hooksPath", hooks.to_str().unwrap()]);
    std::fs::write(d.path().join("a.txt"), "due\n").unwrap();
    stage(&st, "p", &r, &s(&["a.txt"])).unwrap();
    let e = commit(&st, "p", &r, "msg", false).unwrap_err();
    assert!(e.contains("bloccato dal hook"), "{e}");
}

/// Gli hook vedono pathspec normali: un hook che filtra con un glob trova i file (con `GIT_LITERAL_PATHSPECS=1`
/// ereditato non troverebbe nulla e passerebbe a vuoto).
#[test]
fn commit_hooks_see_glob_pathspecs() {
    let (d, _sd, st, r) = seeded();
    let hooks = d.path().join("hooks");
    std::fs::create_dir(&hooks).unwrap();
    let hook = hooks.join("pre-commit");
    let seen = d.path().join("visti.txt");
    std::fs::write(
        &hook,
        format!("#!/bin/sh\nfiles=$(git diff --cached --name-only -- '*.txt')\n[ -n \"$files\" ] || {{ echo 'nessun file *.txt' >&2; exit 1; }}\necho \"$files\" > '{}'\n", seen.display()),
    )
    .unwrap();
    std::fs::set_permissions(&hook, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    g(d.path(), &["config", "core.hooksPath", hooks.to_str().unwrap()]);
    std::fs::write(d.path().join("a.txt"), "due\n").unwrap();
    stage(&st, "p", &r, &s(&["a.txt"])).unwrap();
    commit(&st, "p", &r, "con hook", false).unwrap();
    assert_eq!(std::fs::read_to_string(&seen).unwrap().trim(), "a.txt");
}

#[test]
fn branch_name_starting_with_dash_refused() {
    let (d, _sd, st, r) = seeded();
    let c = confirm(true);
    for bad in ["-D", "--force", "-b", "", "a..b", "@{-1}", "HEAD", "con spazio", "x\0y"] {
        assert!(create_branch(&st, "p", &r, bad, None, false).is_err(), "{bad:?}");
        assert!(checkout(&st, "p", &r, bad).is_err(), "{bad:?}");
        assert!(merge(&st, "p", &r, bad).is_err(), "{bad:?}");
        assert!(delete_branch(&st, "p", &r, bad, &c).is_err(), "{bad:?}");
    }
    assert!(create_branch(&st, "p", &r, "ok", Some("--orphan"), false).is_err());
    assert_eq!(c.asked.load(Ordering::SeqCst), 0);
    assert_eq!(g_str(d.path(), &["branch", "--format=%(refname:short)"]), "main");
}

#[test]
fn create_checkout_delete_branch() {
    let (d, _sd, st, r) = seeded();
    create_branch(&st, "p", &r, "feature/uno", None, true).unwrap();
    assert_eq!(g_str(d.path(), &["symbolic-ref", "--short", "HEAD"]), "feature/uno");
    create_branch(&st, "p", &r, "altro", Some("main"), false).unwrap();
    assert_eq!(g_str(d.path(), &["symbolic-ref", "--short", "HEAD"]), "feature/uno");
    checkout(&st, "p", &r, "main").unwrap();
    assert_eq!(g_str(d.path(), &["symbolic-ref", "--short", "HEAD"]), "main");
    // annullato: il branch resta
    assert!(!delete_branch(&st, "p", &r, "altro", &confirm(false)).unwrap());
    let c = confirm(true);
    assert!(delete_branch(&st, "p", &r, "altro", &c).unwrap());
    assert!(c.last.lock().unwrap().contains("altro"));
    assert!(!g_str(d.path(), &["branch"]).contains("altro"));
    // non unito: `-d` rifiuta con un messaggio leggibile
    checkout(&st, "p", &r, "feature/uno").unwrap();
    std::fs::write(d.path().join("f.txt"), "f").unwrap();
    commit_all(d.path(), "lavoro");
    checkout(&st, "p", &r, "main").unwrap();
    let e = delete_branch(&st, "p", &r, "feature/uno", &confirm(true)).unwrap_err();
    assert!(e.contains("non è unito"), "{e}");
    assert!(g_str(d.path(), &["branch"]).contains("feature/uno"));
    // branch inesistente
    assert!(checkout(&st, "p", &r, "manca").is_err());
}

#[test]
fn checkout_remote_branch_creates_tracking_branch() {
    let up = repo();
    std::fs::write(up.path().join("a.txt"), "1").unwrap();
    commit_all(up.path(), "c1");
    g(up.path(), &["branch", "feature"]);
    let work = tempfile::tempdir().unwrap();
    g(work.path(), &["clone", "-q", up.path().to_str().unwrap(), "."]);
    configure(work.path());
    let (_sd, st) = project(&[work.path()]);
    checkout(&st, "p", &root(&work), "origin/feature").unwrap();
    assert_eq!(g_str(work.path(), &["symbolic-ref", "--short", "HEAD"]), "feature");
    assert_eq!(g_str(work.path(), &["rev-parse", "--abbrev-ref", "feature@{u}"]), "origin/feature");
}

#[test]
fn delete_current_branch_refused() {
    let (d, _sd, st, r) = seeded();
    let c = confirm(true);
    let e = delete_branch(&st, "p", &r, "main", &c).unwrap_err();
    assert!(e.contains("corrente"), "{e}");
    assert_eq!(c.asked.load(Ordering::SeqCst), 0);
    assert!(g_str(d.path(), &["branch"]).contains("main"));
}

#[test]
fn merge_fast_forward() {
    let (d, _sd, st, r) = seeded();
    assert_eq!(merge(&st, "p", &r, "main").unwrap().kind, MergeKind::UpToDate);
    create_branch(&st, "p", &r, "f", None, true).unwrap();
    std::fs::write(d.path().join("b.txt"), "b").unwrap();
    commit_all(d.path(), "su f");
    checkout(&st, "p", &r, "main").unwrap();
    let m = merge(&st, "p", &r, "f").unwrap();
    assert_eq!(m, MergeOutcome { kind: MergeKind::FastForward, conflicts: vec![] });
    assert_eq!(g_str(d.path(), &["rev-parse", "main"]), g_str(d.path(), &["rev-parse", "f"]));
    // storie divergenti senza conflitti: commit di merge
    create_branch(&st, "p", &r, "g", None, true).unwrap();
    std::fs::write(d.path().join("g.txt"), "g").unwrap();
    commit_all(d.path(), "su g");
    checkout(&st, "p", &r, "main").unwrap();
    std::fs::write(d.path().join("m.txt"), "m").unwrap();
    commit_all(d.path(), "su main");
    assert_eq!(merge(&st, "p", &r, "g").unwrap().kind, MergeKind::Merged);
    assert_eq!(g_str(d.path(), &["rev-list", "--count", "--merges", "HEAD"]), "1");
}

#[test]
fn merge_conflicts_reported_and_abort() {
    let (d, _sd, st, r) = seeded();
    create_branch(&st, "p", &r, "f", None, true).unwrap();
    std::fs::write(d.path().join("a.txt"), "da f\n").unwrap();
    commit_all(d.path(), "f");
    checkout(&st, "p", &r, "main").unwrap();
    std::fs::write(d.path().join("a.txt"), "da main\n").unwrap();
    commit_all(d.path(), "main");
    let head = g_str(d.path(), &["rev-parse", "HEAD"]);
    let m = merge(&st, "p", &r, "f").unwrap();
    assert_eq!(m, MergeOutcome { kind: MergeKind::Conflicts, conflicts: vec!["a.txt".into()] });
    assert!(crate::git::status(&st, "p", &r).unwrap().merging);
    merge_abort(&st, "p", &r).unwrap();
    assert!(!crate::git::status(&st, "p", &r).unwrap().merging);
    assert_eq!(g_str(d.path(), &["rev-parse", "HEAD"]), head);
    assert_eq!(std::fs::read_to_string(d.path().join("a.txt")).unwrap(), "da main\n");
}

#[test]
fn discard_moves_untracked_to_trash() {
    let (d, _sd, st, r) = seeded();
    std::fs::write(d.path().join("a.txt"), "modificato\n").unwrap();
    std::fs::write(d.path().join("nuovo.txt"), "da buttare\n").unwrap();
    let t = FakeTrash { bin: tempfile::tempdir().unwrap(), got: Mutex::new(vec![]) };
    // annullato: niente cambia
    let no = confirm(false);
    assert!(!discard(&st, "p", &r, &s(&["a.txt", "nuovo.txt"]), &no, &t).unwrap());
    assert_eq!(*no.last.lock().unwrap(), "Scartare le modifiche a 2 file? Non si possono recuperare.");
    assert!(d.path().join("nuovo.txt").exists());
    assert_eq!(std::fs::read_to_string(d.path().join("a.txt")).unwrap(), "modificato\n");
    let yes = confirm(true);
    assert!(discard(&st, "p", &r, &s(&["a.txt", "nuovo.txt"]), &yes, &t).unwrap());
    assert_eq!(std::fs::read_to_string(d.path().join("a.txt")).unwrap(), "uno\n");
    assert!(!d.path().join("nuovo.txt").exists());
    let got = t.got.lock().unwrap().clone();
    assert_eq!(got, vec![d.path().canonicalize().unwrap().join("nuovo.txt")]);
    assert_eq!(std::fs::read_to_string(t.bin.path().join("0")).unwrap(), "da buttare\n");
}

#[test]
fn paths_outside_project_refused() {
    let (d, _sd, st, r) = seeded();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("x.txt"), "x").unwrap();
    std::os::unix::fs::symlink(outside.path(), d.path().join("link")).unwrap();
    let t = FakeTrash { bin: tempfile::tempdir().unwrap(), got: Mutex::new(vec![]) };
    let c = confirm(true);
    let abs = outside.path().join("x.txt").to_string_lossy().into_owned();
    for p in ["../x.txt", abs.as_str(), "", "a/../../x", "./a.txt", "link/x.txt", "-a.txt", "--all"] {
        assert!(stage(&st, "p", &r, &s(&[p])).is_err(), "{p}");
        assert!(unstage(&st, "p", &r, &s(&[p])).is_err(), "{p}");
        assert!(discard(&st, "p", &r, &s(&[p]), &c, &t).is_err(), "{p}");
    }
    assert!(stage(&st, "p", &r, &[]).is_err());
    assert_eq!(c.asked.load(Ordering::SeqCst), 0);
    assert!(t.got.lock().unwrap().is_empty());
    assert!(outside.path().join("x.txt").exists());
    // un repository che non è del progetto
    let other = repo();
    std::fs::write(other.path().join("a.txt"), "x").unwrap();
    assert!(stage(&st, "p", &root(&other), &s(&["a.txt"])).is_err());
    assert_eq!(staged_names(other.path()), "");
}

#[test]
fn concurrent_write_refused() {
    let (d, _sd, st, r) = seeded();
    // un hook pre-commit lento tiene occupato il repository
    let hooks = d.path().join("hooks");
    std::fs::create_dir(&hooks).unwrap();
    let marker = d.path().join("hook-partito");
    let hook = hooks.join("pre-commit");
    std::fs::write(&hook, format!("#!/bin/sh\ntouch '{}'\nsleep 3\n", marker.display())).unwrap();
    std::fs::set_permissions(&hook, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    g(d.path(), &["config", "core.hooksPath", hooks.to_str().unwrap()]);
    std::fs::write(d.path().join("a.txt"), "due\n").unwrap();
    g(d.path(), &["add", "a.txt"]);
    let st = std::sync::Arc::new(st);
    let (st2, r2) = (st.clone(), r.clone());
    let h = std::thread::spawn(move || commit(&st2, "p", &r2, "lento", false));
    let t0 = std::time::Instant::now();
    while !marker.exists() {
        assert!(t0.elapsed() < std::time::Duration::from_secs(10), "hook mai partito");
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    std::fs::write(d.path().join("b.txt"), "b").unwrap();
    assert_eq!(stage(&st, "p", &r, &s(&["b.txt"])).unwrap_err(), BUSY);
    assert_eq!(checkout(&st, "p", &r, "main").unwrap_err(), BUSY);
    // le letture restano libere
    assert!(crate::git::status(&st, "p", &r).is_ok());
    h.join().unwrap().unwrap();
    stage(&st, "p", &r, &s(&["b.txt"])).unwrap();
}

#[test]
fn init_only_in_project_folder_not_yet_repo() {
    let plain = tempfile::tempdir().unwrap();
    let existing = repo();
    let (_sd, st) = project(&[plain.path(), existing.path()]);
    let elsewhere = tempfile::tempdir().unwrap();
    assert!(init(&st, "p", &root(&elsewhere)).is_err());
    assert!(!elsewhere.path().join(".git").exists());
    assert!(init(&st, "p", &root(&existing)).is_err());
    init(&st, "p", &root(&plain)).unwrap();
    assert!(plain.path().join(".git").is_dir());
    assert_eq!(crate::git::repos_of(&st, "p").unwrap().len(), 2);
}

#[test]
fn refused_when_untrusted() {
    let d = repo();
    let dir = tempfile::tempdir().unwrap();
    let st = Store::open(&dir.path().join("t.db")).unwrap();
    let r = root(&d);
    st.upsert_project(&crate::store::Project { id: "p".into(), name: "p".into(), folders: vec![r.clone()] }).unwrap();
    let u = crate::trust::UNTRUSTED;
    assert_eq!(stage(&st, "p", &r, &s(&["a"])).unwrap_err(), u);
    assert_eq!(commit(&st, "p", &r, "m", false).unwrap_err(), u);
    assert_eq!(checkout(&st, "p", &r, "main").unwrap_err(), u);
    assert_eq!(merge(&st, "p", &r, "main").unwrap_err(), u);
    assert_eq!(init(&st, "p", &r).unwrap_err(), u);
}

/// Cestino che rifiuta tutto.
struct BrokenTrash;

impl Trasher for BrokenTrash {
    fn trash(&self, _: &Path) -> Result<()> {
        Err("disco pieno".into())
    }
}

#[test]
fn discard_trash_failure_says_what_was_already_discarded() {
    let (d, _sd, st, r) = seeded();
    std::fs::write(d.path().join("a.txt"), "modificato\n").unwrap();
    std::fs::write(d.path().join("nuovo.txt"), "x").unwrap();
    let e = discard(&st, "p", &r, &s(&["a.txt", "nuovo.txt"]), &confirm(true), &BrokenTrash).unwrap_err();
    assert!(e.starts_with("1 file scartati") && e.contains("nuovo.txt") && e.contains("disco pieno"), "{e}");
    assert!(d.path().join("nuovo.txt").exists());
}
