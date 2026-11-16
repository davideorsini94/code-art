use super::*;
use crate::store::{Project, Store};
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Store con il progetto "p" sulle cartelle date (canonicalizzate), fidato o no.
fn project(folders: &[&Path], trusted: bool) -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let s = Store::open(&dir.path().join("t.db")).unwrap();
    let folders: Vec<String> = folders.iter().map(|f| f.canonicalize().unwrap().to_string_lossy().into_owned()).collect();
    s.upsert_project(&Project { id: "p".into(), name: "p".into(), folders: folders.clone() }).unwrap();
    if trusted {
        for f in &folders {
            crate::trust::mark_trusted(&s, f).unwrap();
        }
    }
    (dir, s)
}

/// Lancio del server finto (`tests/fake_lsp.sh`) con un ambiente minimo più `extra`.
fn fake(extra: &[(&str, &str)]) -> impl FnOnce(&Path) -> Result<Launch> + use<> {
    let mut env: HashMap<String, String> = [("PATH".to_string(), "/usr/bin:/bin".to_string()), ("HOME".to_string(), "/nonexistent".to_string())].into();
    env.extend(extra.iter().map(|(k, v)| (k.to_string(), v.to_string())));
    move |root| {
        Ok(Launch { program: Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fake_lsp.sh"), args: vec![], env, cwd: root.to_path_buf() })
    }
}

/// Eventi ricevuti, in ordine.
#[derive(Clone, Default)]
struct Events(Arc<Mutex<Vec<LspEvent>>>);

impl Events {
    fn emit(&self) -> Emit {
        let e = self.0.clone();
        Arc::new(move |ev| crate::lock(&e).push(ev))
    }

    /// Aspetta (al massimo 5 s) un evento che soddisfa `f`.
    fn wait(&self, f: impl Fn(&LspEvent) -> bool) -> LspEvent {
        let t0 = Instant::now();
        loop {
            if let Some(ev) = crate::lock(&self.0).iter().find(|e| f(e)) {
                return ev.clone();
            }
            assert!(t0.elapsed() < Duration::from_secs(5), "evento mai arrivato; ricevuti: {:?}", crate::lock(&self.0));
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn exits(&self) -> usize {
        crate::lock(&self.0).iter().filter(|e| matches!(e, LspEvent::Exit { .. })).count()
    }
}

/// `true` se `pid` sparisce entro qualche secondo.
fn gone(pid: i32) -> bool {
    let t0 = Instant::now();
    while t0.elapsed() < Duration::from_secs(5) {
        // SAFETY: segnale 0 = solo verifica di esistenza
        if unsafe { libc::kill(pid, 0) } != 0 {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

fn read_pids(p: &Path) -> Vec<i32> {
    let t0 = Instant::now();
    loop {
        let pids: Vec<i32> = std::fs::read_to_string(p).unwrap_or_default().lines().filter_map(|l| l.trim().parse().ok()).collect();
        if pids.len() == 2 {
            return pids;
        }
        assert!(t0.elapsed() < Duration::from_secs(5), "pid mai scritti");
        std::thread::sleep(Duration::from_millis(10));
    }
}

const INIT: &str = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"processId":null,"rootUri":null,"capabilities":{}}}"#;

#[test]
fn roundtrip_with_fake_server() {
    let root = tempfile::tempdir().unwrap();
    let (_db, store) = project(&[root.path()], true);
    let lsp = Arc::new(Lsp::default());
    let ev = Events::default();
    let s = lsp.start(&store, "p", "python", fake(&[]), ev.emit()).unwrap();
    let canon = root.path().canonicalize().unwrap();
    assert_eq!(s.root_uri, format!("file://{}", canon.display()));
    // un server per progetto e linguaggio, riusato
    let again = lsp.start(&store, "p", "python", |_| panic!("non deve rilanciare"), ev.emit()).unwrap();
    assert_eq!(again, s);

    lsp.send(&store, "p", s.id, INIT).unwrap();
    let LspEvent::Message { id, message } = ev.wait(|e| matches!(e, LspEvent::Message { message, .. } if message.contains("fake-lsp"))) else { unreachable!() };
    assert_eq!(id, s.id);
    let v: Value = serde_json::from_str(&message).unwrap();
    assert_eq!(v["result"]["capabilities"]["textDocumentSync"], 1);

    let uri = format!("file://{}/main.py", canon.display());
    let note = format!(r#"{{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{{"textDocument":{{"uri":"{uri}","languageId":"python","version":1,"text":"x = 1\n"}}}}}}"#);
    lsp.send(&store, "p", s.id, &note).unwrap();
    ev.wait(|e| matches!(e, LspEvent::Message { message, .. } if message.contains("didOpen") && message.contains("main.py")));

    // l'id di un altro progetto non vale
    assert!(lsp.send(&store, "altro", s.id, &note).is_err());
    lsp.stop("p", s.id);
    assert!(lsp.send(&store, "p", s.id, &note).is_err());
    assert_eq!(ev.exits(), 0, "uno stop voluto non è un'uscita da segnalare");
}

#[test]
fn refused_when_untrusted() {
    let root = tempfile::tempdir().unwrap();
    let (_db, store) = project(&[root.path()], false);
    let lsp = Arc::new(Lsp::default());
    let e = lsp.start(&store, "p", "python", |_| panic!("non deve partire"), Events::default().emit()).unwrap_err();
    assert_eq!(e, crate::trust::UNTRUSTED);
    assert_eq!(lsp.start(&store, "nessuno", "python", |_| panic!("non deve partire"), Events::default().emit()).unwrap_err(), crate::trust::UNTRUSTED);
    // fidato ma linguaggio sconosciuto
    let (_db2, ok) = project(&[root.path()], true);
    assert!(lsp.start(&ok, "p", "cobol", |_| panic!("non deve partire"), Events::default().emit()).is_err());
    // un server avviato smette di ricevere messaggi se il progetto non è più fidato
    let ev = Events::default();
    let s = lsp.start(&ok, "p", "go", fake(&[]), ev.emit()).unwrap();
    assert_eq!(lsp.send(&store, "p", s.id, INIT).unwrap_err(), crate::trust::UNTRUSTED);
    lsp.stop("p", s.id);
}

#[test]
fn stop_kills_process_group() {
    let root = tempfile::tempdir().unwrap();
    let (_db, store) = project(&[root.path()], true);
    let pidf = root.path().join("pids");
    let lsp = Arc::new(Lsp::default());
    let s = lsp.start(&store, "p", "rust", fake(&[("FAKE_LSP_PIDS", pidf.to_str().unwrap())]), Events::default().emit()).unwrap();
    let pids = read_pids(&pidf);
    // un altro progetto non può fermarlo
    lsp.stop("altro", s.id);
    assert!(!gone_now(pids[0]));
    lsp.stop("p", s.id);
    for pid in pids {
        assert!(gone(pid), "processo {pid} ancora vivo");
    }
}

fn gone_now(pid: i32) -> bool {
    // SAFETY: segnale 0 = solo verifica di esistenza
    unsafe { libc::kill(pid, 0) != 0 }
}

#[test]
fn stop_project_stops_every_server_of_that_project() {
    let root = tempfile::tempdir().unwrap();
    let (_db, store) = project(&[root.path()], true);
    let (pa, pb) = (root.path().join("a"), root.path().join("b"));
    let lsp = Arc::new(Lsp::default());
    lsp.start(&store, "p", "go", fake(&[("FAKE_LSP_PIDS", pa.to_str().unwrap())]), Events::default().emit()).unwrap();
    lsp.start(&store, "p", "rust", fake(&[("FAKE_LSP_PIDS", pb.to_str().unwrap())]), Events::default().emit()).unwrap();
    let pids: Vec<i32> = read_pids(&pa).into_iter().chain(read_pids(&pb)).collect();
    lsp.stop_project("p");
    for pid in pids {
        assert!(gone(pid), "processo {pid} ancora vivo");
    }
}

#[test]
fn startup_failure_reports_stderr() {
    let root = tempfile::tempdir().unwrap();
    let (_db, store) = project(&[root.path()], true);
    let lsp = Arc::new(Lsp::default());
    let ev = Events::default();
    let e = lsp.start(&store, "p", "go", fake(&[("FAKE_LSP_MODE", "die")]), ev.emit()).unwrap_err();
    assert!(e.contains("configurazione mancante") && e.contains('7'), "{e}");
    // nessun riavvio né server rimasto: il prossimo avvio riprova
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(ev.exits(), 0);
    assert!(lsp.start(&store, "p", "go", fake(&[]), ev.emit()).is_ok());
}

#[test]
fn crash_restarts_then_halts() {
    let root = tempfile::tempdir().unwrap();
    let (_db, store) = project(&[root.path()], true);
    let lsp = Arc::new(Lsp::default());
    let ev = Events::default();
    let s = lsp.start(&store, "p", "go", fake(&[("FAKE_LSP_MODE", "crash")]), ev.emit()).unwrap();
    for n in 1..=4 {
        // ogni messaggio fa uscire il server con 3; dopo l'uscita lo stesso id resta valido finché c'è budget
        lsp.send(&store, "p", s.id, INIT).unwrap();
        let t0 = Instant::now();
        while ev.exits() < n {
            assert!(t0.elapsed() < Duration::from_secs(5), "uscita {n} mai segnalata");
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(matches!(ev.wait(|e| matches!(e, LspEvent::Exit { .. })), LspEvent::Exit { id, code: 3 } if id == s.id));
    }
    // tre riavvii, poi "fermo" fino alla riapertura del progetto
    assert!(lsp.send(&store, "p", s.id, INIT).is_err());
    let e = lsp.start(&store, "p", "go", |_| panic!("fermo: non deve ripartire"), ev.emit()).unwrap_err();
    assert!(e.contains("fermo"), "{e}");
    lsp.stop_project("p");
    assert!(lsp.start(&store, "p", "go", fake(&[]), ev.emit()).is_ok());
}

#[test]
fn malformed_output_closes_the_server() {
    let root = tempfile::tempdir().unwrap();
    let (_db, store) = project(&[root.path()], true);
    let lsp = Arc::new(Lsp::default());
    let ev = Events::default();
    let s = lsp.start(&store, "p", "go", fake(&[("FAKE_LSP_MODE", "garbage")]), ev.emit()).unwrap();
    lsp.send(&store, "p", s.id, INIT).unwrap();
    assert!(matches!(ev.wait(|e| matches!(e, LspEvent::Exit { .. })), LspEvent::Exit { id, .. } if id == s.id));
    lsp.stop_project("p");
}

#[test]
fn restart_budget_three_in_five_minutes() {
    let t0 = Instant::now();
    let mut b = RestartBudget::default();
    assert!(b.allow(t0));
    assert!(b.allow(t0 + Duration::from_secs(60)));
    assert!(b.allow(t0 + Duration::from_secs(120)));
    assert!(!b.allow(t0 + Duration::from_secs(180)), "quarto riavvio entro 5 minuti");
    assert!(!b.allow(t0 + Duration::from_secs(299)));
    // il primo esce dalla finestra di 5 minuti: ne torna disponibile uno
    assert!(b.allow(t0 + Duration::from_secs(301)));
    assert!(!b.allow(t0 + Duration::from_secs(302)));
}

#[test]
fn message_must_be_json_rpc() {
    let roots: Vec<PathBuf> = vec![];
    for bad in ["", "[]", "42", r#"{"method":"x"}"#, r#"{"jsonrpc":"1.0","method":"x"}"#, r#"{"jsonrpc":"2.0"}"#, r#"{"jsonrpc":"2.0","method":5}"#, "{"] {
        assert!(check_message(bad, &roots).is_err(), "{bad}");
    }
    for ok in [
        r#"{"jsonrpc":"2.0","method":"initialized","params":{}}"#,
        r#"{"jsonrpc":"2.0","id":3,"method":"workspace/executeCommand","params":{"command":"x","arguments":["jdt://contents/a"]}}"#,
        r#"{"jsonrpc":"2.0","id":7,"result":null}"#,
        r#"{"jsonrpc":"2.0","id":"a","error":{"code":-32601,"message":"no"}}"#,
        r#"{"jsonrpc":"2.0","method":"$/qualunque","params":{"uri":"untitled:Untitled-1"}}"#,
    ] {
        assert!(check_message(ok, &roots).is_ok(), "{ok}");
    }
}

#[test]
fn file_uri_outside_project_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path().canonicalize().unwrap();
    let (proj, outside) = (base.join("progetto"), base.join("fuori"));
    std::fs::create_dir_all(proj.join("src")).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("segreto.py"), "x").unwrap();
    std::os::unix::fs::symlink(&outside, proj.join("scorciatoia")).unwrap();
    let roots = vec![proj.clone()];
    let p = proj.display();
    let o = outside.display();
    let msg = |params: String| format!(r#"{{"jsonrpc":"2.0","id":1,"method":"textDocument/definition","params":{params}}}"#);
    let at = |uri: String| msg(format!(r#"{{"textDocument":{{"uri":"{uri}"}},"position":{{"line":0,"character":0}}}}"#));
    let refused = [
        at(format!("file://{o}/segreto.py")),
        at(format!("file://{p}/../fuori/segreto.py")),
        at(format!("file://{p}/src/%2E%2E/%2E%2E/fuori/segreto.py")),
        at(format!("file://{p}/src/..%2F..%2Ffuori/segreto.py")),
        at(format!("file://{p}/scorciatoia/segreto.py")),
        at(format!("FILE://{o}/segreto.py")),
        at(format!("file://altrohost{p}/src/a.py")),
        at("file:///etc/passwd".to_string()),
        // annidato, in un array o come chiave (`WorkspaceEdit.changes`)
        msg(format!(r#"{{"edit":{{"changes":{{"file://{o}/segreto.py":[]}}}}}}"#)),
        msg(format!(r#"{{"items":[{{"x":["file://{p}/src/a.py","file:///etc/hosts"]}}]}}"#)),
        msg(format!(r#"{{"command":"x","arguments":["file://{o}"]}}"#)),
        // `rootPath` è un percorso, non un URI
        msg(format!(r#"{{"rootPath":"{o}"}}"#)),
    ];
    for m in &refused {
        assert!(check_message(m, &roots).is_err(), "{m}");
    }
    let accepted = [
        at(format!("file://{p}/src/a.py")),
        at(format!("file://{p}/src/nuovo%20file.py")),
        at(format!("file://localhost{p}/src/a.py")),
        msg(format!(r#"{{"rootPath":"{p}","rootUri":"file://{p}","workspaceFolders":[{{"uri":"file://{p}","name":"p"}}]}}"#)),
        // il testo di un documento non è un URI
        msg(format!(r#"{{"textDocument":{{"uri":"file://{p}/a.py","text":"file:///etc/passwd"}}}}"#)),
        at("jdt://contents/rt.jar/java.lang/String.class".to_string()),
    ];
    for m in &accepted {
        assert!(check_message(m, &roots).is_ok(), "{m}");
    }
}
