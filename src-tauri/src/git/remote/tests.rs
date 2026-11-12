use super::*;
use crate::git::testutil::*;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// Origine `--bare` locale con un commit su `main` e un clone di lavoro fidato nel progetto "p".
struct Setup {
    origin: tempfile::TempDir,
    work: tempfile::TempDir,
    _sd: tempfile::TempDir,
    st: Store,
}

fn setup() -> Setup {
    let seed = repo();
    std::fs::write(seed.path().join("a.txt"), "uno\n").unwrap();
    commit_all(seed.path(), "init");
    let origin = tempfile::tempdir().unwrap();
    g(origin.path(), &["clone", "-q", "--bare", seed.path().to_str().unwrap(), "."]);
    let work = tempfile::tempdir().unwrap();
    g(work.path(), &["clone", "-q", origin.path().to_str().unwrap(), "."]);
    configure(work.path());
    let (sd, st) = project(&[work.path()]);
    Setup { origin, work, _sd: sd, st }
}

type Rx = mpsc::Receiver<RemoteEvent>;

fn begin(s: &Setup, ops: &Arc<RemoteOps>, op: RemoteOp) -> (Result<u64>, Rx) {
    let (tx, rx) = mpsc::channel();
    let tx = Mutex::new(tx);
    let r = start(&s.st, "p", &root(&s.work), op, ops.clone(), move |e| {
        let _ = tx.lock().unwrap().send(e);
    });
    (r, rx)
}

/// Aspetta la fine: (ok, messaggio, righe di avanzamento).
fn wait_done(rx: &Rx, within: Duration) -> (bool, String, Vec<String>) {
    let deadline = Instant::now() + within;
    let mut lines = Vec::new();
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match rx.recv_timeout(left).expect("operazione non finita in tempo") {
            RemoteEvent::Progress { line, .. } => lines.push(line),
            RemoteEvent::Done { ok, message, .. } => return (ok, message, lines),
        }
    }
}

#[test]
fn push_sets_upstream_on_bare_origin() {
    let s = setup();
    let w = s.work.path();
    g(w, &["switch", "-q", "-c", "nuovo"]);
    std::fs::write(w.join("b.txt"), "b\n").unwrap();
    commit_all(w, "su nuovo");
    assert!(!g_out(w, &["rev-parse", "--abbrev-ref", "nuovo@{u}"]).status.success());
    let ops = Arc::new(RemoteOps::default());
    let (id, rx) = begin(&s, &ops, RemoteOp::Push);
    let id = id.unwrap();
    let (ok, msg, _) = wait_done(&rx, Duration::from_secs(60));
    assert!(ok, "{msg}");
    assert_eq!(g_str(w, &["rev-parse", "--abbrev-ref", "nuovo@{u}"]), "origin/nuovo");
    assert_eq!(g_str(s.origin.path(), &["rev-parse", "nuovo"]), g_str(w, &["rev-parse", "HEAD"]));
    // finita: non è più annullabile
    assert!(!ops.cancel("p", id));
    // con l'upstream già impostato un secondo push va liscio
    std::fs::write(w.join("c.txt"), "c\n").unwrap();
    commit_all(w, "ancora");
    let (_, rx) = begin(&s, &ops, RemoteOp::Push);
    assert!(wait_done(&rx, Duration::from_secs(60)).0);
    assert_eq!(g_str(s.origin.path(), &["rev-parse", "nuovo"]), g_str(w, &["rev-parse", "HEAD"]));
}

#[test]
fn push_rejected_is_not_forced() {
    let s = setup();
    let w = s.work.path();
    // qualcun altro ha spinto su main
    let other = tempfile::tempdir().unwrap();
    g(other.path(), &["clone", "-q", s.origin.path().to_str().unwrap(), "."]);
    configure(other.path());
    std::fs::write(other.path().join("x.txt"), "x").unwrap();
    commit_all(other.path(), "altro");
    g(other.path(), &["push", "-q"]);
    let theirs = g_str(s.origin.path(), &["rev-parse", "main"]);
    std::fs::write(w.join("y.txt"), "y").unwrap();
    commit_all(w, "mio");
    let ops = Arc::new(RemoteOps::default());
    let (_, rx) = begin(&s, &ops, RemoteOp::Push);
    let (ok, msg, _) = wait_done(&rx, Duration::from_secs(60));
    assert!(!ok && msg.starts_with("git: "), "{msg}");
    assert_eq!(g_str(s.origin.path(), &["rev-parse", "main"]), theirs);
}

#[test]
fn pull_is_ff_only_by_default() {
    let s = setup();
    let w = s.work.path();
    let other = tempfile::tempdir().unwrap();
    g(other.path(), &["clone", "-q", s.origin.path().to_str().unwrap(), "."]);
    configure(other.path());
    std::fs::write(other.path().join("x.txt"), "x").unwrap();
    commit_all(other.path(), "altro");
    g(other.path(), &["push", "-q"]);
    let ops = Arc::new(RemoteOps::default());
    let (_, rx) = begin(&s, &ops, RemoteOp::Pull);
    let (ok, msg, _) = wait_done(&rx, Duration::from_secs(60));
    assert!(ok, "{msg}");
    assert_eq!(g_str(w, &["rev-parse", "HEAD"]), g_str(s.origin.path(), &["rev-parse", "main"]));
    // storie divergenti: `--ff-only` rifiuta, niente commit di merge
    std::fs::write(other.path().join("z.txt"), "z").unwrap();
    commit_all(other.path(), "altro ancora");
    g(other.path(), &["push", "-q"]);
    std::fs::write(w.join("mio.txt"), "m").unwrap();
    commit_all(w, "mio");
    let head = g_str(w, &["rev-parse", "HEAD"]);
    let (_, rx) = begin(&s, &ops, RemoteOp::Pull);
    let (ok, _, _) = wait_done(&rx, Duration::from_secs(60));
    assert!(!ok);
    assert_eq!(g_str(w, &["rev-parse", "HEAD"]), head);
    // con `git_pull_mode = rebase` si ribasa
    s.st.setting_set(PULL_MODE_KEY, "rebase").unwrap();
    let (_, rx) = begin(&s, &ops, RemoteOp::Pull);
    let (ok, msg, _) = wait_done(&rx, Duration::from_secs(60));
    assert!(ok, "{msg}");
    assert_eq!(g_str(w, &["rev-parse", "HEAD~1"]), g_str(s.origin.path(), &["rev-parse", "main"]));
    assert_eq!(g_str(w, &["rev-list", "--count", "--merges", "HEAD"]), "0");
}

#[test]
fn pull_mode_values() {
    for ok in ["ff-only", "rebase", "merge"] {
        assert!(valid_pull_mode(ok));
    }
    for bad in ["", "force", "--rebase", "FF-ONLY"] {
        assert!(!valid_pull_mode(bad));
    }
    assert_eq!(pull_args(None), ["--ff-only"]);
    assert_eq!(pull_args(Some("merge")), ["--no-rebase", "--no-edit"]);
    assert_eq!(pull_args(Some("rebase")), ["--rebase"]);
    assert_eq!(pull_args(Some("strano")), ["--ff-only"]);
}

#[test]
fn fetch_unreachable_remote_fails_fast_without_prompt() {
    let s = setup();
    g(s.work.path(), &["remote", "set-url", "origin", "https://127.0.0.1:9/x.git"]);
    let ops = Arc::new(RemoteOps::default());
    let t0 = Instant::now();
    let (id, rx) = begin(&s, &ops, RemoteOp::Fetch);
    id.unwrap();
    let (ok, msg, _) = wait_done(&rx, Duration::from_secs(20));
    assert!(!ok, "{msg}");
    assert!(msg.starts_with("git: ") && msg.len() > 5, "{msg}");
    assert!(t0.elapsed() < Duration::from_secs(20));
}

/// `remote.origin.uploadpack` al posto di `git-upload-pack`: un fetch che resta appeso finché non è ucciso.
fn hang_fetch(s: &Setup) -> PathBuf {
    let dir = s._sd.path().join("bin");
    std::fs::create_dir_all(&dir).unwrap();
    let marker = dir.join("partito");
    let script = dir.join("appeso.sh");
    std::fs::write(&script, format!("#!/bin/sh\ntouch '{}'\nsleep 60\n", marker.display())).unwrap();
    std::fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    g(s.work.path(), &["config", "remote.origin.uploadpack", script.to_str().unwrap()]);
    marker
}

fn wait_for(p: &Path) {
    let t0 = Instant::now();
    while !p.exists() {
        assert!(t0.elapsed() < Duration::from_secs(10), "processo mai partito");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn cancel_kills_long_op() {
    let s = setup();
    let marker = hang_fetch(&s);
    let ops = Arc::new(RemoteOps::default());
    let (id, rx) = begin(&s, &ops, RemoteOp::Fetch);
    let id = id.unwrap();
    wait_for(&marker);
    // un'altra finestra non può annullarla
    assert!(!ops.cancel("altro", id));
    let t0 = Instant::now();
    assert!(ops.cancel("p", id));
    let (ok, msg, _) = wait_done(&rx, Duration::from_secs(10));
    assert!(!ok);
    assert_eq!(msg, CANCELLED);
    assert!(t0.elapsed() < Duration::from_secs(5));
    // il repository è di nuovo libero
    let (id2, rx2) = begin(&s, &ops, RemoteOp::Fetch);
    id2.unwrap();
    ops.cancel_project("p");
    assert_eq!(wait_done(&rx2, Duration::from_secs(10)).1, CANCELLED);
}

#[test]
fn remote_op_holds_the_repo_lock() {
    let s = setup();
    let marker = hang_fetch(&s);
    let ops = Arc::new(RemoteOps::default());
    let (id, rx) = begin(&s, &ops, RemoteOp::Fetch);
    let id = id.unwrap();
    wait_for(&marker);
    std::fs::write(s.work.path().join("b.txt"), "b").unwrap();
    let r = root(&s.work);
    assert_eq!(crate::git::write::stage(&s.st, "p", &r, &["b.txt".into()]).unwrap_err(), crate::git::write::BUSY);
    assert_eq!(begin(&s, &ops, RemoteOp::Push).0.unwrap_err(), crate::git::write::BUSY);
    assert!(crate::git::status(&s.st, "p", &r).is_ok());
    ops.cancel("p", id);
    wait_done(&rx, Duration::from_secs(10));
    crate::git::write::stage(&s.st, "p", &r, &["b.txt".into()]).unwrap();
}

#[test]
fn progress_lines_are_split_on_carriage_returns() {
    let mut got = Vec::new();
    let mut split = LineSplitter::default();
    split.feed(b"Counting objects:  50% (1/2)\rCounting objects: 100% (2/2), done.\nTo /x\n par", &mut |l| got.push(l.to_string()));
    split.feed(b"ziale", &mut |l| got.push(l.to_string()));
    split.finish(&mut |l| got.push(l.to_string()));
    assert_eq!(got, ["Counting objects:  50% (1/2)", "Counting objects: 100% (2/2), done.", "To /x", " parziale"]);
}

#[test]
fn ssh_is_batch_mode_unless_user_configured() {
    let s = setup();
    let env: HashMap<String, String> = HashMap::new();
    assert_eq!(ssh_override(&env, s.work.path()), Some(SSH_BATCH));
    let user: HashMap<String, String> = [("GIT_SSH_COMMAND".to_string(), "ssh -i k".to_string())].into();
    assert_eq!(ssh_override(&user, s.work.path()), None);
    g(s.work.path(), &["config", "core.sshCommand", "ssh -F cfg"]);
    assert_eq!(ssh_override(&env, s.work.path()), None);
}
