//! Verifiche di sicurezza a livello di integrazione: confinamento dei percorsi, permessi Tauri, CSP e
//! inventario dei comandi (ogni `#[tauri::command]` è registrato e documentato in `docs/sicurezza.md`).
use code_art_lib::fs_tree::{ensure_inside, write};
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

const CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; connect-src ipc: http://ipc.localhost; worker-src 'self' blob:; object-src 'none'; base-uri 'none'";

fn manifest() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(p: &Path) -> String {
    std::fs::read_to_string(p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

/// Stessa sequenza dei comandi `fs_*`: `ensure_inside` sulle cartelle del progetto, poi l'operazione.
fn fs_write(roots: &[PathBuf], path: &Path, text: &str) -> Result<i64, code_art_lib::fs_tree::FsError> {
    write(&ensure_inside(roots, path)?, text, None)
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
    let roots = vec![proj.clone()];

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
        let err = fs_write(&roots, p, "compromesso").expect_err(&p.display().to_string());
        assert_eq!(err.code, "forbidden", "{}", p.display());
        assert_eq!(serde_json::to_value(&err).unwrap()["code"], "forbidden");
    }
    assert_eq!(read(&outside.join("segreto.txt")), "originale");
    assert!(!outside.join("nuovo.txt").exists() && !outside.join("altro.txt").exists());

    // dentro il progetto si scrive
    fs_write(&roots, &proj.join("src/main.rs"), "fn main() {}").unwrap();
    assert_eq!(read(&proj.join("src/main.rs")), "fn main() {}");
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
