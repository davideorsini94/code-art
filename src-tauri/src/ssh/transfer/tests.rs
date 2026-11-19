use super::*;
use std::sync::atomic::AtomicBool;

/// Il server finto: `/bin/sh -c <script> _ <argomenti>`, gli stessi script e lo stesso percorso di streaming del
/// trasporto vero. Ogni processo scrive il proprio pid in un file, per provare che nessuno resta vivo.
struct LocalSh {
    pidfile: PathBuf,
}

impl LocalSh {
    fn new(dir: &Path) -> Self {
        LocalSh { pidfile: dir.join("pids.txt") }
    }
    fn pids(&self) -> Vec<i32> {
        std::fs::read_to_string(&self.pidfile).unwrap_or_default().lines().filter_map(|l| l.trim().parse().ok()).collect()
    }
}

impl Transport for LocalSh {
    fn command(&self, script: &str, args: &[&str]) -> Result<Command, String> {
        let mut c = Command::new("/bin/sh");
        c.arg("-c").arg(format!("echo $$ >> '{}'\n{script}", self.pidfile.display())).arg("_").args(args);
        Ok(c)
    }
    fn run(&self, script: &str, args: &[&str], stdin: Option<Vec<u8>>) -> Result<ToolOutput, String> {
        let mut c = Command::new("/bin/sh");
        c.arg("-c").arg(script).arg("_").args(args);
        tools::run_tool(c, QUICK, stdin)
    }
}

fn no_cancel() -> Arc<AtomicBool> {
    Arc::new(AtomicBool::new(false))
}

fn run_req(t: &LocalSh, req: &Request, gap: Duration) -> (Outcome, Vec<TransferEvent>) {
    let events = Mutex::new(Vec::new());
    let out = execute("t1", req, t, &no_cancel(), &|e| events.lock().unwrap().push(e), gap);
    (out, events.into_inner().unwrap())
}

fn up(src: &[&Path], remote: &Path) -> Request {
    Request::Upload { sources: src.iter().map(|p| p.to_path_buf()).collect(), remote_dir: remote.to_string_lossy().into_owned() }
}

fn down(src: &[&Path], local: &Path) -> Request {
    Request::Download { sources: src.iter().map(|p| p.to_string_lossy().into_owned()).collect(), local_dir: local.to_path_buf() }
}

fn bytes(n: usize) -> Vec<u8> {
    (0..n).map(|i| (i * 31 % 251) as u8).collect()
}

fn no_parts(dir: &Path) {
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            assert!(!e.file_name().to_string_lossy().ends_with(".codeart-part"), "resto: {:?}", e.path());
            if e.file_type().unwrap().is_dir() {
                stack.push(e.path());
            }
        }
    }
}

/// (percorso relativo, contenuto o `None` per le cartelle), ordinati.
fn tree(root: &Path) -> Vec<(String, Option<Vec<u8>>)> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            let p = e.path();
            let rel = p.strip_prefix(root).unwrap().to_string_lossy().into_owned();
            let ft = e.file_type().unwrap();
            if ft.is_dir() {
                out.push((rel, None));
                stack.push(p);
            } else if ft.is_file() {
                out.push((rel, Some(std::fs::read(&p).unwrap())));
            }
        }
    }
    out.sort();
    out
}

fn dead(pid: i32) -> bool {
    // SAFETY: segnale 0, nessun effetto.
    unsafe { libc::kill(pid, 0) != 0 }
}

#[test]
fn upload_single_file_counts_bytes() {
    let tmp = tempfile::tempdir().unwrap();
    let (local, remote) = (tmp.path().join("l"), tmp.path().join("r"));
    std::fs::create_dir_all(&local).unwrap();
    std::fs::create_dir_all(&remote).unwrap();
    let data = bytes(300_000);
    std::fs::write(local.join("a.bin"), &data).unwrap();
    let t = LocalSh::new(tmp.path());
    let (out, ev) = run_req(&t, &up(&[&local.join("a.bin")], &remote), Duration::ZERO);
    assert_eq!(out.state, TransferState::Done);
    assert_eq!(std::fs::read(remote.join("a.bin")).unwrap(), data);
    no_parts(&remote);
    let last = ev.last().unwrap();
    assert_eq!((last.state, last.done, last.total), (TransferState::Done, 300_000, 300_000));
    assert_eq!(last.errors, Some(vec![]));
    let running: Vec<_> = ev.iter().filter(|e| e.state == TransferState::Running).collect();
    assert!(running.iter().all(|e| e.total == 300_000 || e.total == 0));
    assert!(running.windows(2).all(|w| w[0].done <= w[1].done));
    assert!(running.iter().any(|e| e.done > 0 && e.done < 300_000), "avanzamento intermedio");
    assert!(running.iter().any(|e| e.file == "a.bin"));
    assert!(t.pids().iter().all(|p| dead(*p)));
}

#[test]
fn download_single_file_counts_bytes() {
    let tmp = tempfile::tempdir().unwrap();
    let (local, remote) = (tmp.path().join("l"), tmp.path().join("r"));
    std::fs::create_dir_all(&local).unwrap();
    std::fs::create_dir_all(&remote).unwrap();
    let data = bytes(200_001);
    std::fs::write(remote.join("b.bin"), &data).unwrap();
    let t = LocalSh::new(tmp.path());
    let (out, ev) = run_req(&t, &down(&[&remote.join("b.bin")], &local), Duration::ZERO);
    assert_eq!(out.state, TransferState::Done);
    assert_eq!(std::fs::read(local.join("b.bin")).unwrap(), data);
    no_parts(&local);
    let last = ev.last().unwrap();
    assert_eq!((last.done, last.total), (200_001, 200_001));
    assert_eq!(out.files_copied, 1);
}

fn hostile_tree(root: &Path) {
    std::fs::create_dir_all(root.join("it's \"q\" $HOME `x`")).unwrap();
    std::fs::create_dir_all(root.join("-dash/sub dir")).unwrap();
    std::fs::create_dir_all(root.join("vuota")).unwrap();
    std::fs::write(root.join("it's \"q\" $HOME `x`/a\nb.txt"), b"newline").unwrap();
    std::fs::write(root.join("-dash/sub dir/*.txt"), b"star").unwrap();
    std::fs::write(root.join("-dash/.hidden"), b"dot").unwrap();
    std::fs::write(root.join("-dash/..odd"), b"dotdot").unwrap();
    std::fs::write(root.join("-dash/tab\tname;rm -rf x"), b"tab").unwrap();
    std::fs::write(root.join("è ✓.txt"), "unicode".as_bytes()).unwrap();
    std::fs::write(root.join("vuoto"), b"").unwrap();
    std::fs::write(root.join("-dash/sub dir/big"), bytes(150_000)).unwrap();
}

#[test]
fn nested_directory_with_hostile_names_roundtrips() {
    let tmp = tempfile::tempdir().unwrap();
    let (local, remote, back) = (tmp.path().join("l"), tmp.path().join("r"), tmp.path().join("b"));
    for d in [&remote, &back] {
        std::fs::create_dir_all(d).unwrap();
    }
    let src = local.join("proj");
    hostile_tree(&src);
    std::os::unix::fs::symlink("/etc/passwd", src.join("link")).unwrap();
    let t = LocalSh::new(tmp.path());
    let (out, _) = run_req(&t, &up(&[&src], &remote), EVENT_GAP);
    assert_eq!(out.state, TransferState::Done);
    // il link non si segue e non si copia: è riportato
    assert_eq!(out.errors.len(), 1);
    assert_eq!(out.errors[0].path, "proj/link");
    assert!(out.errors[0].message.contains("collegamento"));
    assert_eq!(tree(&remote.join("proj")), tree(&src));
    no_parts(&remote);
    // di nuovo giù
    let (out, ev) = run_req(&t, &down(&[&remote.join("proj")], &back), EVENT_GAP);
    assert_eq!(out.state, TransferState::Done, "{:?}", out.errors);
    assert!(out.errors.is_empty(), "{:?}", out.errors);
    assert_eq!(tree(&back.join("proj")), tree(&src));
    no_parts(&back);
    let last = ev.last().unwrap();
    assert_eq!(last.done, last.total);
    assert_eq!(last.total, 7 + 4 + 3 + 6 + 3 + 7 + 150_000);
    assert!(t.pids().iter().all(|p| dead(*p)));
}

#[test]
fn download_skips_remote_symlinks_and_reports_them() {
    let tmp = tempfile::tempdir().unwrap();
    let (local, remote) = (tmp.path().join("l"), tmp.path().join("r"));
    std::fs::create_dir_all(&local).unwrap();
    std::fs::create_dir_all(remote.join("d")).unwrap();
    std::fs::write(remote.join("d/f"), b"x").unwrap();
    std::os::unix::fs::symlink("/etc", remote.join("d/to-etc")).unwrap();
    let t = LocalSh::new(tmp.path());
    let (out, _) = run_req(&t, &down(&[&remote.join("d")], &local), EVENT_GAP);
    assert_eq!(out.state, TransferState::Done);
    assert_eq!(out.errors.len(), 1);
    assert_eq!(out.errors[0].path, "d/to-etc");
    assert_eq!(tree(&local), vec![("d".to_string(), None), ("d/f".to_string(), Some(b"x".to_vec()))]);
}

#[test]
fn upload_conflicts_are_per_item_and_never_overwrite() {
    let tmp = tempfile::tempdir().unwrap();
    let (local, remote) = (tmp.path().join("l"), tmp.path().join("r"));
    std::fs::create_dir_all(local.join("dir")).unwrap();
    std::fs::create_dir_all(remote.join("dir")).unwrap();
    std::fs::write(local.join("dir/x"), b"nuovo").unwrap();
    std::fs::write(remote.join("dir/old"), b"vecchio").unwrap();
    std::fs::write(local.join("same"), b"nuovo").unwrap();
    std::fs::write(remote.join("same"), b"vecchio").unwrap();
    std::fs::write(local.join("fresh"), b"ok").unwrap();
    std::os::unix::fs::symlink("/nowhere", remote.join("lnk")).unwrap();
    std::fs::write(local.join("lnk"), b"x").unwrap();
    let t = LocalSh::new(tmp.path());
    let (out, ev) = run_req(&t, &up(&[&local.join("same"), &local.join("dir"), &local.join("fresh"), &local.join("lnk")], &remote), EVENT_GAP);
    assert_eq!(out.state, TransferState::Done);
    let mut got: Vec<_> = out.errors.iter().map(|e| (e.path.as_str(), e.message.as_str())).collect();
    got.sort();
    assert_eq!(got, vec![("dir", EXISTS), ("lnk", EXISTS), ("same", EXISTS)]);
    assert_eq!(std::fs::read(remote.join("same")).unwrap(), b"vecchio");
    assert_eq!(std::fs::read(remote.join("fresh")).unwrap(), b"ok");
    assert!(!remote.join("dir/x").exists());
    no_parts(&remote);
    assert_eq!(ev.last().unwrap().errors.as_ref().unwrap().len(), 3);
}

#[test]
fn download_conflicts_are_per_item_and_nothing_succeeded_is_an_error() {
    let tmp = tempfile::tempdir().unwrap();
    let (local, remote) = (tmp.path().join("l"), tmp.path().join("r"));
    std::fs::create_dir_all(&local).unwrap();
    std::fs::create_dir_all(remote.join("dir")).unwrap();
    std::fs::write(remote.join("dir/f"), b"nuovo").unwrap();
    std::fs::write(remote.join("same"), b"nuovo").unwrap();
    std::fs::write(local.join("same"), b"vecchio").unwrap();
    let t = LocalSh::new(tmp.path());
    let (out, _) = run_req(&t, &down(&[&remote.join("same")], &local), EVENT_GAP);
    assert_eq!(out.state, TransferState::Error);
    assert_eq!(std::fs::read(local.join("same")).unwrap(), b"vecchio");
    assert_eq!(out.errors, vec![err_entry("same", EXISTS)]);
    // una cartella che esiste: errore per lei, il resto continua
    std::fs::create_dir_all(local.join("dir")).unwrap();
    std::fs::write(remote.join("ok"), b"1").unwrap();
    let (out, _) = run_req(&t, &down(&[&remote.join("dir"), &remote.join("ok")], &local), EVENT_GAP);
    assert_eq!(out.state, TransferState::Done);
    assert_eq!(out.errors, vec![err_entry("dir", EXISTS)]);
    assert!(!local.join("dir/f").exists());
    assert_eq!(std::fs::read(local.join("ok")).unwrap(), b"1");
    no_parts(&local);
}

#[test]
fn download_rejects_bad_remote_paths() {
    let tmp = tempfile::tempdir().unwrap();
    let t = LocalSh::new(tmp.path());
    let req = Request::Download { sources: vec!["relativo".into(), "/".into(), "/non/esiste/mai".into()], local_dir: tmp.path().into() };
    let (out, _) = run_req(&t, &req, EVENT_GAP);
    assert_eq!(out.state, TransferState::Error);
    assert_eq!(out.errors.len(), 3);
}

fn cancel_after_first_bytes(req: Request, t: &LocalSh) -> (Outcome, Vec<TransferEvent>) {
    let cancel = no_cancel();
    let c2 = cancel.clone();
    let events = Mutex::new(Vec::new());
    let out = execute(
        "t1",
        &req,
        t,
        &cancel,
        &|e| {
            if e.state == TransferState::Running && e.done > 0 {
                c2.store(true, Ordering::SeqCst);
            }
            events.lock().unwrap().push(e);
        },
        Duration::ZERO,
    );
    (out, events.into_inner().unwrap())
}

#[test]
fn cancel_upload_mid_file_leaves_destination_clean() {
    let tmp = tempfile::tempdir().unwrap();
    let (local, remote) = (tmp.path().join("l"), tmp.path().join("r"));
    std::fs::create_dir_all(&local).unwrap();
    std::fs::create_dir_all(&remote).unwrap();
    std::fs::write(local.join("a-done"), b"piccolo").unwrap();
    std::fs::write(local.join("big"), bytes(40 * 1024 * 1024)).unwrap();
    let t = LocalSh::new(tmp.path());
    // il primo file finisce, durante il secondo si annulla dopo i primi byte
    let cancel = no_cancel();
    let c2 = cancel.clone();
    let seen = Mutex::new(Vec::new());
    let req = up(&[&local.join("a-done"), &local.join("big")], &remote);
    let out = execute(
        "t1",
        &req,
        &t,
        &cancel,
        &|e| {
            if e.state == TransferState::Running && e.file == "big" && e.done > 7 {
                c2.store(true, Ordering::SeqCst);
            }
            seen.lock().unwrap().push(e);
        },
        Duration::ZERO,
    );
    assert_eq!(out.state, TransferState::Cancelled);
    assert!(remote.join("a-done").exists(), "il file completo resta");
    assert!(!remote.join("big").exists());
    no_parts(&remote);
    let last = seen.lock().unwrap().last().unwrap().clone();
    assert_eq!(last.state, TransferState::Cancelled);
    assert!(last.message.unwrap().contains("1 file già copiato"));
    assert!(last.done < last.total);
    assert!(t.pids().iter().all(|p| dead(*p)), "processi ancora vivi: {:?}", t.pids());
}

#[test]
fn cancel_download_mid_file_leaves_destination_clean() {
    let tmp = tempfile::tempdir().unwrap();
    let (local, remote) = (tmp.path().join("l"), tmp.path().join("r"));
    std::fs::create_dir_all(&local).unwrap();
    std::fs::create_dir_all(&remote).unwrap();
    std::fs::write(remote.join("big"), bytes(40 * 1024 * 1024)).unwrap();
    let t = LocalSh::new(tmp.path());
    let (out, ev) = cancel_after_first_bytes(down(&[&remote.join("big")], &local), &t);
    assert_eq!(out.state, TransferState::Cancelled);
    assert!(std::fs::read_dir(&local).unwrap().next().is_none(), "destinazione pulita");
    assert!(ev.last().unwrap().done < 40 * 1024 * 1024);
    assert!(t.pids().iter().all(|p| dead(*p)), "processi ancora vivi");
}

#[test]
fn running_events_are_throttled() {
    let tmp = tempfile::tempdir().unwrap();
    let (local, remote) = (tmp.path().join("l"), tmp.path().join("r"));
    std::fs::create_dir_all(&local).unwrap();
    std::fs::create_dir_all(&remote).unwrap();
    std::fs::write(local.join("a"), bytes(20 * 1024 * 1024)).unwrap(); // 320 pezzi
    let t = LocalSh::new(tmp.path());
    let started = Instant::now();
    let (_, ev) = run_req(&t, &up(&[&local.join("a")], &remote), EVENT_GAP);
    let secs = started.elapsed().as_secs_f64();
    let running = ev.iter().filter(|e| e.state == TransferState::Running).count();
    // 2 eventi forzati (inizio, totale) + al massimo uno ogni 100 ms
    assert!(running as f64 <= secs * 10.0 + 3.0, "{running} eventi in {secs:.2} s");
    assert!(running < 320 / 2);
    assert_eq!(ev.last().unwrap().state, TransferState::Done);
}

#[test]
fn registry_ids_are_unique_and_cancel_is_per_project() {
    let tmp = tempfile::tempdir().unwrap();
    let (local, remote) = (tmp.path().join("l"), tmp.path().join("r"));
    std::fs::create_dir_all(&local).unwrap();
    std::fs::create_dir_all(&remote).unwrap();
    std::fs::write(local.join("big"), bytes(30 * 1024 * 1024)).unwrap();
    std::fs::write(local.join("small"), b"x").unwrap();
    let transfers = Arc::new(Transfers::default());
    let (tx, rx) = std::sync::mpsc::channel::<TransferEvent>();
    let tx = Mutex::new(tx);
    let emit: Emitter = Arc::new(move |e| {
        let _ = tx.lock().unwrap().send(e);
    });
    let t: Arc<dyn Transport> = Arc::new(LocalSh::new(tmp.path()));
    let a = transfers.start("p1", up(&[&local.join("big")], &remote), t.clone(), emit.clone());
    let b = transfers.start("p2", up(&[&local.join("small")], &remote.join("..").join("r")), t, emit);
    assert_ne!(a, b);
    // un altro progetto non annulla il trasferimento di p1
    assert!(!transfers.cancel("p2", &a));
    assert!(!transfers.cancel("p1", "t999"));
    assert!(transfers.cancel("p1", &a));
    let mut finals = std::collections::HashMap::new();
    while finals.len() < 2 {
        let e = rx.recv_timeout(Duration::from_secs(30)).expect("evento finale");
        if e.state != TransferState::Running {
            finals.insert(e.transfer_id.clone(), e.state);
        }
    }
    assert_eq!(finals[&a], TransferState::Cancelled);
    assert_eq!(finals[&b], TransferState::Done);
    // finito: tolto dal registro
    std::thread::sleep(Duration::from_millis(100));
    assert!(!transfers.cancel("p1", &a));
    no_parts(&remote);
}

#[test]
fn scripts_are_constants_and_paths_travel_as_arguments() {
    for s in [UPLOAD_SCRIPT, DOWNLOAD_SCRIPT, EXISTS_SCRIPT, MKDIR_NEW_SCRIPT, REMOVE_SCRIPT, WALK_SCRIPT] {
        assert!(!s.contains("{}"));
    }
    let a = super::super::conn::remote_cmd(UPLOAD_SCRIPT, &["/home/dev/it's here.codeart-part"]);
    assert!(a.ends_with("_ '/home/dev/it'\\''s here.codeart-part'"));
}
