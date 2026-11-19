//! Trasferimenti di file e cartelle da e verso un server SSH, con avanzamento e annullamento.
//!
//! I byte passano da `cat` sul server (stdin o stdout del processo `ssh`, sulla connessione condivisa), a pezzi di
//! 64 KB, così l'app li conta e può fermarli. Ogni file è scritto come `<nome>.codeart-part` e rinominato solo alla
//! fine, mai sopra una destinazione che esiste. Le cartelle si elencano prima (si conosce il totale) e si creano a
//! destinazione una per una. Uno script è sempre una costante; i percorsi arrivano solo come argomenti (`$1`, `$2`).
//!
//! Il trasporto è iniettabile (`Transport`): nell'app avvia `/usr/bin/ssh`, nei test `/bin/sh -c <script> _ <args>`,
//! con gli stessi script e lo stesso percorso di streaming.
use super::fs::{self as rfs, validate_path, RENAME_SCRIPT};
use crate::claude::locate::{in_new_session, kill_group};
use crate::tools::{self, ToolOutput};
use serde::Serialize;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const CHUNK: usize = 64 * 1024;
/// Al massimo 10 eventi `running` al secondo.
pub const EVENT_GAP: Duration = Duration::from_millis(100);
const PART_SUFFIX: &str = ".codeart-part";
/// Voci al massimo in un trasferimento (file e cartelle).
const MAX_ITEMS: usize = 100_000;
#[cfg(test)]
const QUICK: Duration = Duration::from_secs(60);
const STDERR_MAX: usize = 8 * 1024;
pub const EXISTS: &str = "esiste già";
pub const LOST: &str = "Connessione persa";

const EXIT_EXISTS: i32 = 4;
const EXIT_NOT_FOUND: i32 = 6;

/// `$1` = file temporaneo; mai sopra qualcosa che esiste (anche un link); i byte arrivano da stdin.
pub const UPLOAD_SCRIPT: &str = "if [ -e \"$1\" ] || [ -L \"$1\" ]; then exit 4; fi\nset -C\ncat > \"$1\"";
pub const DOWNLOAD_SCRIPT: &str = "cat -- \"$1\"";
/// Esce con 4 se `$1` esiste (anche un link rotto).
pub const EXISTS_SCRIPT: &str = "if [ -e \"$1\" ] || [ -L \"$1\" ]; then exit 4; fi";
/// Come `mkdir --` ma una destinazione che esiste (anche un link) è un conflitto (4), non un errore generico.
pub const MKDIR_NEW_SCRIPT: &str = "if [ -e \"$1\" ] || [ -L \"$1\" ]; then exit 4; fi\nmkdir -- \"$1\"";
pub const REMOVE_SCRIPT: &str = "rm -f -- \"$1\"";
/// Albero di `$1` in un solo giro (un percorso per record, `tipo\tbyte\tpercorso\0`, cartelle prima del contenuto):
/// `d` cartella, `f` file, `l` link (non si seguono), `o` altro. Una chiamata sola invece di una per voce.
pub const WALK_SCRIPT: &str = "walk() { for f in \"$1\"/* \"$1\"/.[!.]* \"$1\"/..?*; do\n\
  [ -e \"$f\" ] || [ -L \"$f\" ] || continue\n\
  if [ -L \"$f\" ]; then printf 'l\\t0\\t%s\\0' \"$f\"\n\
  elif [ -d \"$f\" ]; then printf 'd\\t0\\t%s\\0' \"$f\"; walk \"$f\"\n\
  elif [ -f \"$f\" ]; then s=$({ wc -c < \"$f\"; } 2>/dev/null | tr -d ' '); printf 'f\\t%s\\t%s\\0' \"${s:-0}\" \"$f\"\n\
  else printf 'o\\t0\\t%s\\0' \"$f\"; fi\n\
done; }\n\
if [ -L \"$1\" ]; then printf 'l\\t0\\t%s\\0' \"$1\"\n\
elif [ -d \"$1\" ]; then printf 'd\\t0\\t%s\\0' \"$1\"; walk \"$1\"\n\
elif [ -f \"$1\" ]; then s=$({ wc -c < \"$1\"; } 2>/dev/null | tr -d ' '); printf 'f\\t%s\\t%s\\0' \"${s:-0}\" \"$1\"\n\
elif [ -e \"$1\" ]; then printf 'o\\t0\\t%s\\0' \"$1\"\n\
else exit 6; fi\n";

// ---------- eventi ----------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TransferState {
    Running,
    Done,
    Cancelled,
    Error,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TransferError {
    pub path: String,
    pub message: String,
}

/// Evento `ssh:transfer`: l'ultimo (stato diverso da `running`) porta l'elenco degli errori e un messaggio.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferEvent {
    pub transfer_id: String,
    pub done: u64,
    pub total: u64,
    pub file: String,
    pub state: TransferState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub errors: Option<Vec<TransferError>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

pub type Emitter = Arc<dyn Fn(TransferEvent) + Send + Sync>;

// ---------- trasporto ----------

/// Come eseguire uno script sul server.
pub trait Transport: Send + Sync {
    /// Comando per uno script di lunga durata (stdio da impostare a chi chiama).
    fn command(&self, script: &str, args: &[&str]) -> Result<Command, String>;
    /// Script breve con output in memoria.
    fn run(&self, script: &str, args: &[&str], stdin: Option<Vec<u8>>) -> Result<ToolOutput, String>;
}

/// Il trasporto vero: la connessione condivisa del server.
pub struct SshTransport {
    pub conns: Arc<super::conn::Connections>,
    pub server: super::Server,
}

impl Transport for SshTransport {
    fn command(&self, script: &str, args: &[&str]) -> Result<Command, String> {
        self.conns.stream_command(&self.server, script, args)
    }
    fn run(&self, script: &str, args: &[&str], stdin: Option<Vec<u8>>) -> Result<ToolOutput, String> {
        self.conns.run(&self.server, script, args, stdin)
    }
}

// ---------- richiesta ----------

/// Percorsi già validati dal chiamante: locali dentro il progetto, remoti assoluti (con `~` risolta).
#[derive(Debug, Clone)]
pub enum Request {
    Upload { sources: Vec<PathBuf>, remote_dir: String },
    Download { sources: Vec<String>, local_dir: PathBuf },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Dir,
    File,
}

struct Item {
    kind: Kind,
    /// Percorso relativo alla destinazione (inizia col nome della sorgente): per l'elenco degli errori.
    rel: String,
    src: String,
    dst: String,
    size: u64,
    /// Sorgente indicata dall'utente: la sua destinazione si controlla subito (conflitto).
    top: bool,
}

enum ItemErr {
    Cancelled,
    Failed(String),
}

type ItemResult = Result<(), ItemErr>;

fn failed(m: impl Into<String>) -> ItemErr {
    ItemErr::Failed(m.into())
}

/// Testo per l'interfaccia: i caratteri di controllo diventano `?`.
fn clean(s: &str) -> String {
    s.chars().map(|c| if c.is_control() { '?' } else { c }).collect()
}

fn join(dir: &str, name: &str) -> String {
    if dir.ends_with('/') { format!("{dir}{name}") } else { format!("{dir}/{name}") }
}

/// Primo rigo non vuoto dell'errore di un processo, per un messaggio breve.
fn first_line(stderr: &str) -> String {
    clean(stderr.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or(""))
}

fn script_failure(status: i32, stderr: &str) -> String {
    let l = first_line(stderr);
    match status {
        EXIT_EXISTS => EXISTS.into(),
        255 if l.is_empty() => LOST.into(),
        _ if l.is_empty() => format!("errore del server (codice {status})"),
        _ => l,
    }
}

// ---------- avanzamento ----------

/// Conta i byte e inoltra gli eventi `running` al massimo uno ogni `gap`.
struct Progress<'a> {
    id: &'a str,
    emit: &'a dyn Fn(TransferEvent),
    gap: Duration,
    last: Option<Instant>,
    done: u64,
    total: u64,
    file: String,
}

impl Progress<'_> {
    fn event(&self, state: TransferState) -> TransferEvent {
        TransferEvent {
            transfer_id: self.id.to_string(),
            done: self.done,
            total: self.total,
            file: self.file.clone(),
            state,
            errors: None,
            message: None,
        }
    }

    fn tick(&mut self, force: bool) {
        let now = Instant::now();
        if force || self.last.is_none_or(|l| now.duration_since(l) >= self.gap) {
            self.last = Some(now);
            (self.emit)(self.event(TransferState::Running));
        }
    }

    fn start_file(&mut self, rel: &str) {
        self.file = clean(rel);
        self.tick(false);
    }

    fn add(&mut self, n: u64) {
        self.done += n;
        self.total = self.total.max(self.done);
        self.tick(false);
    }

    fn finish(&self, state: TransferState, errors: Vec<TransferError>, message: String) {
        let mut e = self.event(state);
        e.file = String::new();
        e.errors = Some(errors);
        e.message = Some(message);
        (self.emit)(e);
    }
}

// ---------- processo in streaming ----------

/// Un processo avviato per uno script, con il suo gruppo: si ferma da solo se il trasferimento è annullato o l'app
/// si chiude (registro di `tools`).
struct Stream {
    child: Child,
    /// `false` dopo che il chiamante ha preso il processo per raccoglierlo: il guardiano non spara più (pid riusato).
    alive: Arc<Mutex<bool>>,
    stderr: Option<std::thread::JoinHandle<String>>,
    _ticket: tools::Ticket<'static>,
}

fn spawn_stream(t: &dyn Transport, script: &str, args: &[&str], piped_in: bool, cancel: &Arc<AtomicBool>) -> Result<Stream, String> {
    let ticket = tools::RUNNING.register()?;
    let mut cmd = t.command(script, args)?;
    in_new_session(&mut cmd);
    cmd.stdin(if piped_in { Stdio::piped() } else { Stdio::null() }).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| format!("avvio non riuscito: {e}"))?;
    let alive = Arc::new(Mutex::new(true));
    let pid = child.id();
    let (a, c, s) = (alive.clone(), cancel.clone(), ticket.stop.clone());
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(Duration::from_millis(20));
            let live = crate::lock(&a);
            if !*live {
                return;
            }
            if c.load(Ordering::SeqCst) || s.load(Ordering::SeqCst) {
                kill_group(pid);
                return;
            }
        }
    });
    let stderr = child.stderr.take().map(|mut e| {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let mut chunk = [0u8; 4096];
            while let Ok(n) = e.read(&mut chunk) {
                if n == 0 {
                    break;
                }
                if buf.len() < STDERR_MAX {
                    buf.extend_from_slice(&chunk[..n.min(STDERR_MAX - buf.len())]);
                }
            }
            String::from_utf8_lossy(&buf).into_owned()
        })
    });
    Ok(Stream { child, alive, stderr, _ticket: ticket })
}

impl Stream {
    /// Chiude stdin, aspetta il processo; `(codice, stderr)`; `-1` = ucciso da un segnale.
    fn finish(mut self) -> (i32, String) {
        drop(self.child.stdin.take());
        *crate::lock(&self.alive) = false;
        let status = self.child.wait().ok().and_then(|s| s.code()).unwrap_or(-1);
        let err = self.stderr.take().and_then(|h| h.join().ok()).unwrap_or_default();
        (status, err)
    }
}

// ---------- il lavoro ----------

struct Ctx<'a> {
    t: &'a dyn Transport,
    cancel: &'a Arc<AtomicBool>,
    p: Progress<'a>,
}

impl Ctx<'_> {
    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::SeqCst)
    }

    fn run(&self, script: &str, args: &[&str]) -> Result<ToolOutput, String> {
        self.t.run(script, args, None)
    }

    /// Script breve che deve riuscire; altrimenti il messaggio.
    fn run_ok(&self, script: &str, args: &[&str]) -> Result<(), String> {
        let out = self.run(script, args)?;
        if out.status == 0 { Ok(()) } else { Err(script_failure(out.status, &out.stderr)) }
    }

    fn remove_remote(&self, path: &str) {
        let _ = self.run(REMOVE_SCRIPT, &[path]);
    }
}

fn part_of(dst: &str) -> String {
    format!("{dst}{PART_SUFFIX}")
}

fn upload_file(c: &mut Ctx, item: &Item) -> ItemResult {
    let part = part_of(&item.dst);
    if item.top {
        let out = c.run(EXISTS_SCRIPT, &[&item.dst]).map_err(failed)?;
        if out.status != 0 {
            return Err(failed(script_failure(out.status, &out.stderr)));
        }
    }
    let mut file = open_nofollow(Path::new(&item.src)).map_err(|e| failed(format!("lettura non riuscita: {e}")))?;
    if !file.metadata().is_ok_and(|m| m.is_file()) {
        return Err(failed("non è un file normale"));
    }
    c.p.start_file(&item.rel);
    let mut s = spawn_stream(c.t, UPLOAD_SCRIPT, &[&part], true, c.cancel).map_err(failed)?;
    let mut stdin = s.child.stdin.take();
    let mut buf = vec![0u8; CHUNK];
    let mut io_err: Option<String> = None;
    loop {
        if c.cancelled() {
            break;
        }
        let n = match file.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) => {
                io_err = Some(format!("lettura non riuscita: {e}"));
                break;
            }
        };
        let Some(w) = stdin.as_mut() else { break };
        if w.write_all(&buf[..n]).is_err() {
            break; // il server ha chiuso: l'esito lo dice lo stato di uscita
        }
        c.p.add(n as u64);
    }
    drop(stdin);
    let (status, stderr) = s.finish();
    if c.cancelled() {
        c.remove_remote(&part);
        return Err(ItemErr::Cancelled);
    }
    if status == EXIT_EXISTS {
        return Err(failed(format!("{EXISTS}: il file temporaneo")));
    }
    if status != 0 || io_err.is_some() {
        c.remove_remote(&part);
        return Err(failed(io_err.unwrap_or_else(|| script_failure(status, &stderr))));
    }
    let out = c.run(RENAME_SCRIPT, &[&part, &item.dst]);
    match out {
        Ok(o) if o.status == 0 => Ok(()),
        Ok(o) => {
            c.remove_remote(&part);
            Err(failed(script_failure(o.status, &o.stderr)))
        }
        Err(e) => {
            c.remove_remote(&part);
            Err(failed(e))
        }
    }
}

fn open_nofollow(p: &Path) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW).open(p)
}

fn exists_local(p: &Path) -> bool {
    std::fs::symlink_metadata(p).is_ok()
}

fn download_file(c: &mut Ctx, item: &Item) -> ItemResult {
    let dst = Path::new(&item.dst);
    let part = PathBuf::from(part_of(&item.dst));
    if item.top && exists_local(dst) {
        return Err(failed(EXISTS));
    }
    let mut out = match std::fs::OpenOptions::new().write(true).create_new(true).open(&part) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => return Err(failed(format!("{EXISTS}: il file temporaneo"))),
        Err(e) => return Err(failed(format!("scrittura non riuscita: {e}"))),
    };
    c.p.start_file(&item.rel);
    let mut s = match spawn_stream(c.t, DOWNLOAD_SCRIPT, &[&item.src], false, c.cancel) {
        Ok(s) => s,
        Err(e) => {
            let _ = std::fs::remove_file(&part);
            return Err(failed(e));
        }
    };
    let mut stdout = s.child.stdout.take();
    let mut buf = vec![0u8; CHUNK];
    let mut io_err: Option<String> = None;
    loop {
        if c.cancelled() {
            break;
        }
        let n = match stdout.as_mut().map(|o| o.read(&mut buf)) {
            Some(Ok(0)) | None => break,
            Some(Ok(n)) => n,
            Some(Err(e)) => {
                io_err = Some(format!("lettura non riuscita: {e}"));
                break;
            }
        };
        if let Err(e) = out.write_all(&buf[..n]) {
            io_err = Some(format!("scrittura non riuscita: {e}"));
            break;
        }
        c.p.add(n as u64);
    }
    if io_err.is_some() || c.cancelled() {
        // niente più lettori sulla pipe: il processo si ferma (il guardiano lo uccide se annullato)
        drop(stdout);
    }
    let (status, stderr) = s.finish();
    drop(out);
    if c.cancelled() {
        let _ = std::fs::remove_file(&part);
        return Err(ItemErr::Cancelled);
    }
    if status != 0 || io_err.is_some() {
        let _ = std::fs::remove_file(&part);
        return Err(failed(io_err.unwrap_or_else(|| script_failure(status, &stderr))));
    }
    if exists_local(dst) {
        let _ = std::fs::remove_file(&part);
        return Err(failed(EXISTS));
    }
    std::fs::rename(&part, dst).map_err(|e| {
        let _ = std::fs::remove_file(&part);
        failed(format!("rinomina non riuscita: {e}"))
    })
}

fn make_dir(c: &mut Ctx, item: &Item, upload: bool) -> ItemResult {
    c.p.start_file(&item.rel);
    if upload {
        c.run_ok(MKDIR_NEW_SCRIPT, &[&item.dst]).map_err(failed)
    } else {
        std::fs::create_dir(&item.dst).map_err(|e| {
            if e.kind() == std::io::ErrorKind::AlreadyExists { failed(EXISTS) } else { failed(format!("creazione non riuscita: {e}")) }
        })
    }
}

// ---------- elenco delle voci ----------

type Plan = (Vec<Item>, Vec<TransferError>);

fn err_entry(path: &str, message: impl Into<String>) -> TransferError {
    TransferError { path: clean(path), message: message.into() }
}

fn walk_local(dir: &Path, rel: &str, dst: &str, items: &mut Vec<Item>, errors: &mut Vec<TransferError>) {
    let rd = match std::fs::read_dir(dir) {
        Ok(r) => r,
        Err(e) => return errors.push(err_entry(rel, format!("cartella non leggibile: {e}"))),
    };
    let mut entries: Vec<_> = rd.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let Some(name) = e.file_name().to_str().map(str::to_string) else {
            errors.push(err_entry(&e.file_name().to_string_lossy(), "nome non valido (non UTF-8)"));
            continue;
        };
        if items.len() >= MAX_ITEMS {
            return errors.push(err_entry(rel, "troppe voci: il resto è saltato"));
        }
        add_local(&e.path(), &join(rel, &name), &join(dst, &name), false, items, errors);
    }
}

fn add_local(src: &Path, rel: &str, dst: &str, top: bool, items: &mut Vec<Item>, errors: &mut Vec<TransferError>) {
    let meta = match std::fs::symlink_metadata(src) {
        Ok(m) => m,
        Err(e) => return errors.push(err_entry(rel, format!("non leggibile: {e}"))),
    };
    let src_s = src.to_string_lossy().into_owned();
    let ft = meta.file_type();
    if ft.is_symlink() {
        errors.push(err_entry(rel, "saltato: collegamento simbolico"));
    } else if ft.is_dir() {
        items.push(Item { kind: Kind::Dir, rel: rel.into(), src: src_s, dst: dst.into(), size: 0, top });
        walk_local(src, rel, dst, items, errors);
    } else if ft.is_file() {
        items.push(Item { kind: Kind::File, rel: rel.into(), src: src_s, dst: dst.into(), size: meta.len(), top });
    } else {
        errors.push(err_entry(rel, "saltato: non è un file normale"));
    }
}

fn plan_upload(sources: &[PathBuf], remote_dir: &str) -> Plan {
    let (mut items, mut errors) = (Vec::new(), Vec::new());
    for s in sources {
        let Some(name) = s.file_name().and_then(|n| n.to_str()) else {
            errors.push(err_entry(&s.to_string_lossy(), "nome non valido"));
            continue;
        };
        add_local(s, name, &join(remote_dir, name), true, &mut items, &mut errors);
    }
    (items, errors)
}

/// Nome dell'ultimo componente di un percorso remoto assoluto (mai la radice).
fn base_name(path: &str) -> Option<&str> {
    let t = path.trim_end_matches('/');
    let n = t.rsplit('/').next()?;
    if n.is_empty() || n == "." || n == ".." { None } else { Some(n) }
}

fn plan_download(t: &dyn Transport, sources: &[String], local_dir: &Path) -> Plan {
    let (mut items, mut errors) = (Vec::new(), Vec::new());
    let local_dir = local_dir.to_string_lossy().into_owned();
    'sources: for s in sources {
        if let Err(e) = validate_path(s) {
            errors.push(err_entry(s, e.message));
            continue;
        }
        let root = s.trim_end_matches('/');
        let Some(name) = base_name(root) else {
            errors.push(err_entry(s, "percorso non valido"));
            continue;
        };
        let out = match t.run(WALK_SCRIPT, &[root], None) {
            Ok(o) => o,
            Err(e) => {
                errors.push(err_entry(name, e));
                continue;
            }
        };
        if out.status == EXIT_NOT_FOUND {
            errors.push(err_entry(name, "non trovato sul server"));
            continue;
        }
        if out.status != 0 {
            errors.push(err_entry(name, script_failure(out.status, &out.stderr)));
            continue;
        }
        for rec in out.stdout.split(|b| *b == 0).filter(|r| !r.is_empty()) {
            if items.len() >= MAX_ITEMS {
                errors.push(err_entry(name, "troppe voci: il resto è saltato"));
                break 'sources;
            }
            let Ok(rec) = std::str::from_utf8(rec) else {
                errors.push(err_entry(name, "nome non valido (non UTF-8): voce saltata"));
                continue;
            };
            let mut it = rec.splitn(3, '\t');
            let (Some(kind), Some(size), Some(path)) = (it.next(), it.next(), it.next()) else { continue };
            let sub = if path == root { Some("") } else { path.strip_prefix(root).and_then(|r| r.strip_prefix('/')) };
            // ogni componente del percorso relativo è un nome vero: niente `.`, `..`, vuoti
            let Some(sub) = sub.filter(|r| r.is_empty() || r.split('/').all(|c| !c.is_empty() && c != "." && c != "..")) else {
                errors.push(err_entry(name, "percorso inatteso dal server: voce saltata"));
                continue;
            };
            let rel = if sub.is_empty() { name.to_string() } else { join(name, sub) };
            let dst = join(&local_dir, &rel);
            let top = sub.is_empty();
            match kind {
                "d" => items.push(Item { kind: Kind::Dir, rel, src: path.into(), dst, size: 0, top }),
                "f" => items.push(Item { kind: Kind::File, rel, src: path.into(), dst, size: size.parse().unwrap_or(0), top }),
                "l" => errors.push(err_entry(&rel, "saltato: collegamento simbolico")),
                _ => errors.push(err_entry(&rel, "saltato: non è un file normale")),
            }
        }
    }
    (items, errors)
}

// ---------- esecuzione ----------

/// Fine di un trasferimento: lo stato dell'ultimo evento.
#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    pub state: TransferState,
    pub errors: Vec<TransferError>,
    pub files_copied: u64,
}

/// Esegue la richiesta fino alla fine (o all'annullamento), emettendo `running` (al massimo uno ogni `gap`) e un
/// ultimo evento. Da chiamare su un thread proprio.
pub fn execute(id: &str, req: &Request, t: &dyn Transport, cancel: &Arc<AtomicBool>, emit: &dyn Fn(TransferEvent), gap: Duration) -> Outcome {
    let mut c = Ctx { t, cancel, p: Progress { id, emit, gap, last: None, done: 0, total: 0, file: String::new() } };
    c.p.tick(true); // "sto preparando": totale ancora ignoto
    let upload = matches!(req, Request::Upload { .. });
    let (items, mut errors) = match req {
        Request::Upload { sources, remote_dir } => plan_upload(sources, remote_dir),
        Request::Download { sources, local_dir } => plan_download(t, sources, local_dir),
    };
    c.p.total = items.iter().map(|i| i.size).sum();
    c.p.tick(true);
    let (mut ok, mut files) = (0u64, 0u64);
    let mut dead_dirs: Vec<String> = Vec::new();
    let mut was_cancelled = c.cancelled();
    for item in &items {
        if was_cancelled || c.cancelled() {
            was_cancelled = true;
            break;
        }
        if dead_dirs.iter().any(|d| item.rel.starts_with(d.as_str())) {
            c.p.total = c.p.total.saturating_sub(item.size);
            continue;
        }
        let r = match (item.kind, upload) {
            (Kind::Dir, up) => make_dir(&mut c, item, up),
            (Kind::File, true) => upload_file(&mut c, item),
            (Kind::File, false) => download_file(&mut c, item),
        };
        match r {
            Ok(()) => {
                ok += 1;
                if item.kind == Kind::File {
                    files += 1;
                }
            }
            Err(ItemErr::Cancelled) => {
                was_cancelled = true;
                break;
            }
            Err(ItemErr::Failed(m)) => {
                errors.push(err_entry(&item.rel, m));
                if item.kind == Kind::Dir {
                    dead_dirs.push(format!("{}/", item.rel));
                }
            }
        }
    }
    let (state, message) = if was_cancelled {
        let m = match files {
            0 => "Annullato: nessun file era stato copiato".to_string(),
            1 => "Annullato: 1 file già copiato resta a destinazione".to_string(),
            n => format!("Annullato: {n} file già copiati restano a destinazione"),
        };
        (TransferState::Cancelled, m)
    } else if ok == 0 && !errors.is_empty() {
        (TransferState::Error, "Nessun elemento copiato".to_string())
    } else if errors.is_empty() {
        (TransferState::Done, "Trasferimento completato".to_string())
    } else {
        (TransferState::Done, format!("Completato con {} errori", errors.len()))
    };
    c.p.finish(state, errors.clone(), message);
    Outcome { state, errors, files_copied: files }
}

// ---------- registro dei trasferimenti in corso ----------

/// Trasferimenti in corso: ognuno ha il suo thread, il progetto (finestra) che l'ha avviato e il suo flag.
#[derive(Default)]
pub struct Transfers {
    next: AtomicU64,
    active: Mutex<std::collections::HashMap<String, (String, Arc<AtomicBool>)>>,
}

struct Registered<'a> {
    reg: &'a Transfers,
    id: String,
}

impl Drop for Registered<'_> {
    fn drop(&mut self) {
        crate::lock(&self.reg.active).remove(&self.id);
    }
}

impl Transfers {
    /// Avvia il trasferimento su un thread proprio e ne restituisce l'id (unico in questo avvio dell'app).
    pub fn start(self: &Arc<Self>, project_id: &str, req: Request, t: Arc<dyn Transport>, emit: Emitter) -> String {
        let id = format!("t{}", self.next.fetch_add(1, Ordering::Relaxed) + 1);
        let cancel = Arc::new(AtomicBool::new(false));
        crate::lock(&self.active).insert(id.clone(), (project_id.to_string(), cancel.clone()));
        let (me, tid) = (self.clone(), id.clone());
        std::thread::spawn(move || {
            let _guard = Registered { reg: &me, id: tid.clone() };
            execute(&tid, &req, t.as_ref(), &cancel, &*emit, EVENT_GAP);
        });
        id
    }

    /// Annulla solo un trasferimento avviato dallo stesso progetto; `false` = sconosciuto o già finito.
    pub fn cancel(&self, project_id: &str, id: &str) -> bool {
        match crate::lock(&self.active).get(id) {
            Some((p, flag)) if p == project_id => {
                flag.store(true, Ordering::SeqCst);
                true
            }
            _ => false,
        }
    }
}

// ---------- comandi ----------

use crate::fs_tree::{ensure_inside, roots_of, writable_roots_for_label};
use tauri::{AppHandle, Manager, WebviewWindow};

struct Prepared {
    project_id: String,
    server: super::Server,
    conns: Arc<super::conn::Connections>,
}

fn prepare(app: &AppHandle, window: &WebviewWindow, id: &str) -> Result<(Prepared, crate::store::Store), String> {
    let store = super::trusted_store(app, window)?;
    let project_id = crate::projects::window_project_id(window)?;
    let server = super::find_server(&store, super::read_user_config().as_deref(), id)?;
    let conns = app.state::<Arc<super::conn::Connections>>().inner().clone();
    Ok((Prepared { project_id, server, conns }, store))
}

fn start_transfer(app: &AppHandle, p: Prepared, req: Request) -> String {
    let transfers = app.state::<Arc<Transfers>>().inner().clone();
    let (handle, pid) = (app.clone(), p.project_id.clone());
    let emit: Emitter = Arc::new(move |e: TransferEvent| crate::projects::emit_to_project(&handle, &pid, "ssh:transfer", e));
    transfers.start(&p.project_id, req, Arc::new(SshTransport { conns: p.conns, server: p.server }), emit)
}

fn remote_path(p: &Prepared, path: &str) -> Result<String, String> {
    let run = |s: &str, a: &[&str], i: Option<Vec<u8>>| p.conns.run(&p.server, s, a, i);
    rfs::resolve(&run, path).map_err(|e| e.message)
}

/// Carica file e cartelle del progetto in `remote_dir` sul server; restituisce l'id del trasferimento
/// (avanzamento su `ssh:transfer`). Ogni sorgente deve stare dentro il progetto della finestra.
#[tauri::command]
pub async fn ssh_upload(app: AppHandle, window: WebviewWindow, id: String, local_paths: Vec<String>, remote_dir: String) -> Result<String, String> {
    let (p, store) = prepare(&app, &window, &id)?;
    let roots = roots_of(&window, &store).map_err(|e| e.message)?;
    if local_paths.is_empty() {
        return Err("nessun file da caricare".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let sources = local_paths
            .iter()
            .map(|s| ensure_inside(&roots, Path::new(s)).map_err(|e| e.message))
            .collect::<Result<Vec<_>, _>>()?;
        let remote_dir = remote_path(&p, &remote_dir)?;
        Ok(start_transfer(&app, p, Request::Upload { sources, remote_dir }))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Scarica file e cartelle del server in `local_dir` (dentro il progetto); restituisce l'id del trasferimento.
#[tauri::command]
pub async fn ssh_download(app: AppHandle, window: WebviewWindow, id: String, remote_paths: Vec<String>, local_dir: String) -> Result<String, String> {
    let (p, store) = prepare(&app, &window, &id)?;
    let roots = writable_roots_for_label(&store, window.label()).map_err(|e| e.message)?;
    if remote_paths.is_empty() {
        return Err("nessun file da scaricare".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let local_dir = ensure_inside(&roots, Path::new(&local_dir)).map_err(|e| e.message)?;
        if !local_dir.is_dir() {
            return Err("cartella di destinazione inesistente".to_string());
        }
        let sources = remote_paths.iter().map(|s| remote_path(&p, s)).collect::<Result<Vec<_>, _>>()?;
        Ok(start_transfer(&app, p, Request::Download { sources, local_dir }))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Annulla un trasferimento avviato da questa finestra; già finito o di un'altra finestra = nulla.
#[tauri::command]
pub fn ssh_transfer_cancel(window: WebviewWindow, transfers: tauri::State<'_, Arc<Transfers>>, transfer_id: String) -> Result<(), String> {
    let project_id = crate::projects::window_project_id(&window)?;
    transfers.cancel(&project_id, &transfer_id);
    Ok(())
}

#[cfg(test)]
mod tests;
