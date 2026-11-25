//! Oracle Instant Client: rilevamento, validazione e caricamento (una volta per processo).
//! La libreria non è mai scaricata dall'app: la cartella la sceglie l'utente (selettore nativo) o
//! è fra i percorsi noti, e viene accettata solo se contiene `libclntsh.dylib` come file regolare.

use crate::ssh;
use crate::store::Store;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::{AppHandle, WebviewWindow};

/// Chiave nello store dell'app. Scritta solo dal core (selettore o percorso rilevato e riconvalidato).
pub const CLIENT_DIR_KEY: &str = "oracle_client_dir";
const LIB_NAME: &str = "libclntsh.dylib";

/// Cartella con cui la libreria è stata caricata (solo i successi: un errore resta riprovabile).
static LOADED: Mutex<Option<PathBuf>> = Mutex::new(None);

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OracleClientStatus {
    pub dir: Option<String>,
    pub loaded: bool,
    pub detected: Vec<String>,
    pub error: Option<String>,
}

/// Cartella esistente con `libclntsh.dylib` regolare (o un link che resta dentro la cartella); canonicalizzata.
pub fn validate_client_dir(dir: &Path) -> Result<PathBuf, String> {
    let canon = std::fs::canonicalize(dir).map_err(|_| "la cartella non esiste".to_string())?;
    if !canon.is_dir() {
        return Err("non è una cartella".into());
    }
    let lib = canon.join(LIB_NAME);
    let meta = std::fs::symlink_metadata(&lib).map_err(|_| format!("la cartella non contiene {LIB_NAME}"))?;
    if meta.file_type().is_symlink() {
        let target = std::fs::canonicalize(&lib).map_err(|_| format!("{LIB_NAME} è un collegamento rotto"))?;
        if !target.starts_with(&canon) || !target.is_file() {
            return Err(format!("{LIB_NAME} è un collegamento che esce dalla cartella"));
        }
    } else if !meta.is_file() {
        return Err(format!("{LIB_NAME} non è un file regolare"));
    }
    Ok(canon)
}

fn detect_in(home: Option<&Path>, system: &[&Path]) -> Vec<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    let mut scan = |parent: &Path| {
        if let Ok(rd) = std::fs::read_dir(parent) {
            for e in rd.flatten() {
                if e.file_name().to_string_lossy().starts_with("instantclient") {
                    candidates.push(e.path());
                }
            }
        }
    };
    if let Some(h) = home {
        scan(&h.join("Downloads"));
        scan(h);
    }
    scan(Path::new("/opt/oracle"));
    candidates.extend(system.iter().map(|p| p.to_path_buf()));
    let mut out: Vec<PathBuf> = candidates.iter().filter_map(|c| validate_client_dir(c).ok()).collect();
    out.sort();
    out.dedup();
    out
}

/// Cartelle note con un Instant Client valido, ordinate e senza doppioni.
pub fn detect() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    detect_in(home.as_deref(), &[Path::new("/usr/local/lib"), Path::new("/opt/homebrew/lib")])
}

/// Traduce un errore di caricamento (ODPI-C/dlopen) in italiano con il rimedio. Il testo originale non entra nella UI.
pub fn translate_load_error(msg: &str, dir: &Path) -> String {
    let low = msg.to_lowercase();
    let d = dir.display();
    if low.contains("incompatible architecture") || low.contains("mach-o") || low.contains("wrong architecture") {
        "Instant Client per un'architettura diversa: scarica la versione ARM64 (o quella adatta al tuo Mac)".into()
    } else if low.contains("team id") || low.contains("library validation") || low.contains("code signature") {
        // hardened runtime senza il permesso disable-library-validation (Entitlements.plist): una build firmata
        // senza quel file. La quarantena qui non c'entra e `xattr` non servirebbe.
        "macOS ha rifiutato la firma di Instant Client: questa build di Code Art non ha il permesso di caricare librerie di altri sviluppatori (ricostruiscila con Entitlements.plist)".into()
    } else if low.contains("quarantine") || low.contains("not verified") {
        format!("macOS ha messo in quarantena la libreria: esegui `xattr -dr com.apple.quarantine '{d}'` nel Terminale e riprova")
    } else if low.contains("dpi-1047") {
        "Libreria di Instant Client non trovata nella cartella".into()
    } else {
        "Impossibile caricare Instant Client dalla cartella scelta".into()
    }
}

/// Carica la libreria da `dir` (una sola volta per processo). Errore di caricamento = si può riprovare con un'altra cartella.
pub fn ensure_initialized(dir: &Path) -> Result<(), String> {
    let canon = validate_client_dir(dir)?;
    let mut loaded = LOADED.lock().map_err(|_| "stato interno non disponibile".to_string())?;
    if let Some(cur) = loaded.as_ref() {
        return if *cur == canon { Ok(()) } else { Err("riavvia l'app per usare un'altra cartella".into()) };
    }
    let res = oracle::InitParams::new().oracle_client_lib_dir(&canon).map_err(|e| e.to_string()).and_then(|p| p.init().map_err(|e| e.to_string()));
    match res {
        Ok(_) => {
            *loaded = Some(canon);
            Ok(())
        }
        Err(raw) => {
            eprintln!("oracle: caricamento di Instant Client non riuscito: {raw}");
            Err(translate_load_error(&raw, &canon))
        }
    }
}

fn status_with(store: &Store, detected: Vec<PathBuf>) -> OracleClientStatus {
    let dir = store.setting_get(CLIENT_DIR_KEY).ok().flatten().filter(|d| !d.is_empty());
    let mut error = None;
    let mut loaded = false;
    if let Some(d) = &dir {
        match ensure_initialized(Path::new(d)) {
            Ok(()) => loaded = true,
            Err(e) => error = Some(e),
        }
    }
    OracleClientStatus { dir, loaded, detected: detected.iter().map(|p| p.to_string_lossy().into_owned()).collect(), error }
}

/// Salva la cartella (già validata) e prova a caricarla; l'errore di caricamento va nello stato, la scelta resta.
fn use_dir(store: &Store, dir: &Path) -> Result<(), String> {
    let canon = validate_client_dir(dir)?;
    store.setting_set(CLIENT_DIR_KEY, &canon.to_string_lossy())
}

/// Cartella da cui aprire il selettore: un suggerimento della pagina vale solo se è fra le cartelle rilevate ora,
/// altrimenti la cartella salvata. È solo il punto di partenza: la scelta resta all'utente nel selettore nativo.
fn picker_start(hint: Option<&str>, detected: &[PathBuf], saved: Option<String>) -> Option<PathBuf> {
    hint.and_then(|h| validate_client_dir(Path::new(h)).ok()).filter(|c| detected.contains(c)).or_else(|| saved.map(PathBuf::from))
}

#[tauri::command]
pub async fn db_oracle_client_status(app: AppHandle, window: WebviewWindow) -> Result<OracleClientStatus, String> {
    let store = ssh::trusted_store(&app, &window)?;
    crate::projects::off_main(move || Ok(status_with(&store, detect()))).await
}

#[tauri::command]
pub async fn db_oracle_client_pick(app: AppHandle, window: WebviewWindow, start: Option<String>) -> Result<Option<OracleClientStatus>, String> {
    let store = ssh::trusted_store(&app, &window)?;
    // la libreria si carica solo da una cartella confermata nel selettore nativo: con il permesso
    // disable-library-validation la pagina non deve poter caricare una cartella da sola
    let st0 = store.clone();
    let start = crate::projects::off_main(move || Ok(picker_start(start.as_deref(), &detect(), st0.setting_get(CLIENT_DIR_KEY).ok().flatten()))).await?;
    let Some(picked) = crate::projects::pick_folder_in(&app, &window, "Scegli la cartella di Oracle Instant Client", start).await? else {
        return Ok(None);
    };
    let st = store.clone();
    crate::projects::off_main(move || {
        use_dir(&st, Path::new(&picked))?;
        Ok(Some(status_with(&st, detect())))
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    fn td() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }
    fn fake_client(dir: &Path) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join(LIB_NAME), b"non un mach-o").unwrap();
    }

    #[test]
    fn validate_cases() {
        let t = td();
        let empty = t.path().join("vuota");
        std::fs::create_dir(&empty).unwrap();
        assert!(validate_client_dir(&empty).is_err());
        assert!(validate_client_dir(&t.path().join("nope")).is_err());
        let ok = t.path().join("ok");
        fake_client(&ok);
        assert_eq!(validate_client_dir(&ok).unwrap(), std::fs::canonicalize(&ok).unwrap());
        // link dentro la cartella
        let inl = t.path().join("inl");
        std::fs::create_dir(&inl).unwrap();
        std::fs::write(inl.join("libclntsh.dylib.23.1"), b"x").unwrap();
        symlink("libclntsh.dylib.23.1", inl.join(LIB_NAME)).unwrap();
        assert!(validate_client_dir(&inl).is_ok());
        // link che esce
        let out = t.path().join("out");
        std::fs::create_dir(&out).unwrap();
        symlink(ok.join(LIB_NAME), out.join(LIB_NAME)).unwrap();
        assert!(validate_client_dir(&out).is_err());
        // libclntsh.dylib è una cartella
        let d = t.path().join("dirlib");
        std::fs::create_dir_all(d.join(LIB_NAME)).unwrap();
        assert!(validate_client_dir(&d).is_err());
    }

    #[test]
    fn detect_with_fake_home() {
        let t = td();
        let home = t.path().join("home");
        fake_client(&home.join("Downloads/instantclient_23_3"));
        fake_client(&home.join("instantclient_19"));
        std::fs::create_dir_all(home.join("Downloads/instantclient_vuoto")).unwrap();
        fake_client(&home.join("Downloads/altro"));
        let sys = t.path().join("lib");
        fake_client(&sys);
        let got = detect_in(Some(&home), &[&sys, &sys, &t.path().join("manca")]);
        let canon = |p: PathBuf| std::fs::canonicalize(p).unwrap();
        let mut want = vec![canon(home.join("Downloads/instantclient_23_3")), canon(home.join("instantclient_19")), canon(sys)];
        want.sort();
        assert_eq!(got, want);
        assert!(detect_in(None, &[]).iter().all(|p| p.join(LIB_NAME).exists()));
    }

    #[test]
    fn error_translation() {
        let d = Path::new("/x/ic");
        assert!(translate_load_error("DPI-1047: Cannot locate a 64-bit Oracle Client library: \"dlopen(...)\"", d).contains("non trovata"));
        assert!(translate_load_error("dlopen: (mach-o file, but is an incompatible architecture (have 'x86_64', need 'arm64'))", d).contains("ARM64"));
        let q = translate_load_error("library load disallowed: quarantine attribute, not verified", d);
        assert!(q.contains("xattr -dr com.apple.quarantine '/x/ic'"));
        // hardened runtime: firma di un altro sviluppatore, xattr non serve
        let t = translate_load_error("code signature in <…/libclntsh.dylib> not valid for use in process: mapping process and mapped file (non-platform) have different Team IDs", d);
        assert!(t.contains("Entitlements.plist") && !t.contains("xattr"));
        assert!(!translate_load_error("boh", d).contains("boh"));
    }

    #[test]
    fn use_dir_saves_the_canonical_folder() {
        let t = td();
        let store = Store::open(&t.path().join("s.db")).unwrap();
        assert!(use_dir(&store, &t.path().join("manca")).is_err());
        assert_eq!(store.setting_get(CLIENT_DIR_KEY).unwrap(), None);
        let a = t.path().join("a");
        fake_client(&a);
        use_dir(&store, &a).unwrap();
        assert_eq!(store.setting_get(CLIENT_DIR_KEY).unwrap().unwrap(), std::fs::canonicalize(&a).unwrap().to_string_lossy());
    }

    #[test]
    fn picker_start_trusts_only_detected_hints() {
        let t = td();
        let a = t.path().join("a");
        fake_client(&a);
        let ca = std::fs::canonicalize(&a).unwrap();
        let saved = Some("/salvata".to_string());
        // suggerimento rilevato: il selettore parte da lì
        assert_eq!(picker_start(a.to_str(), std::slice::from_ref(&ca), saved.clone()), Some(ca.clone()));
        // non rilevato o inesistente: cartella salvata, poi nessuna
        assert_eq!(picker_start(a.to_str(), &[], saved.clone()), Some(PathBuf::from("/salvata")));
        assert_eq!(picker_start(Some("/non/esiste"), std::slice::from_ref(&ca), None), None);
        assert_eq!(picker_start(None, std::slice::from_ref(&ca), None), None);
    }

    #[test]
    fn fake_lib_gives_translated_error_and_stays_retryable() {
        let t = td();
        let a = t.path().join("a");
        fake_client(&a);
        let e1 = ensure_initialized(&a).unwrap_err();
        let e2 = ensure_initialized(&a).unwrap_err();
        assert_eq!(e1, e2);
        assert!(!e1.contains("riavvia"));
        assert!(LOADED.lock().unwrap().is_none());
    }
}
