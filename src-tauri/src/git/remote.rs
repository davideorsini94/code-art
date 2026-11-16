//! Operazioni remote lunghe (fetch, pull, push): processo in una sessione propria, avanzamento letto da stderr
//! (`--progress`), annullabili (`stop_group`: SIGTERM, poi SIGKILL), timeout 10 minuti. Mai `--force`. Nessuna richiesta di credenziali
//! nel terminale (`GIT_TERMINAL_PROMPT=0`, ssh in `BatchMode`); gli helper di credenziali dell'utente restano attivi.
use super::cmd::{base_command, git, READ_TIMEOUT, REMOTE_TIMEOUT};
use super::resolve_repo;
use super::write::{lock_repo, RepoGuard};
use crate::claude::locate::{in_new_session, stop_group, TERM_GRACE};
use crate::store::{Result, Store};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager, WebviewWindow};

pub const CANCELLED: &str = "Operazione annullata";
pub const PULL_MODE_KEY: &str = "git_pull_mode";
pub const PULL_MODES: &[&str] = &["ff-only", "rebase", "merge"];
pub const SSH_BATCH: &str = "ssh -o BatchMode=yes";
/// Righe di avanzamento più lunghe vengono accorciate.
const MAX_LINE_CHARS: usize = 500;
/// Righe conservate per il messaggio d'errore.
const TAIL_LINES: usize = 6;
/// Le righe di avanzamento con percentuale si inoltrano al massimo ogni tanto.
const PROGRESS_EVERY: Duration = Duration::from_millis(100);
/// Dopo l'uscita del processo, quanto aspettare ancora la chiusura di stderr.
const DRAIN_GRACE: Duration = Duration::from_millis(300);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RemoteOp {
    Fetch,
    Pull,
    Push,
}

/// Eventi per la finestra: `git:progress` `{ id, line }` e `git:op_done` `{ id, ok, message }`.
#[derive(Debug, Clone, PartialEq)]
pub enum RemoteEvent {
    Progress { id: u64, line: String },
    Done { id: u64, ok: bool, message: String },
}

#[derive(Serialize, Clone)]
struct ProgressPayload {
    id: u64,
    line: String,
}

#[derive(Serialize, Clone)]
struct DonePayload {
    id: u64,
    ok: bool,
    message: String,
}

pub fn valid_pull_mode(m: &str) -> bool {
    PULL_MODES.contains(&m)
}

/// Opzioni di `pull` per l'impostazione `git_pull_mode` (predefinito e valori sconosciuti: `--ff-only`).
pub fn pull_args(mode: Option<&str>) -> Vec<&'static str> {
    match mode {
        Some("rebase") => vec!["--rebase"],
        Some("merge") => vec!["--no-rebase", "--no-edit"],
        _ => vec!["--ff-only"],
    }
}

/// `GIT_SSH_COMMAND` da imporre: ssh senza domande, a meno che l'utente non abbia già scelto il proprio comando
/// ssh (`GIT_SSH_COMMAND`/`GIT_SSH` nell'ambiente, `core.sshCommand` nella configurazione).
pub fn ssh_override(env: &HashMap<String, String>, dir: &Path) -> Option<&'static str> {
    if env.contains_key("GIT_SSH_COMMAND") || env.contains_key("GIT_SSH") {
        return None;
    }
    let configured = git(dir, &["config", "--get", "core.sshCommand"], READ_TIMEOUT).is_ok_and(|o| o.status == 0);
    (!configured).then_some(SSH_BATCH)
}

/// Spezza lo stderr di git in righe su `\n` e `\r` (l'avanzamento si riscrive con `\r`).
#[derive(Default)]
pub struct LineSplitter(Vec<u8>);

impl LineSplitter {
    pub fn feed(&mut self, bytes: &[u8], out: &mut dyn FnMut(&str)) {
        for &b in bytes {
            if b == b'\n' || b == b'\r' {
                self.flush(out);
            } else {
                self.0.push(b);
            }
        }
    }

    pub fn finish(&mut self, out: &mut dyn FnMut(&str)) {
        self.flush(out);
    }

    fn flush(&mut self, out: &mut dyn FnMut(&str)) {
        let line = String::from_utf8_lossy(&self.0).into_owned();
        self.0.clear();
        if !line.trim().is_empty() {
            let line: String = line.chars().take(MAX_LINE_CHARS).collect();
            out(&line);
        }
    }
}

/// Riga di avanzamento con percentuale ("Receiving objects:  45% (9/20)"): una cifra seguita da `% ` o `%,`.
/// Un URL con caratteri codificati (`%2F`) non lo è, e resta nel messaggio d'errore.
fn is_progress(line: &str) -> bool {
    line.contains(": ") && line.as_bytes().windows(3).any(|w| w[0].is_ascii_digit() && w[1] == b'%' && matches!(w[2], b' ' | b','))
}

/// Argomenti per `op`. `push` imposta l'upstream se manca (`-u origin <branch>`).
fn op_args(store: &Store, dir: &Path, op: RemoteOp) -> Result<Vec<String>> {
    let mut args: Vec<String> = match op {
        RemoteOp::Fetch => vec!["fetch".into(), "--progress".into()],
        RemoteOp::Pull => {
            let mode = store.setting_get(PULL_MODE_KEY)?;
            let mut a = vec!["pull".to_string(), "--progress".into()];
            a.extend(pull_args(mode.as_deref()).into_iter().map(String::from));
            a
        }
        RemoteOp::Push => vec!["push".into(), "--progress".into()],
    };
    if op == RemoteOp::Push {
        let head = git(dir, &["symbolic-ref", "--quiet", "--short", "HEAD"], READ_TIMEOUT)?;
        if head.status != 0 {
            return Err("HEAD staccato: passa a un branch prima di inviare".into());
        }
        let branch = String::from_utf8_lossy(&head.stdout).trim_end_matches('\n').to_string();
        let upstream = git(dir, &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"], READ_TIMEOUT)?;
        if upstream.status != 0 {
            if git(dir, &["remote", "get-url", "origin"], READ_TIMEOUT)?.status != 0 {
                return Err("Nessun remoto «origin» a cui inviare".into());
            }
            // il nome viene da git (un branch non può iniziare con `-`)
            args.extend(["-u".to_string(), "origin".into(), branch]);
        }
    }
    Ok(args)
}

fn remote_command(dir: &Path) -> Result<Command> {
    let mut cmd = base_command(dir)?;
    if let Some(ssh) = ssh_override(crate::tools::env(), dir) {
        cmd.env("GIT_SSH_COMMAND", ssh);
    }
    Ok(cmd)
}

/// Comando completo per `op` in `dir` (argomenti e ssh), lo stesso per le operazioni in sottofondo e per `push_blocking`.
fn op_command(store: &Store, dir: &Path, op: RemoteOp) -> Result<Command> {
    let args = op_args(store, dir, op)?;
    let mut cmd = remote_command(dir)?;
    cmd.args(&args);
    Ok(cmd)
}

/// Push sincrono sullo stesso percorso di `git_remote_op` (upstream impostato se manca, mai `--force`), per chi
/// tiene già il repository occupato (`lock_repo`): la creazione di una pull request.
pub(crate) fn push_blocking(store: &Store, dir: &Path) -> Result<String> {
    run(op_command(store, dir, RemoteOp::Push)?, &AtomicBool::new(false), REMOTE_TIMEOUT, &mut |_| {})
}

/// Esegue `cmd` finché finisce, scade o viene annullato; `on_line` riceve le righe di stderr (quelle con
/// percentuale al massimo ogni `PROGRESS_EVERY`). `Ok` = ultima riga utile, `Err` = messaggio leggibile.
fn run(mut cmd: Command, cancel: &AtomicBool, timeout: Duration, on_line: &mut dyn FnMut(&str)) -> Result<String> {
    in_new_session(&mut cmd);
    let mut child = cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped()).spawn().map_err(|e| format!("avvio non riuscito: {e}"))?;
    let (tx, rx) = mpsc::channel::<String>();
    if let Some(mut err) = child.stderr.take() {
        std::thread::spawn(move || {
            let mut split = LineSplitter::default();
            let mut chunk = [0u8; 4096];
            let mut send = |l: &str| {
                let _ = tx.send(l.to_string());
            };
            while let Ok(n) = err.read(&mut chunk) {
                if n == 0 {
                    break;
                }
                split.feed(&chunk[..n], &mut send);
            }
            split.finish(&mut send);
        });
    }
    let mut tail: VecDeque<String> = VecDeque::new();
    let mut last_progress: Option<Instant> = None;
    let mut handle = |line: String| {
        let progress = is_progress(&line);
        // la riga finale di una fase ("100% (20/20), done.") passa sempre: l'interfaccia non resta al 97%
        let final_step = line.ends_with("done.");
        if !progress || final_step || last_progress.is_none_or(|t| t.elapsed() >= PROGRESS_EVERY) {
            on_line(&line);
            if progress {
                last_progress = Some(Instant::now());
            }
        }
        if !progress {
            if tail.len() == TAIL_LINES {
                tail.pop_front();
            }
            tail.push_back(line);
        }
    };
    let deadline = Instant::now() + timeout;
    let status = loop {
        if cancel.load(Ordering::SeqCst) {
            // SIGTERM prima: git toglie i suoi lock (indice, ref) e il repository resta usabile
            stop_group(&mut child, TERM_GRACE);
            return Err(CANCELLED.into());
        }
        match child.try_wait() {
            Ok(Some(st)) => break st,
            Ok(None) if Instant::now() >= deadline => {
                stop_group(&mut child, TERM_GRACE);
                return Err(format!("tempo scaduto dopo {} s", timeout.as_secs().max(1)));
            }
            Ok(None) => {
                if let Ok(line) = rx.recv_timeout(Duration::from_millis(50)) {
                    handle(line);
                }
            }
            Err(e) => return Err(format!("attesa non riuscita: {e}")),
        }
    };
    // un discendente può tenere aperto stderr: si abbandona dopo la tolleranza
    let until = Instant::now() + DRAIN_GRACE;
    while let Ok(line) = rx.recv_timeout(until.saturating_duration_since(Instant::now())) {
        handle(line);
    }
    let tail: Vec<String> = tail.into_iter().collect();
    if status.success() {
        Ok(tail.last().cloned().unwrap_or_default())
    } else {
        let why = if tail.is_empty() { format!("uscita {}", status.code().unwrap_or(-1)) } else { tail.join("\n") };
        Err(format!("git: {why}"))
    }
}

/// Operazioni remote in corso: id → (progetto, flag di annullamento).
#[derive(Default)]
pub struct RemoteOps {
    next: AtomicU64,
    active: Mutex<HashMap<u64, (String, Arc<AtomicBool>)>>,
}

impl RemoteOps {
    /// Annulla `id` se appartiene al progetto della finestra chiamante; `true` se era in corso.
    pub fn cancel(&self, project_id: &str, id: u64) -> bool {
        match crate::lock(&self.active).get(&id) {
            Some((p, flag)) if p == project_id => {
                flag.store(true, Ordering::SeqCst);
                true
            }
            _ => false,
        }
    }

    /// Annulla tutte le operazioni del progetto (chiusura della finestra).
    pub fn cancel_project(&self, project_id: &str) {
        for (p, flag) in crate::lock(&self.active).values() {
            if p == project_id {
                flag.store(true, Ordering::SeqCst);
            }
        }
    }
}

/// Avvia `op` su un thread dedicato e restituisce subito l'id. Il repository resta occupato fino alla fine (una
/// seconda scrittura riceve subito `BUSY`); `emit` riceve l'avanzamento e, una volta sola, la fine.
pub fn start(store: &Store, project_id: &str, repo: &str, op: RemoteOp, ops: Arc<RemoteOps>, emit: impl Fn(RemoteEvent) + Send + Sync + 'static) -> Result<u64> {
    let dir = resolve_repo(store, project_id, repo)?;
    let guard: RepoGuard = lock_repo(&dir)?;
    let cmd = op_command(store, &dir, op)?;
    let id = ops.next.fetch_add(1, Ordering::Relaxed) + 1;
    let cancel = Arc::new(AtomicBool::new(false));
    crate::lock(&ops.active).insert(id, (project_id.to_string(), cancel.clone()));
    std::thread::spawn(move || {
        let _guard = guard;
        let result = run(cmd, &cancel, REMOTE_TIMEOUT, &mut |line| emit(RemoteEvent::Progress { id, line: line.to_string() }));
        crate::lock(&ops.active).remove(&id);
        let (ok, message) = match result {
            Ok(m) => (true, m),
            Err(m) => (false, m),
        };
        // il repository si libera prima dell'evento: l'interfaccia può ripartire subito
        drop(_guard);
        emit(RemoteEvent::Done { id, ok, message });
    });
    Ok(id)
}

// ---------- comandi ----------

#[tauri::command]
pub async fn git_remote_op(app: AppHandle, window: WebviewWindow, repo: String, op: RemoteOp) -> Result<u64> {
    let id = crate::projects::window_project_id(&window)?;
    crate::projects::off_main(move || {
        let ops = app.state::<Arc<RemoteOps>>().inner().clone();
        let (handle, pid) = (app.clone(), id.clone());
        start(&app.state::<Store>(), &id, &repo, op, ops, move |e| match e {
            RemoteEvent::Progress { id, line } => crate::projects::emit_to_project(&handle, &pid, "git:progress", ProgressPayload { id, line }),
            RemoteEvent::Done { id, ok, message } => crate::projects::emit_to_project(&handle, &pid, "git:op_done", DonePayload { id, ok, message }),
        })
    })
    .await
}

/// Annulla l'operazione `id` se appartiene alla finestra chiamante.
#[tauri::command]
pub fn git_cancel(window: WebviewWindow, ops: tauri::State<'_, Arc<RemoteOps>>, id: u64) -> Result<()> {
    let project_id = crate::projects::window_project_id(&window)?;
    ops.cancel(&project_id, id);
    Ok(())
}

#[cfg(test)]
mod tests;
