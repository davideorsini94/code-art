//! Albero file: lettura/scrittura sicura e watcher per progetto.
use crate::store::{Project, Store};
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, RecvTimeoutError};
use std::sync::Mutex;
use std::time::{Duration, UNIX_EPOCH};
use tauri::{AppHandle, Manager, State, WebviewWindow};

pub const MAX_READ_BYTES: u64 = 5 * 1024 * 1024;
/// Tetto per `fs_read_bytes` (anteprime di immagini, PDF, audio e video).
pub const MAX_BYTES_READ: u64 = 50 * 1024 * 1024;
const DEBOUNCE: Duration = Duration::from_millis(150);

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FsError {
    pub code: &'static str, // 'conflict' | 'too_large' | 'not_text' | 'io' | 'forbidden' | 'untrusted_folder'
    pub message: String,
}

impl FsError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self { code, message: message.into() }
    }
}

impl From<std::io::Error> for FsError {
    fn from(e: std::io::Error) -> Self {
        Self::new("io", e.to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DirEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub ignored: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileContent {
    pub text: String,
    pub mtime_ms: i64,
}

#[derive(Serialize, Clone)]
struct FsChanged {
    paths: Vec<String>,
}

// ---------- logica pura ----------

fn mtime_ms(meta: &std::fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Regole git valide per `dir`: i `.gitignore` dalla radice del repo (cartella con `.git`) fino a `dir`.
fn gitignore_for(dir: &Path) -> Option<Gitignore> {
    let mut chain: Vec<PathBuf> = Vec::new();
    let mut root = None;
    for a in dir.ancestors() {
        chain.push(a.to_path_buf());
        if a.join(".git").exists() {
            root = Some(a.to_path_buf());
            break;
        }
    }
    let root = root?;
    let mut b = GitignoreBuilder::new(&root);
    for d in chain.iter().rev() {
        let f = d.join(".gitignore");
        if f.is_file() {
            let _ = b.add(f);
        }
    }
    b.build().ok()
}

pub fn list(path: &Path) -> Result<Vec<DirEntry>, FsError> {
    let ig = gitignore_for(path);
    let mut out = Vec::new();
    for e in std::fs::read_dir(path)? {
        let e = e?;
        let name = e.file_name().to_string_lossy().to_string();
        if name == ".git" {
            continue;
        }
        let p = e.path();
        let is_dir = e.metadata().map(|m| m.is_dir()).unwrap_or(false) || p.is_dir();
        let ignored = ig.as_ref().is_some_and(|g| g.matched_path_or_any_parents(&p, is_dir).is_ignore());
        out.push(DirEntry { name, path: p.to_string_lossy().to_string(), is_dir, ignored });
    }
    out.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    Ok(out)
}

pub fn create(path: &Path, is_dir: bool) -> Result<(), FsError> {
    if is_dir {
        if path.exists() {
            return Err(FsError::new("io", "esiste già"));
        }
        std::fs::create_dir_all(path)?;
    } else {
        std::fs::OpenOptions::new().write(true).create_new(true).open(path)?;
    }
    Ok(())
}

pub fn rename(from: &Path, to: &Path) -> Result<(), FsError> {
    if to.exists() {
        return Err(FsError::new("io", "la destinazione esiste già"));
    }
    std::fs::rename(from, to)?;
    Ok(())
}

pub fn trash_path(path: &Path) -> Result<(), FsError> {
    crate::trasher::move_to_trash(path).map_err(|e| FsError::new("io", e))
}

pub fn read(path: &Path) -> Result<FileContent, FsError> {
    let meta = std::fs::metadata(path)?;
    if meta.len() > MAX_READ_BYTES {
        return Err(FsError::new("too_large", "file oltre 5 MB"));
    }
    let bytes = std::fs::read(path)?;
    let text = String::from_utf8(bytes).map_err(|_| FsError::new("not_text", "il file non è testo UTF-8"))?;
    Ok(FileContent { text, mtime_ms: mtime_ms(&meta) })
}

/// Legge i byte di un file regolare, al massimo `max` (clamp a `MAX_BYTES_READ`; anche se il file cresce durante la lettura).
/// Il tetto si decide dai metadati prima di leggere.
pub fn read_bytes(path: &Path, max: Option<u64>) -> Result<Vec<u8>, FsError> {
    use std::io::Read;
    let cap = max.unwrap_or(MAX_BYTES_READ).min(MAX_BYTES_READ);
    let meta = std::fs::metadata(path)?;
    if !meta.is_file() {
        return Err(FsError::new("io", "non è un file"));
    }
    if meta.len() > cap {
        return Err(FsError::new("too_large", "file oltre il limite"));
    }
    let mut buf = Vec::with_capacity(meta.len() as usize);
    std::fs::File::open(path)?.take(cap + 1).read_to_end(&mut buf)?;
    if buf.len() as u64 > cap {
        return Err(FsError::new("too_large", "file oltre il limite"));
    }
    Ok(buf)
}

/// Come `read_bytes`, ma solo dentro le radici date (stesso confinamento di `fs_read`).
pub fn read_bytes_confined(roots: &[PathBuf], path: &Path, max: Option<u64>) -> Result<Vec<u8>, FsError> {
    read_bytes(&ensure_inside(roots, path)?, max)
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileStat {
    pub size: u64,
    pub is_file: bool,
}

/// Dimensione e tipo (senza leggere il contenuto), solo dentro le radici date.
pub fn stat_confined(roots: &[PathBuf], path: &Path) -> Result<FileStat, FsError> {
    let meta = std::fs::metadata(ensure_inside(roots, path)?)?;
    Ok(FileStat { size: meta.len(), is_file: meta.is_file() })
}

/// Scrive solo se il file non è cambiato su disco rispetto a `expected` (se dato).
pub fn write(path: &Path, text: &str, expected: Option<i64>) -> Result<i64, FsError> {
    if let Some(exp) = expected {
        match std::fs::metadata(path) {
            Ok(meta) if mtime_ms(&meta) == exp => {}
            Ok(_) => return Err(FsError::new("conflict", "il file è cambiato su disco")),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(FsError::new("conflict", "il file non esiste più (rimosso o spostato)"));
            }
            Err(e) => return Err(e.into()),
        }
    }
    std::fs::write(path, text)?;
    Ok(mtime_ms(&std::fs::metadata(path)?))
}

/// Tetto di `fs_all_files`.
pub const ALL_FILES_MAX: usize = 50_000;

/// Tutti i file sotto le cartelle del progetto, come percorsi relativi (con il nome della cartella davanti solo
/// se le cartelle sono più d'una). Rispetta i `.gitignore`, salta `.git`, non segue i link simbolici.
pub fn all_files(folders: &[PathBuf]) -> Vec<String> {
    let multi = folders.len() > 1;
    let mut out: Vec<String> = Vec::new();
    for folder in folders {
        let prefix = if multi { folder.file_name().map(|n| format!("{}/", n.to_string_lossy())).unwrap_or_default() } else { String::new() };
        let walk = ignore::WalkBuilder::new(folder)
            .hidden(false)
            .follow_links(false)
            .require_git(false)
            .filter_entry(|e| {
                let n = e.file_name();
                let is_dir = e.file_type().is_some_and(|t| t.is_dir());
                !(is_dir && (n == ".git" || n == "node_modules" || n == "target"))
            })
            .build();
        for entry in walk.flatten() {
            if out.len() >= ALL_FILES_MAX {
                break;
            }
            if !entry.file_type().is_some_and(|t| t.is_file()) {
                continue;
            }
            if let Ok(rel) = entry.path().strip_prefix(folder) {
                out.push(format!("{prefix}{}", rel.to_string_lossy()));
            }
        }
    }
    out.sort();
    out
}

// ---------- watcher ----------

/// Percorsi che non vanno notificati: dentro `.git` (tranne `.git/HEAD`, serve al branch), `node_modules`, `target`.
/// Contano solo i componenti sotto la radice del progetto (una radice dentro `target/` non zittisce tutto).
/// Fuori da ogni radice: si notifica.
pub fn is_noise(roots: &[PathBuf], path: &Path) -> bool {
    let Some(rel) = roots.iter().find_map(|r| path.strip_prefix(r).ok()) else {
        return false;
    };
    let comps: Vec<_> = rel.components().map(|c| c.as_os_str().to_string_lossy()).collect();
    if comps.iter().any(|c| c == "node_modules" || c == "target") {
        return true;
    }
    match comps.iter().position(|c| c == ".git") {
        Some(i) => !(comps.len() == i + 2 && comps[i + 1] == "HEAD"),
        None => false,
    }
}

/// Un watcher per ogni progetto aperto (chiave: id progetto).
#[derive(Default)]
pub struct Watchers(Mutex<HashMap<String, RecommendedWatcher>>);

/// (Ri)avvia il watcher del progetto su tutte le sue cartelle; emette `fs:changed:<id progetto>` alla finestra del progetto.
pub fn watch_project(app: &AppHandle, project: &Project) {
    let (tx, rx) = channel::<PathBuf>();
    // radici come date e canonicalizzate: gli eventi possono arrivare con l'uno o l'altro percorso
    let roots: Vec<PathBuf> = project
        .folders
        .iter()
        .flat_map(|f| [Some(PathBuf::from(f)), Path::new(f).canonicalize().ok()])
        .flatten()
        .collect();
    let Ok(mut watcher) = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if let Ok(ev) = res {
            for p in ev.paths.into_iter().filter(|p| !is_noise(&roots, p)) {
                let _ = tx.send(p);
            }
        }
    }) else {
        return;
    };
    for f in &project.folders {
        let _ = watcher.watch(Path::new(f), RecursiveMode::Recursive);
    }
    let (handle, project_id) = (app.clone(), project.id.clone());
    std::thread::spawn(move || {
        // il thread termina quando il watcher (e quindi `tx`) viene distrutto
        while let Ok(first) = rx.recv() {
            let mut paths = vec![first];
            let deadline = std::time::Instant::now() + DEBOUNCE;
            loop {
                let left = deadline.saturating_duration_since(std::time::Instant::now());
                match rx.recv_timeout(left) {
                    Ok(p) => paths.push(p),
                    Err(RecvTimeoutError::Timeout) => break,
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            }
            let mut seen = HashSet::new();
            let paths: Vec<String> = paths
                .into_iter()
                .map(|p| p.to_string_lossy().to_string())
                .filter(|p| seen.insert(p.clone()))
                .collect();
            crate::projects::emit_to_project(&handle, &project_id, "fs:changed", FsChanged { paths });
        }
    });
    if let Ok(mut map) = app.state::<Watchers>().0.lock() {
        map.insert(project.id.clone(), watcher);
    }
}

pub fn unwatch_project(app: &AppHandle, project_id: &str) {
    if let Ok(mut map) = app.state::<Watchers>().0.lock() {
        map.remove(project_id);
    }
}

// ---------- confinamento ----------

/// Risolve `path` (anche se non esiste ancora) e lo accetta solo se sta dentro una delle `roots`.
/// Canonicalizza il più vicino antenato esistente (risolve `..` e symlink) e rimette in coda il resto,
/// che non può contenere `..`. Restituisce il percorso risolto, da usare per l'operazione.
pub fn ensure_inside(roots: &[PathBuf], path: &Path) -> Result<PathBuf, FsError> {
    let forbidden = || FsError::new("forbidden", "Percorso fuori dal progetto");
    if !path.is_absolute() {
        return Err(forbidden());
    }
    let mut rest: Vec<std::ffi::OsString> = Vec::new();
    let mut base = path;
    let canon = loop {
        match base.canonicalize() {
            Ok(c) => break c,
            Err(_) => {
                // un componente che esiste come symlink (pendente) non è "da creare": seguirlo uscirebbe dal controllo
                if std::fs::symlink_metadata(base).is_ok_and(|m| m.file_type().is_symlink()) {
                    return Err(forbidden());
                }
                let (Some(parent), Some(name)) = (base.parent(), base.file_name()) else {
                    return Err(forbidden()); // include `..` finale o radice inesistente
                };
                rest.push(name.to_os_string());
                base = parent;
            }
        }
    };
    let mut resolved = canon;
    for part in rest.iter().rev() {
        resolved.push(part);
    }
    let inside = roots.iter().filter_map(|r| r.canonicalize().ok()).any(|r| resolved.starts_with(&r));
    if inside { Ok(resolved) } else { Err(forbidden()) }
}

/// Cartelle del progetto della finestra con etichetta `label` (le finestre non di progetto non ne hanno).
/// È il passo etichetta → radici di ogni comando su file.
pub fn roots_for_label(store: &Store, label: &str) -> Result<Vec<PathBuf>, FsError> {
    let id = crate::projects::project_id_of_label(label).ok_or_else(|| FsError::new("forbidden", "Percorso fuori dal progetto"))?;
    let project = store
        .get_project(id)
        .map_err(|e| FsError::new("io", e))?
        .ok_or_else(|| FsError::new("io", "progetto inesistente"))?;
    Ok(project.folders.iter().map(PathBuf::from).collect())
}

/// Come `roots_for_label`, per i comandi che cambiano il disco: un progetto con anche una sola cartella non
/// fidata è in sola lettura (errore `untrusted_folder`). Scrivere lì potrebbe piantare file eseguiti altrove
/// (hook di `.git` o di `.claude` in una cartella fidata annidata, `~/.zshrc` letto dai terminali).
pub fn writable_roots_for_label(store: &Store, label: &str) -> Result<Vec<PathBuf>, FsError> {
    let roots = roots_for_label(store, label)?;
    let id = crate::projects::project_id_of_label(label).unwrap_or_default();
    crate::trust::ensure_project_trusted(store, id)
        .map_err(|_| FsError::new(crate::trust::UNTRUSTED, "Cartella non fidata: sola lettura"))?;
    Ok(roots)
}

/// Cartelle del progetto della finestra chiamante (dall'etichetta, mai da un argomento).
pub(crate) fn roots_of(window: &WebviewWindow, store: &Store) -> Result<Vec<PathBuf>, FsError> {
    roots_for_label(store, window.label())
}

/// Percorso da leggere per la finestra `label`: dentro il suo progetto.
pub fn readable_path(store: &Store, label: &str, path: &str) -> Result<PathBuf, FsError> {
    ensure_inside(&roots_for_label(store, label)?, Path::new(path))
}

/// Percorso da modificare per la finestra `label`: dentro il suo progetto, e progetto fidato.
pub fn mutable_path(store: &Store, label: &str, path: &str) -> Result<PathBuf, FsError> {
    ensure_inside(&writable_roots_for_label(store, label)?, Path::new(path))
}

/// Le cartelle del progetto non si rinominano né si spostano nel Cestino dall'albero (`forbidden`).
pub fn refuse_root(roots: &[PathBuf], resolved: &Path) -> Result<(), FsError> {
    if roots.iter().filter_map(|r| r.canonicalize().ok()).any(|r| r == resolved) {
        return Err(FsError::new("forbidden", "Cartella del progetto: non si può rinominare né spostare nel Cestino"));
    }
    Ok(())
}

/// Percorso da rinominare/spostare o mettere nel Cestino: come `mutable_path`, ma mai una cartella del progetto.
pub fn movable_path(store: &Store, label: &str, path: &str) -> Result<PathBuf, FsError> {
    let roots = writable_roots_for_label(store, label)?;
    let resolved = ensure_inside(&roots, Path::new(path))?;
    refuse_root(&roots, &resolved)?;
    Ok(resolved)
}

fn checked(window: &WebviewWindow, store: &Store, path: &str) -> Result<PathBuf, FsError> {
    readable_path(store, window.label(), path)
}

fn checked_mut(window: &WebviewWindow, store: &Store, path: &str) -> Result<PathBuf, FsError> {
    mutable_path(store, window.label(), path)
}

// ---------- comandi ----------

#[tauri::command]
pub async fn fs_list(window: WebviewWindow, store: State<'_, Store>, path: String) -> Result<Vec<DirEntry>, FsError> {
    list(&checked(&window, &store, &path)?)
}

#[tauri::command]
pub async fn fs_create(window: WebviewWindow, store: State<'_, Store>, path: String, is_dir: bool) -> Result<(), FsError> {
    create(&checked_mut(&window, &store, &path)?, is_dir)
}

#[tauri::command]
pub async fn fs_rename(window: WebviewWindow, store: State<'_, Store>, from: String, to: String) -> Result<(), FsError> {
    let (from, to) = (movable_path(&store, window.label(), &from)?, checked_mut(&window, &store, &to)?);
    rename(&from, &to)
}

#[tauri::command]
pub async fn fs_trash(window: WebviewWindow, store: State<'_, Store>, path: String) -> Result<(), FsError> {
    trash_path(&movable_path(&store, window.label(), &path)?)
}

#[tauri::command]
pub async fn fs_read(window: WebviewWindow, store: State<'_, Store>, path: String) -> Result<FileContent, FsError> {
    read(&checked(&window, &store, &path)?)
}

/// Byte grezzi di un file del progetto (anteprime). Lettura: ammessa anche nei progetti non fidati.
/// Risposta binaria dell'IPC, non un array JSON; lettura fuori dal thread principale.
#[tauri::command]
pub async fn fs_read_bytes(
    window: WebviewWindow,
    store: State<'_, Store>,
    path: String,
    max_bytes: Option<u64>,
) -> Result<tauri::ipc::Response, FsError> {
    let path = checked(&window, &store, &path)?;
    let bytes = tauri::async_runtime::spawn_blocking(move || read_bytes(&path, max_bytes)).await.map_err(|e| FsError::new("io", e.to_string()))??;
    Ok(tauri::ipc::Response::new(bytes))
}

/// Dimensione di un file del progetto, senza leggerlo (lettura: ammessa anche nei progetti non fidati).
#[tauri::command]
pub async fn fs_stat(window: WebviewWindow, store: State<'_, Store>, path: String) -> Result<FileStat, FsError> {
    let roots = roots_of(&window, &store)?;
    let path = PathBuf::from(path);
    tauri::async_runtime::spawn_blocking(move || stat_confined(&roots, &path)).await.map_err(|e| FsError::new("io", e.to_string()))?
}

#[tauri::command]
pub async fn fs_write(
    window: WebviewWindow,
    store: State<'_, Store>,
    path: String,
    text: String,
    expected_mtime_ms: Option<i64>,
) -> Result<i64, FsError> {
    write(&checked_mut(&window, &store, &path)?, &text, expected_mtime_ms)
}

/// Elenco piatto dei file del progetto della finestra chiamante (per l'apertura rapida).
#[tauri::command]
pub async fn fs_all_files(window: WebviewWindow, store: State<'_, Store>) -> Result<Vec<String>, FsError> {
    let roots = roots_of(&window, &store)?;
    tauri::async_runtime::spawn_blocking(move || all_files(&roots)).await.map_err(|e| FsError::new("io", e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_sorts_dirs_first_case_insensitive() {
        let d = tempfile::tempdir().unwrap();
        for f in ["b.txt", "A.txt", "c.txt"] {
            std::fs::write(d.path().join(f), "").unwrap();
        }
        for n in ["Zeta", "alpha"] {
            std::fs::create_dir(d.path().join(n)).unwrap();
        }
        std::fs::create_dir(d.path().join(".git")).unwrap();
        let names: Vec<_> = list(d.path()).unwrap().into_iter().map(|e| e.name).collect();
        assert_eq!(names, vec!["alpha", "Zeta", "A.txt", "b.txt", "c.txt"]);
    }

    #[test]
    fn list_marks_gitignored() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir(d.path().join(".git")).unwrap();
        std::fs::write(d.path().join(".gitignore"), "target/\n*.log\n").unwrap();
        std::fs::create_dir(d.path().join("target")).unwrap();
        std::fs::write(d.path().join("x.log"), "").unwrap();
        std::fs::write(d.path().join("main.rs"), "").unwrap();
        let by: HashMap<_, _> = list(d.path()).unwrap().into_iter().map(|e| (e.name, e.ignored)).collect();
        assert!(by["target"]);
        assert!(by["x.log"]);
        assert!(!by["main.rs"]);
    }

    #[test]
    fn list_marks_gitignored_in_subfolder() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join(".git")).unwrap();
        std::fs::create_dir_all(d.path().join("target/debug")).unwrap();
        std::fs::write(d.path().join(".gitignore"), "target/\n").unwrap();
        std::fs::write(d.path().join("target/debug/a"), "").unwrap();
        let e = list(&d.path().join("target/debug")).unwrap();
        assert!(e[0].ignored);
    }

    #[test]
    fn write_with_stale_mtime_is_conflict() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("a.txt");
        std::fs::write(&f, "uno").unwrap();
        let m = read(&f).unwrap().mtime_ms;
        let err = write(&f, "due", Some(m - 5000)).unwrap_err();
        assert_eq!(err.code, "conflict");
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "uno");
        let new = write(&f, "due", Some(m)).unwrap();
        assert_eq!(read(&f).unwrap().mtime_ms, new);
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "due");
    }

    #[test]
    fn write_with_expected_mtime_on_missing_file_is_conflict() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("gone.txt");
        assert_eq!(write(&f, "x", Some(1)).unwrap_err().code, "conflict");
        assert!(!f.exists());
        write(&f, "x", None).unwrap(); // nuovo file senza mtime atteso: ok
        assert!(f.exists());
    }

    fn roots(d: &Path) -> Vec<PathBuf> {
        let r = d.join("proj");
        std::fs::create_dir_all(&r).unwrap();
        vec![r]
    }

    #[test]
    fn ensure_inside_accepts_existing_and_new_paths() {
        let d = tempfile::tempdir().unwrap();
        let r = roots(d.path());
        std::fs::write(r[0].join("a.txt"), "").unwrap();
        assert!(ensure_inside(&r, &r[0].join("a.txt")).is_ok());
        assert!(ensure_inside(&r, &r[0]).is_ok());
        assert!(ensure_inside(&r, &r[0].join("nuova/cartella/file.txt")).is_ok());
    }

    #[test]
    fn ensure_inside_rejects_outside_and_dotdot() {
        let d = tempfile::tempdir().unwrap();
        let r = roots(d.path());
        std::fs::write(d.path().join("outside.txt"), "").unwrap();
        assert_eq!(ensure_inside(&r, &d.path().join("outside.txt")).unwrap_err().code, "forbidden");
        assert_eq!(ensure_inside(&r, &r[0].join("../outside.txt")).unwrap_err().code, "forbidden");
        assert_eq!(ensure_inside(&r, &r[0].join("nuova/../../outside.txt")).unwrap_err().code, "forbidden");
        assert_eq!(ensure_inside(&r, Path::new("relativo/file")).unwrap_err().code, "forbidden");
    }

    #[test]
    fn ensure_inside_rejects_symlink_leading_outside() {
        let d = tempfile::tempdir().unwrap();
        let r = roots(d.path());
        let out = d.path().join("outside");
        std::fs::create_dir(&out).unwrap();
        std::fs::write(out.join("s.txt"), "").unwrap();
        std::os::unix::fs::symlink(&out, r[0].join("link")).unwrap();
        assert_eq!(ensure_inside(&r, &r[0].join("link")).unwrap_err().code, "forbidden");
        assert_eq!(ensure_inside(&r, &r[0].join("link/s.txt")).unwrap_err().code, "forbidden");
        assert_eq!(ensure_inside(&r, &r[0].join("link/nuovo.txt")).unwrap_err().code, "forbidden");
    }

    #[test]
    fn ensure_inside_rejects_sibling_sharing_prefix() {
        let d = tempfile::tempdir().unwrap();
        let r = roots(d.path());
        let sib = d.path().join("proj-old");
        std::fs::create_dir(&sib).unwrap();
        assert_eq!(ensure_inside(&r, &sib).unwrap_err().code, "forbidden");
        assert_eq!(ensure_inside(&r, &sib.join("x.txt")).unwrap_err().code, "forbidden");
    }

    #[test]
    fn project_roots_cannot_be_moved_or_trashed() {
        let d = tempfile::tempdir().unwrap();
        let r = roots(d.path());
        std::fs::create_dir(r[0].join("sub")).unwrap();
        let canon = r[0].canonicalize().unwrap();
        assert_eq!(refuse_root(&r, &canon).unwrap_err().code, "forbidden");
        assert!(refuse_root(&r, &canon.join("sub")).is_ok());
        // con `/` finale o `.` il percorso risolto è sempre la radice
        let resolved = ensure_inside(&r, &r[0].join("sub/..")).unwrap_or(canon.clone());
        assert_eq!(refuse_root(&r, &resolved).unwrap_err().code, "forbidden");
    }

    #[test]
    fn noise_filter() {
        let roots = vec![PathBuf::from("/p")];
        let n = |p: &str| is_noise(&roots, Path::new(p));
        assert!(n("/p/.git/index.lock"));
        assert!(n("/p/.git/refs/heads/x"));
        assert!(!n("/p/.git/HEAD"));
        assert!(n("/p/node_modules/a/b.js"));
        assert!(n("/p/target/debug/x"));
        assert!(!n("/p/src/main.rs"));
        assert!(!n("/p/.gitignore"));
    }

    #[test]
    fn noise_filter_is_relative_to_root() {
        let roots = vec![PathBuf::from("/tmp/x/target/myproj")];
        assert!(!is_noise(&roots, Path::new("/tmp/x/target/myproj/src/a.rs")));
        assert!(is_noise(&roots, Path::new("/tmp/x/target/myproj/target/debug/a")));
        assert!(is_noise(&roots, Path::new("/tmp/x/target/myproj/.git/index.lock")));
    }

    #[test]
    fn dangling_symlink_to_outside_is_forbidden() {
        let d = tempfile::tempdir().unwrap();
        let r = roots(d.path());
        let outside = d.path().join("outside-new.txt");
        std::os::unix::fs::symlink(&outside, r[0].join("link")).unwrap();
        assert_eq!(ensure_inside(&r, &r[0].join("link")).unwrap_err().code, "forbidden");
        assert!(!outside.exists());
        // anche un link pendente a metà percorso
        let outdir = d.path().join("outdir");
        std::os::unix::fs::symlink(&outdir, r[0].join("dir")).unwrap();
        assert_eq!(ensure_inside(&r, &r[0].join("dir/f.txt")).unwrap_err().code, "forbidden");
        assert!(!outdir.exists());
    }

    #[test]
    fn read_over_5mb_is_too_large() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("big.txt");
        std::fs::write(&f, vec![b'a'; (MAX_READ_BYTES + 1) as usize]).unwrap();
        assert_eq!(read(&f).unwrap_err().code, "too_large");
    }

    #[test]
    fn read_bytes_is_confined_and_capped() {
        let d = tempfile::tempdir().unwrap();
        let r = roots(d.path());
        std::fs::write(r[0].join("a.bin"), [0xff, 0x00, 0x80]).unwrap();
        assert_eq!(read_bytes_confined(&r, &r[0].join("a.bin"), None).unwrap(), vec![0xff, 0x00, 0x80]);
        std::fs::write(d.path().join("out.bin"), [1]).unwrap();
        assert_eq!(read_bytes_confined(&r, &d.path().join("out.bin"), None).unwrap_err().code, "forbidden");
        std::os::unix::fs::symlink(d.path().join("out.bin"), r[0].join("link.bin")).unwrap();
        assert_eq!(read_bytes_confined(&r, &r[0].join("link.bin"), None).unwrap_err().code, "forbidden");
        let big = std::fs::File::create(r[0].join("big.bin")).unwrap();
        big.set_len(MAX_BYTES_READ + 1).unwrap();
        assert_eq!(read_bytes_confined(&r, &r[0].join("big.bin"), None).unwrap_err().code, "too_large");
        assert_eq!(read_bytes_confined(&r, &r[0], None).unwrap_err().code, "io");
        // tetto per chiamata deciso dai metadati; mai oltre il tetto assoluto
        std::fs::write(r[0].join("m.bin"), vec![0u8; 100]).unwrap();
        assert_eq!(read_bytes_confined(&r, &r[0].join("m.bin"), Some(100)).unwrap().len(), 100);
        assert_eq!(read_bytes_confined(&r, &r[0].join("m.bin"), Some(99)).unwrap_err().code, "too_large");
        assert_eq!(read_bytes_confined(&r, &r[0].join("big.bin"), Some(u64::MAX)).unwrap_err().code, "too_large");
    }

    #[test]
    fn stat_is_confined_and_reports_size() {
        let d = tempfile::tempdir().unwrap();
        let r = roots(d.path());
        std::fs::write(r[0].join("a.bin"), [1, 2, 3, 4]).unwrap();
        assert_eq!(stat_confined(&r, &r[0].join("a.bin")).unwrap(), FileStat { size: 4, is_file: true });
        assert!(!stat_confined(&r, &r[0]).unwrap().is_file);
        std::fs::write(d.path().join("out.bin"), [1]).unwrap();
        assert_eq!(stat_confined(&r, &d.path().join("out.bin")).unwrap_err().code, "forbidden");
        std::os::unix::fs::symlink(d.path().join("out.bin"), r[0].join("link.bin")).unwrap();
        assert_eq!(stat_confined(&r, &r[0].join("link.bin")).unwrap_err().code, "forbidden");
        assert_eq!(stat_confined(&r, &r[0].join("assente")).unwrap_err().code, "io");
    }

    #[test]
    fn read_invalid_utf8_is_not_text() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("b.bin");
        std::fs::write(&f, [0xff, 0xfe, 0x00, 0x80]).unwrap();
        assert_eq!(read(&f).unwrap_err().code, "not_text");
    }

    #[test]
    fn trash_removes_path() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("gone-code-art-test.txt");
        std::fs::write(&f, "x").unwrap();
        trash_path(&f).unwrap();
        assert!(!f.exists());
    }

    #[test]
    fn create_and_rename() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("n.txt");
        create(&f, false).unwrap();
        assert_eq!(create(&f, false).unwrap_err().code, "io");
        let g = d.path().join("m.txt");
        rename(&f, &g).unwrap();
        assert!(g.exists() && !f.exists());
        create(&d.path().join("x/y"), true).unwrap();
    }

    #[test]
    fn error_serializes_code_and_message() {
        let v = serde_json::to_value(FsError::new("conflict", "m")).unwrap();
        assert_eq!(v, serde_json::json!({ "code": "conflict", "message": "m" }));
    }

    #[test]
    fn fs_all_files_skips_ignored() {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        std::fs::create_dir_all(r.join(".git")).unwrap();
        std::fs::write(r.join(".git/config"), "").unwrap();
        std::fs::write(r.join(".gitignore"), "target/\n*.log\n").unwrap();
        std::fs::create_dir_all(r.join("target")).unwrap();
        std::fs::write(r.join("target/out.bin"), "").unwrap();
        std::fs::write(r.join("x.log"), "").unwrap();
        std::fs::create_dir_all(r.join("src")).unwrap();
        std::fs::write(r.join("src/main.rs"), "").unwrap();
        std::fs::write(r.join(".env"), "").unwrap();
        assert_eq!(all_files(&[r.to_path_buf()]), vec![".env", ".gitignore", "src/main.rs"]);
    }

    #[test]
    fn fs_all_files_does_not_follow_symlinks_and_prefixes_multi_folders() {
        let d = tempfile::tempdir().unwrap();
        let outside = d.path().join("outside");
        let a = d.path().join("a");
        let b = d.path().join("b");
        for p in [&outside, &a, &b] {
            std::fs::create_dir_all(p).unwrap();
        }
        std::fs::write(outside.join("secret.txt"), "").unwrap();
        std::os::unix::fs::symlink(&outside, a.join("link")).unwrap();
        std::os::unix::fs::symlink(outside.join("secret.txt"), a.join("s.txt")).unwrap();
        std::fs::write(a.join("one.txt"), "").unwrap();
        std::fs::write(b.join("two.txt"), "").unwrap();
        assert_eq!(all_files(&[a.clone()]), vec!["one.txt"]);
        assert_eq!(all_files(&[a, b]), vec!["a/one.txt", "b/two.txt"]);
    }

    #[test]
    fn fs_all_files_honours_gitignore_without_git_and_skips_heavy_dirs() {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        std::fs::write(r.join(".gitignore"), "*.log\n").unwrap();
        std::fs::write(r.join("x.log"), "").unwrap();
        std::fs::write(r.join("a.txt"), "").unwrap();
        for dir in ["node_modules/pkg", "target/debug", "sub/node_modules"] {
            std::fs::create_dir_all(r.join(dir)).unwrap();
            std::fs::write(r.join(dir).join("f"), "").unwrap();
        }
        assert_eq!(all_files(&[r.to_path_buf()]), vec![".gitignore", "a.txt"]);
    }
}
