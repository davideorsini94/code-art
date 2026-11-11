//! Verifiche di sicurezza a livello di integrazione: confinamento dei percorsi, permessi Tauri, CSP e
//! inventario dei comandi (ogni `#[tauri::command]` è registrato e documentato in `docs/sicurezza.md`).
use code_art_lib::fs_tree::{self, create, list, mutable_path, read as fs_read, readable_path, rename, write, FsError};
use code_art_lib::store::{Project, Store};
use code_art_lib::trust;
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

const CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; connect-src ipc: http://ipc.localhost; worker-src 'self' blob:; object-src 'none'; base-uri 'none'; form-action 'none'";

fn manifest() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(p: &Path) -> String {
    std::fs::read_to_string(p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

const LABEL: &str = "proj-p1";

/// Store con il progetto `p1` (finestra `proj-p1`) sulle cartelle date.
fn store_with(dir: &Path, folders: &[&Path], trusted: bool) -> Store {
    let store = Store::open(&dir.join("t.db")).unwrap();
    let folders: Vec<String> = folders.iter().map(|f| f.to_string_lossy().into_owned()).collect();
    store.upsert_project(&Project { id: "p1".into(), name: "p".into(), folders: folders.clone() }).unwrap();
    if trusted {
        for f in &folders {
            trust::mark_trusted(&store, f).unwrap();
        }
    }
    store
}

/// Stessa sequenza di `fs_write`: etichetta della finestra → radici (fidate) → `ensure_inside` → scrittura.
fn fs_write(store: &Store, label: &str, path: &Path, text: &str) -> Result<i64, FsError> {
    write(&mutable_path(store, label, &path.to_string_lossy())?, text, None)
}

#[test]
fn fs_write_outside_project_is_rejected() {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path().canonicalize().unwrap();
    let (proj, outside) = (base.join("progetto"), base.join("fuori"));
    std::fs::create_dir_all(proj.join("src")).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("segreto.txt"), "originale").unwrap();
    std::os::unix::fs::symlink(&outside, proj.join("scorciatoia")).unwrap();
    std::os::unix::fs::symlink(outside.join("nuovo.txt"), proj.join("pendente")).unwrap();
    let store = store_with(&base, &[&proj], true);

    let attempts = [
        outside.join("segreto.txt"),                      // fuori dalla cartella
        proj.join("../fuori/segreto.txt"),                // con `..`
        proj.join("src/../../fuori/segreto.txt"),         // `..` annidati
        proj.join("scorciatoia/segreto.txt"),             // link simbolico che esce dal progetto
        proj.join("scorciatoia/nuovo.txt"),               // file nuovo attraverso il link
        proj.join("pendente"),                            // link pendente verso l'esterno
        proj.join("src/nuova/../../../fuori/altro.txt"),  // `..` dopo componenti inesistenti
        PathBuf::from("relativo.txt"),                    // percorso relativo
    ];
    for p in &attempts {
        let err = fs_write(&store, LABEL, p, "compromesso").expect_err(&p.display().to_string());
        assert_eq!(err.code, "forbidden", "{}", p.display());
        assert_eq!(serde_json::to_value(&err).unwrap()["code"], "forbidden");
    }
    // le finestre non di progetto non hanno radici
    for label in ["welcome", "proj-", "proj-altro"] {
        assert!(fs_write(&store, label, &proj.join("src/x.rs"), "x").is_err(), "{label}");
    }
    assert_eq!(read(&outside.join("segreto.txt")), "originale");
    assert!(!outside.join("nuovo.txt").exists() && !outside.join("altro.txt").exists());

    // dentro il progetto si scrive
    fs_write(&store, LABEL, &proj.join("src/main.rs"), "fn main() {}").unwrap();
    assert_eq!(read(&proj.join("src/main.rs")), "fn main() {}");
}

#[test]
fn untrusted_project_is_read_only() {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path().canonicalize().unwrap();
    let (a, b) = (base.join("a"), base.join("b"));
    std::fs::create_dir_all(&a).unwrap();
    std::fs::create_dir_all(&b).unwrap();
    std::fs::write(a.join("f.txt"), "uno").unwrap();
    // `a` fidata, `b` no: basta una cartella non fidata per la sola lettura
    let store = store_with(&base, &[&a, &b], false);
    trust::mark_trusted(&store, &a.to_string_lossy()).unwrap();
    let p = |path: PathBuf| path.to_string_lossy().into_owned();
    let refused = |r: Result<PathBuf, FsError>| assert_eq!(r.unwrap_err().code, trust::UNTRUSTED);

    // fs_write, fs_create, fs_rename (entrambi i capi), fs_trash: rifiutati
    refused(mutable_path(&store, LABEL, &p(a.join("f.txt"))));
    refused(mutable_path(&store, LABEL, &p(a.join("nuovo.txt"))));
    refused(mutable_path(&store, LABEL, &p(b.join(".git/hooks/pre-commit"))));
    assert_eq!(fs_write(&store, LABEL, &a.join("f.txt"), "due").unwrap_err().code, trust::UNTRUSTED);
    assert_eq!(read(&a.join("f.txt")), "uno");
    // lettura, elenco, ricerca dei file: permessi
    assert_eq!(fs_read(&readable_path(&store, LABEL, &p(a.join("f.txt"))).unwrap()).unwrap().text, "uno");
    assert_eq!(list(&readable_path(&store, LABEL, &p(a.clone())).unwrap()).unwrap().len(), 1);
    assert_eq!(fs_tree::all_files(&fs_tree::roots_for_label(&store, LABEL).unwrap()).len(), 1);

    // fidate tutte: le modifiche passano
    trust::mark_trusted(&store, &b.to_string_lossy()).unwrap();
    fs_write(&store, LABEL, &a.join("f.txt"), "due").unwrap();
    create(&mutable_path(&store, LABEL, &p(b.join("nuovo.txt"))).unwrap(), false).unwrap();
    rename(&mutable_path(&store, LABEL, &p(b.join("nuovo.txt"))).unwrap(), &mutable_path(&store, LABEL, &p(b.join("x.txt"))).unwrap()).unwrap();
    assert!(b.join("x.txt").is_file());
    // fs_trash usa lo stesso `mutable_path` (non lo si esegue: sposterebbe file nel Cestino dell'utente)
    assert!(mutable_path(&store, LABEL, &p(b.join("x.txt"))).is_ok());
    assert_eq!(read(&a.join("f.txt")), "due");
}

fn capabilities() -> Vec<(String, Value)> {
    let dir = manifest().join("capabilities");
    let mut out: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .map(|e| (e.file_name().to_string_lossy().into_owned(), serde_json::from_str(&read(&e.path())).unwrap()))
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    assert!(!out.is_empty());
    out
}

fn permission_ids(cap: &Value) -> Vec<String> {
    cap["permissions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p.as_str().or_else(|| p["identifier"].as_str()).unwrap().to_string())
        .collect()
}

#[test]
fn capabilities_grant_no_shell_or_fs_plugin() {
    for (file, cap) in capabilities() {
        let ids = permission_ids(&cap);
        for id in &ids {
            assert!(!id.starts_with("shell:") && !id.starts_with("fs:"), "{file}: {id}");
            // né insiemi ampi (core:default, dialog:default, …) né plugin che la WebView non usa
            assert!(!id.ends_with(":default") && !id.starts_with("dialog:") && !id.starts_with("notification:"), "{file}: {id}");
            assert!(!id.contains("allow-emit"), "{file}: la WebView non emette eventi verso altre finestre ({id})");
        }
        for p in cap["permissions"].as_array().unwrap().iter().filter(|p| p["identifier"] == "opener:allow-open-url") {
            for scope in p["allow"].as_array().unwrap() {
                let url = scope["url"].as_str().unwrap();
                assert!(url == "https://*" || url == "http://*", "{file}: opener limitato a http(s), trovato {url}");
            }
            assert!(p.get("deny").is_none());
        }
        assert!(!cap["windows"].as_array().unwrap().iter().any(|w| w == "*"), "{file}: finestre esplicite");
    }
    // la finestra di benvenuto non apre link e non ascolta eventi
    let (_, welcome) = capabilities().into_iter().find(|(_, c)| c["windows"] == serde_json::json!(["welcome"])).unwrap();
    assert!(permission_ids(&welcome).iter().all(|id| id.starts_with("core:window:")));
}

#[test]
fn csp_has_no_unsafe_eval_or_remote_origin() {
    let conf: Value = serde_json::from_str(&read(&manifest().join("tauri.conf.json"))).unwrap();
    let sec = &conf["app"]["security"];
    assert_eq!(sec["csp"], CSP);
    let dev = sec["devCsp"].as_str().unwrap();
    assert_eq!(dev, CSP.replace("http://ipc.localhost;", "http://ipc.localhost ws://localhost:1420;"));
    assert!(CSP.contains("form-action 'none'") && CSP.contains("object-src 'none'") && CSP.contains("base-uri 'none'"));
    for csp in [CSP, dev] {
        assert!(!csp.contains("unsafe-eval") && !csp.contains("wasm-unsafe-eval"));
        assert!(!csp.contains('*'));
        for token in csp.split([' ', ';']).filter(|t| t.contains(':')) {
            let allowed = ["data:", "blob:", "ipc:", "http://ipc.localhost", "ws://localhost:1420"];
            assert!(allowed.contains(&token), "origine non ammessa nella CSP: {token}");
        }
    }
    assert_eq!(sec["dangerousDisableAssetCspModification"], false);
    assert!(sec.get("assetProtocol").is_none());
    assert_ne!(conf["app"]["withGlobalTauri"], true);
}

/// Nomi delle funzioni marcate `#[tauri::command]` nei sorgenti.
fn declared_commands() -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut stack = vec![manifest().join("src")];
    while let Some(dir) = stack.pop() {
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
                continue;
            }
            if p.extension().is_none_or(|x| x != "rs") {
                continue;
            }
            let text = read(&p);
            let mut lines = text.lines();
            while let Some(l) = lines.next() {
                if l.trim() != "#[tauri::command]" {
                    continue;
                }
                let sig = lines.by_ref().map(str::trim).find(|l| !l.starts_with("#[")).unwrap();
                let name = sig.split("fn ").nth(1).unwrap().split(['(', '<']).next().unwrap();
                out.insert(name.to_string());
            }
        }
    }
    out
}

#[test]
fn every_command_is_registered_and_listed() {
    let declared = declared_commands();
    let lib = read(&manifest().join("src/lib.rs"));
    let handler = lib.split("generate_handler![").nth(1).unwrap().split(']').next().unwrap();
    let registered: BTreeSet<String> = handler
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.rsplit("::").next().unwrap().to_string())
        .collect();
    assert_eq!(declared, registered, "comandi dichiarati e registrati devono coincidere");
    let doc = read(&manifest().join("../docs/sicurezza.md"));
    let missing: Vec<_> = declared.iter().filter(|c| !doc.contains(&format!("| `{c}` |"))).collect();
    assert!(missing.is_empty(), "comandi non elencati in docs/sicurezza.md: {missing:?}");
    for gone in ["project_open_folder", "project_add_folder", "project_relocate"] {
        assert!(!declared.contains(gone), "{gone} accetterebbe un percorso dalla WebView");
    }
}
