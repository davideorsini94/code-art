//! Connessione condivisa per server: un master `ssh` (ControlMaster) per server, socket nella cartella dati dell'app,
//! e le operazioni (`run`) come client di quel master. Sempre `/usr/bin/ssh`, `/usr/bin/ssh-keygen`,
//! `/usr/bin/ssh-keyscan` con lista di argomenti; l'unica shell è quella del server, con `remote_cmd` (script
//! costante, argomenti passati da `quote_sh`).
//!
//! Chiavi dei server: `StrictHostKeyChecking=yes` con `~/.ssh/known_hosts` e `<dati app>/known_hosts`. Server
//! sconosciuto: `ssh-keyscan`, impronte in un dialogo nativo (il chiamante fornisce `confirm`), solo con conferma la
//! chiave si aggiunge a `<dati app>/known_hosts` (mai in `~/.ssh`). Chiave cambiata: errore, nessuna conferma.
use super::{validate_target, Server, ServerSource};
use crate::tools::{self, ToolOutput};
use serde::Serialize;
use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const SSH: &str = super::askpass::SSH_BIN;
pub const SSH_KEYGEN: &str = "/usr/bin/ssh-keygen";
pub const SSH_KEYSCAN: &str = "/usr/bin/ssh-keyscan";

/// Lunghezza massima del socket del master. OpenSSH lo crea prima con un nome temporaneo (`<percorso>.<16 caratteri>`)
/// e poi lo rinomina, e `sun_path` su macOS ha 104 byte: il limite vale per quel nome temporaneo.
const SOCKET_MAX: usize = 100;
const SOCKET_TMP_SUFFIX: usize = 17;

/// Tempo massimo per stabilire il master (connessione e autenticazione).
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
/// Ogni quanto si controlla che il master sia ancora vivo.
const MONITOR_EVERY: Duration = Duration::from_secs(5);
/// Tempo massimo di un comando breve (`-G`, `-O`, `ssh-keygen`).
const QUICK_TIMEOUT: Duration = Duration::from_secs(10);
/// Tempo massimo predefinito di `run`.
pub const RUN_TIMEOUT: Duration = Duration::from_secs(60);

/// Codice d'errore di `connect` quando l'autenticazione non riesce: l'interfaccia chiede la password.
pub const AUTH: &str = "auth";
pub const DISCONNECTED: &str = "Non connesso al server";
pub const CANCELLED: &str = "Connessione annullata";

/// Dialogo per un server nuovo (titolo fisso; "Annulla" è il primo pulsante, quindi il predefinito di NSAlert).
pub const NEW_HOST_TITLE: &str = "Nuovo server SSH";
pub const BTN_CANCEL: &str = "Annulla";
pub const BTN_TRUST_HOST: &str = "Aggiungi e connetti";

/// Percorsi usati dalle connessioni.
#[derive(Debug, Clone)]
pub struct SshPaths {
    /// `<dati app>` (`~/Library/Application Support/CodeArt`).
    pub data_dir: PathBuf,
    /// Cartella di casa (per `~/.ssh/known_hosts`).
    pub home: PathBuf,
    /// `$TMPDIR`, se il socket nella cartella dati è troppo lungo.
    pub tmp_dir: PathBuf,
    pub uid: u32,
}

impl SshPaths {
    pub fn new(data_dir: PathBuf) -> Self {
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
        // SAFETY: `getuid` non fallisce mai.
        let uid = unsafe { libc::getuid() };
        SshPaths { data_dir, home, tmp_dir: std::env::temp_dir(), uid }
    }

    pub fn app_known_hosts(&self) -> PathBuf {
        self.data_dir.join("known_hosts")
    }

    pub fn user_known_hosts(&self) -> PathBuf {
        self.home.join(".ssh/known_hosts")
    }

    /// Socket del master: `<dati app>/s/<8 hex>`; se troppo lungo `$TMPDIR/codeart-<uid>/<8 hex>`, poi
    /// `/tmp/codeart-<uid>/<8 hex>` (cartelle 0700, vedi `ensure_private_dir`).
    pub fn socket(&self, id: &str) -> PathBuf {
        let h = short_hash(id);
        let candidates = [
            self.data_dir.join("s"),
            self.tmp_dir.join(format!("codeart-{}", self.uid)),
            PathBuf::from(format!("/tmp/codeart-{}", self.uid)),
        ];
        let last = candidates[2].join(&h);
        candidates.into_iter().map(|d| d.join(&h)).find(|p| socket_fits(p)).unwrap_or(last)
    }
}

fn socket_fits(p: &Path) -> bool {
    p.as_os_str().len() + SOCKET_TMP_SUFFIX <= SOCKET_MAX
}

/// Hash stabile (FNV-1a a 32 bit) dell'id del server, in 8 cifre esadecimali: non cambia tra versioni di Rust.
pub fn short_hash(id: &str) -> String {
    let mut h: u32 = 0x811c_9dc5;
    for b in id.bytes() {
        h ^= u32::from(b);
        h = h.wrapping_mul(0x0100_0193);
    }
    format!("{h:08x}")
}

/// Valore di un'opzione `-o` di OpenSSH tra virgolette (i percorsi dell'app contengono spazi:
/// "Application Support"); `%` raddoppiato perché ControlPath e UserKnownHostsFile espandono i `%x`.
fn cfg_value(p: &Path) -> String {
    let s = p.to_string_lossy().replace('\\', "\\\\").replace('"', "\\\"").replace('%', "%%");
    format!("\"{s}\"")
}

/// Destinazione passata a `ssh`: l'host per i salvati, l'alias per quelli di `~/.ssh/config`.
pub fn destination(server: &Server) -> String {
    match server.source {
        ServerSource::Saved => server.host.clone(),
        ServerSource::Config => server.id.strip_prefix("config:").unwrap_or(&server.name).to_string(),
    }
}

/// Destinazione e utente validi (`validate_target`): l'unica difesa contro l'iniezione di opzioni.
pub fn check_server(server: &Server) -> Result<(), String> {
    validate_target("Host", &destination(server))?;
    if let Some(u) = server.user.as_deref().filter(|_| server.source == ServerSource::Saved) {
        validate_target("Utente", u)?;
    }
    if server.port == Some(0) {
        return Err("Porta: da 1 a 65535".into());
    }
    Ok(())
}

/// Argomenti comuni: socket condiviso (`ControlMaster=no`: client del master), chiavi verificate, poi porta, utente
/// e identità per i salvati, poi la destinazione.
pub fn base_args(server: &Server, paths: &SshPaths) -> Vec<String> {
    args_with(server, paths, false, &[])
}

/// Come `base_args`, con `ControlMaster=yes` per il master e `extra` subito prima della destinazione.
fn args_with(server: &Server, paths: &SshPaths, master: bool, extra: &[&str]) -> Vec<String> {
    let mut a: Vec<String> = vec![
        "-o".into(),
        format!("ControlPath={}", cfg_value(&paths.socket(&server.id))),
        "-o".into(),
        format!("ControlMaster={}", if master { "yes" } else { "no" }),
        "-o".into(),
        "StrictHostKeyChecking=yes".into(),
        "-o".into(),
        format!("UserKnownHostsFile=\"~/.ssh/known_hosts\" {}", cfg_value(&paths.app_known_hosts())),
        "-o".into(),
        "BatchMode=no".into(),
        "-o".into(),
        "ServerAliveInterval=15".into(),
        "-o".into(),
        "ServerAliveCountMax=3".into(),
    ];
    if server.source == ServerSource::Saved {
        if let Some(p) = server.port {
            a.extend(["-p".into(), p.to_string()]);
        }
        if let Some(u) = server.user.as_deref().filter(|u| !u.is_empty()) {
            a.extend(["-l".into(), u.to_string()]);
        }
        if let Some(i) = server.identity_file.as_deref().filter(|i| !i.is_empty()) {
            a.extend(["-i".into(), i.to_string()]);
        }
    }
    a.extend(extra.iter().map(|s| s.to_string()));
    a.push(destination(server));
    a
}

/// Argomenti del master: in sottofondo dopo l'autenticazione (`-N` con `ControlPersist`), si chiude da solo dopo
/// 10 minuti senza client (anche se l'app termina male).
pub fn master_args(server: &Server, paths: &SshPaths) -> Vec<String> {
    args_with(server, paths, true, &["-M", "-N", "-o", "ControlPersist=600", "-o", "ConnectTimeout=10"])
}

/// `ssh -O <op>` sul socket del server (`check`, `exit`).
pub fn control_args(server: &Server, paths: &SshPaths, op: &str) -> Vec<String> {
    args_with(server, paths, false, &["-O", op])
}

/// Argomenti di `run`: niente terminale (i byte passano intatti), niente inoltri della config, poi il comando.
pub fn run_args(server: &Server, paths: &SshPaths, script: &str, args: &[&str]) -> Vec<String> {
    let mut a = args_with(server, paths, false, &["-T", "-o", "ClearAllForwardings=yes"]);
    a.push(remote_cmd(script, args));
    a
}

/// Parola per la shell POSIX: tra apici singoli, ogni `'` diventa `'\''`.
pub fn quote_sh(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Riga di comando per il server: `sh -c '<script>' _ '<arg1>' …`. Lo script è una costante del core; i dati
/// arrivano solo come argomenti (`$1`, `$2`…), mai nel testo dello script.
pub fn remote_cmd(script: &str, args: &[&str]) -> String {
    let mut s = format!("sh -c {} _", quote_sh(script));
    for a in args {
        s.push(' ');
        s.push_str(&quote_sh(a));
    }
    s
}

// ---------- errori di OpenSSH ----------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    Auth,
    HostKeyChanged,
    HostKeyUnknown,
    Unreachable(&'static str),
    Disconnected,
    Other(String),
}

/// Classifica lo stderr di `ssh` (uscita 255).
pub fn classify(stderr: &str) -> Failure {
    let has = |s: &str| stderr.contains(s);
    if has("REMOTE HOST IDENTIFICATION HAS CHANGED") || has("has changed and you have requested strict checking") {
        return Failure::HostKeyChanged;
    }
    if has("host key is known for") || has("No matching host key") {
        return Failure::HostKeyUnknown;
    }
    if has("Host key verification failed") {
        // il server era nei known_hosts (si verifica prima di connettersi): la chiave non corrisponde
        return Failure::HostKeyChanged;
    }
    if has("Permission denied") || has("Too many authentication failures") {
        return Failure::Auth;
    }
    if has("Could not resolve hostname") {
        return Failure::Unreachable("Server non trovato: il nome non si risolve");
    }
    if has("Connection refused") {
        return Failure::Unreachable("Connessione rifiutata dal server");
    }
    if has("Operation timed out") || has("Connection timed out") {
        return Failure::Unreachable("Server non raggiungibile: tempo scaduto");
    }
    if has("No route to host") || has("Network is unreachable") || has("Host is down") {
        return Failure::Unreachable("Server non raggiungibile");
    }
    if has("Control socket connect")
        || has("mux_client")
        || has("closed by remote host")
        || has("Connection reset")
        || has("Broken pipe")
        || has("Connection closed")
    {
        return Failure::Disconnected;
    }
    let line = stderr
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with("Warning:") && !l.starts_with("Pseudo-terminal"))
        .unwrap_or("");
    let line: String = line.chars().filter(|c| !c.is_control()).take(200).collect();
    Failure::Other(line)
}

impl Failure {
    pub fn message(&self) -> String {
        match self {
            Failure::Auth => "Autenticazione non riuscita".into(),
            Failure::HostKeyChanged => "La chiave del server è cambiata rispetto a quella conosciuta: connessione bloccata. \
                Può essere un attacco (qualcuno tra te e il server) o il server reinstallato: verifica con chi lo gestisce \
                e correggi known_hosts a mano."
                .into(),
            Failure::HostKeyUnknown => "La chiave del server non è conosciuta: riprova a connetterti per verificarla".into(),
            Failure::Unreachable(m) => (*m).into(),
            Failure::Disconnected => "Connessione chiusa".into(),
            Failure::Other(l) if l.is_empty() => "Errore di ssh".into(),
            Failure::Other(l) => format!("Errore di ssh: {l}"),
        }
    }

    /// Errore restituito da `connect`: `auth` per l'autenticazione, altrimenti il messaggio.
    pub fn connect_error(&self) -> String {
        if *self == Failure::Auth { AUTH.into() } else { self.message() }
    }
}

// ---------- chiavi dei server ----------

/// Indirizzo reale di una destinazione secondo `ssh -G` (alias della config risolti).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    pub hostname: String,
    pub port: u16,
    pub host_key_alias: Option<String>,
}

impl Resolved {
    /// Nome cercato in known_hosts, come lo usa OpenSSH: `HostKeyAlias` se c'è, altrimenti `host` (porta 22) o
    /// `[host]:porta`.
    pub fn lookup_name(&self) -> String {
        match &self.host_key_alias {
            Some(a) => a.clone(),
            None => scan_name(&self.hostname, self.port),
        }
    }
}

/// Nome con cui `ssh-keyscan` stampa le chiavi (e con cui OpenSSH le cerca, senza `HostKeyAlias`).
fn scan_name(host: &str, port: u16) -> String {
    if port == 22 { host.to_string() } else { format!("[{host}]:{port}") }
}

/// Legge `hostname`, `port` e `hostkeyalias` dall'uscita di `ssh -G`.
pub fn parse_ssh_g(out: &str) -> Result<Resolved, String> {
    let mut hostname = None;
    let mut port = None;
    let mut alias = None;
    for line in out.lines() {
        let Some((k, v)) = line.trim().split_once(' ') else { continue };
        let v = v.trim();
        match k {
            "hostname" if hostname.is_none() => hostname = Some(v.to_string()),
            "port" if port.is_none() => port = v.parse::<u16>().ok().filter(|p| *p > 0),
            "hostkeyalias" if alias.is_none() && v != "none" => alias = Some(v.to_string()),
            _ => {}
        }
    }
    let hostname = hostname.ok_or("ssh -G: host mancante")?;
    validate_target("Host", &hostname)?;
    if let Some(a) = &alias {
        validate_target("HostKeyAlias", a)?;
    }
    Ok(Resolved { hostname, port: port.ok_or("ssh -G: porta mancante")?, host_key_alias: alias })
}

const KEY_TYPES: &[&str] = &[
    "ssh-ed25519",
    "ssh-rsa",
    "ecdsa-sha2-nistp256",
    "ecdsa-sha2-nistp384",
    "ecdsa-sha2-nistp521",
    "sk-ssh-ed25519@openssh.com",
    "sk-ecdsa-sha2-nistp256@openssh.com",
];

/// Righe di `ssh-keyscan` per `expected` (host o `[host]:porta`), riscritte come `<name> <tipo> <chiave>`: solo
/// tipi noti e chiavi in base64, nient'altro entra in known_hosts.
pub fn parse_keyscan(out: &str, expected: &str, name: &str) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for line in out.lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        let [host, kind, key] = f[..] else { continue };
        let b64 = !key.is_empty() && key.len() <= 16 * 1024 && key.bytes().all(|b| b.is_ascii_alphanumeric() || b"+/=".contains(&b));
        if host == expected && KEY_TYPES.contains(&kind) && b64 {
            let l = format!("{name} {kind} {key}");
            if !lines.contains(&l) {
                lines.push(l);
            }
        }
    }
    lines
}

/// Impronte da `ssh-keygen -lf -`: `(tipo, SHA256:…)`.
pub fn parse_fingerprints(out: &str) -> Vec<(String, String)> {
    out.lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            let fp = *f.get(1)?;
            let kind = f.last()?.strip_prefix('(')?.strip_suffix(')')?;
            let ok = fp.strip_prefix("SHA256:").is_some_and(|b| !b.is_empty() && b.bytes().all(|c| c.is_ascii_alphanumeric() || b"+/=".contains(&c)))
                && !kind.is_empty()
                && kind.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-');
            ok.then(|| (kind.to_string(), fp.to_string()))
        })
        .collect()
}

/// Solo caratteri ASCII visibili (niente caratteri di formato o bidi che camuffano il nome nel dialogo).
fn shown(s: &str) -> String {
    s.chars().take(200).map(|c| if c.is_ascii_graphic() { c } else { '?' }).collect()
}

/// Testo del dialogo per un server nuovo.
pub fn new_host_message(dest: &str, lookup: &str, fps: &[(String, String)]) -> String {
    let mut m = format!("Il server «{}» non è ancora conosciuto.\n", shown(dest));
    if lookup != dest {
        m.push_str(&format!("Indirizzo: {}\n", shown(lookup)));
    }
    m.push_str("\nImpronte della sua chiave:\n");
    for (k, fp) in fps {
        m.push_str(&format!("{k}  {fp}\n"));
    }
    m.push_str("\nAggiungi il server solo se le impronte corrispondono a quelle che ti ha dato chi lo gestisce.");
    m
}

/// File known_hosts in cui OpenSSH cercherà la chiave.
fn known_hosts_files(paths: &SshPaths) -> Vec<PathBuf> {
    [
        paths.user_known_hosts(),
        paths.app_known_hosts(),
        PathBuf::from("/etc/ssh/ssh_known_hosts"),
        PathBuf::from("/etc/ssh/ssh_known_hosts2"),
    ]
    .into()
}

fn command(bin: &str) -> Command {
    tools::command_in(Path::new(bin), tools::env())
}

/// `true` se `name` è in uno dei `files` (`ssh-keygen -F`: 0 trovato, 1 no; righe con hash comprese). I file
/// assenti si saltano.
pub fn is_known(name: &str, files: &[PathBuf]) -> Result<bool, String> {
    for f in files.iter().filter(|f| f.is_file()) {
        let mut c = command(SSH_KEYGEN);
        c.arg("-F").arg(name).arg("-f").arg(f);
        let out = tools::run_tool(c, QUICK_TIMEOUT, None)?;
        match out.status {
            0 => return Ok(true),
            1 => {}
            _ => return Err(format!("ssh-keygen: {}", out.stderr.trim())),
        }
    }
    Ok(false)
}

/// Aggiunge `lines` a un known_hosts (creato 0600).
pub fn append_known_hosts(file: &Path, lines: &[String]) -> Result<(), String> {
    use std::os::unix::fs::OpenOptionsExt;
    let needs_nl = std::fs::read(file).ok().is_some_and(|b| !b.is_empty() && !b.ends_with(b"\n"));
    let mut f = std::fs::OpenOptions::new().create(true).append(true).mode(0o600).open(file).map_err(|e| e.to_string())?;
    let mut text = String::new();
    if needs_nl {
        text.push('\n');
    }
    for l in lines {
        text.push_str(l);
        text.push('\n');
    }
    f.write_all(text.as_bytes()).map_err(|e| e.to_string())
}

/// Cartella privata: creata 0700, non un link simbolico, di proprietà dell'utente.
pub fn ensure_private_dir(dir: &Path, uid: u32) -> Result<(), String> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
    std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let m = std::fs::symlink_metadata(dir).map_err(|e| e.to_string())?;
    if !m.is_dir() || m.uid() != uid {
        return Err(format!("{}: cartella non sicura", dir.display()));
    }
    if m.mode() & 0o777 != 0o700 {
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())?;
    }
    Ok(())
}

// ---------- stato delle connessioni ----------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ConnState {
    Connecting,
    Connected,
    Disconnected,
    Error,
}

/// Evento `ssh:state` e voce di `ssh_states`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StateEvent {
    pub id: String,
    pub state: ConnState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

struct Entry {
    state: ConnState,
    message: Option<String>,
    /// Cambia a ogni connessione o disconnessione: un'operazione vecchia non sovrascrive lo stato nuovo.
    generation: u64,
    server: Server,
}

pub type Emitter = Arc<dyn Fn(StateEvent) + Send + Sync>;

/// Connessioni condivise dell'app (una per server, valide per tutte le finestre).
pub struct Connections {
    paths: SshPaths,
    entries: Mutex<HashMap<String, Entry>>,
    next: AtomicU64,
    emit: Emitter,
}

impl Connections {
    pub fn new(paths: SshPaths, emit: Emitter) -> Self {
        Connections { paths, entries: Mutex::new(HashMap::new()), next: AtomicU64::new(1), emit }
    }

    pub fn paths(&self) -> &SshPaths {
        &self.paths
    }

    pub fn states(&self) -> Vec<StateEvent> {
        let mut v: Vec<StateEvent> = crate::lock(&self.entries)
            .iter()
            .map(|(id, e)| StateEvent { id: id.clone(), state: e.state, message: e.message.clone() })
            .collect();
        v.sort_by(|a, b| a.id.cmp(&b.id));
        v
    }

    pub fn state_of(&self, id: &str) -> Option<ConnState> {
        crate::lock(&self.entries).get(id).map(|e| e.state)
    }

    /// Nuova generazione per `server` nello stato `state`; emette l'evento.
    fn begin(&self, server: &Server, state: ConnState, message: Option<String>) -> u64 {
        let generation = self.next.fetch_add(1, Ordering::Relaxed);
        crate::lock(&self.entries).insert(server.id.clone(), Entry { state, message: message.clone(), generation, server: server.clone() });
        (self.emit)(StateEvent { id: server.id.clone(), state, message });
        generation
    }

    /// Imposta lo stato solo se la generazione è ancora quella corrente; `false` = superata.
    fn set_if(&self, id: &str, generation: u64, state: ConnState, message: Option<String>) -> bool {
        {
            let mut map = crate::lock(&self.entries);
            match map.get_mut(id) {
                Some(e) if e.generation == generation => {
                    e.state = state;
                    e.message = message.clone();
                }
                _ => return false,
            }
        }
        (self.emit)(StateEvent { id: id.to_string(), state, message });
        true
    }

    fn generation_of(&self, id: &str) -> Option<u64> {
        crate::lock(&self.entries).get(id).map(|e| e.generation)
    }

    /// Connette `server` (o non fa nulla se è già connesso o in connessione: lo stato arriva con `ssh:state`).
    /// `confirm` mostra il dialogo nativo per un server nuovo e dice se l'utente l'ha accettato.
    /// Da chiamare fuori dal thread principale.
    pub fn connect(self: &Arc<Self>, server: &Server, confirm: &dyn Fn(&str) -> bool) -> Result<(), String> {
        check_server(server)?;
        {
            let map = crate::lock(&self.entries);
            if map.get(&server.id).is_some_and(|e| matches!(e.state, ConnState::Connecting | ConnState::Connected)) {
                return Ok(());
            }
        }
        let generation = self.begin(server, ConnState::Connecting, None);
        match self.establish(server, generation, confirm) {
            Ok(()) => {
                if self.set_if(&server.id, generation, ConnState::Connected, None) {
                    self.spawn_monitor(server.clone(), generation);
                    Ok(())
                } else {
                    // disconnesso mentre si connetteva: il master appena nato si chiude
                    let _ = self.control(server, "exit");
                    Err(CANCELLED.into())
                }
            }
            Err(ConnectError::Cancelled) => {
                self.set_if(&server.id, generation, ConnState::Disconnected, None);
                Err(CANCELLED.into())
            }
            Err(ConnectError::Failed(f)) => {
                self.set_if(&server.id, generation, ConnState::Error, Some(f.message()));
                Err(f.connect_error())
            }
            Err(ConnectError::Io(m)) => {
                self.set_if(&server.id, generation, ConnState::Error, Some(m.clone()));
                Err(m)
            }
        }
    }

    fn establish(&self, server: &Server, generation: u64, confirm: &dyn Fn(&str) -> bool) -> Result<(), ConnectError> {
        let sock = self.paths.socket(&server.id);
        if let Some(dir) = sock.parent() {
            ensure_private_dir(dir, self.paths.uid).map_err(ConnectError::Io)?;
        }
        ensure_private_dir(&self.paths.data_dir, self.paths.uid).map_err(ConnectError::Io)?;
        // un master ancora vivo (avvio precedente dell'app terminato male) si riusa
        if self.control(server, "check").is_ok_and(|o| o.status == 0) {
            return Ok(());
        }
        // socket rimasto da un master morto: OpenSSH altrimenti disattiverebbe la condivisione
        let _ = std::fs::remove_file(&sock);
        self.verify_host_key(server, confirm)?;
        if self.generation_of(&server.id) != Some(generation) {
            return Err(ConnectError::Cancelled);
        }
        self.start_master(server)
    }

    /// Server già in un known_hosts: nulla da fare. Sconosciuto: `ssh-keyscan`, impronte, conferma, aggiunta a
    /// `<dati app>/known_hosts`. Una chiave cambiata la scopre poi il master (errore, nessuna conferma).
    fn verify_host_key(&self, server: &Server, confirm: &dyn Fn(&str) -> bool) -> Result<(), ConnectError> {
        let mut c = command(SSH);
        c.arg("-G").args(base_args(server, &self.paths));
        let out = tools::run_tool(c, QUICK_TIMEOUT, None).map_err(ConnectError::Io)?;
        if out.status != 0 {
            return Err(ConnectError::Failed(classify(&out.stderr)));
        }
        let r = parse_ssh_g(&String::from_utf8_lossy(&out.stdout)).map_err(ConnectError::Io)?;
        let name = r.lookup_name();
        if is_known(&name, &known_hosts_files(&self.paths)).map_err(ConnectError::Io)? {
            return Ok(());
        }
        let mut c = command(SSH_KEYSCAN);
        c.args(["-T", "5", "-p", &r.port.to_string(), "--", &r.hostname]);
        let out = tools::run_tool(c, Duration::from_secs(15), None).map_err(ConnectError::Io)?;
        let lines = parse_keyscan(&String::from_utf8_lossy(&out.stdout), &scan_name(&r.hostname, r.port), &name);
        if lines.is_empty() {
            let f = match classify(&out.stderr) {
                Failure::Other(_) => Failure::Unreachable("Server non raggiungibile: nessuna chiave ricevuta"),
                f => f,
            };
            return Err(ConnectError::Failed(f));
        }
        let mut c = command(SSH_KEYGEN);
        c.args(["-lf", "-"]);
        let out = tools::run_tool(c, QUICK_TIMEOUT, Some(format!("{}\n", lines.join("\n")).into_bytes())).map_err(ConnectError::Io)?;
        let fps = parse_fingerprints(&String::from_utf8_lossy(&out.stdout));
        if out.status != 0 || fps.is_empty() {
            return Err(ConnectError::Io("impronte della chiave non leggibili".into()));
        }
        if !confirm(&new_host_message(&destination(server), &name, &fps)) {
            return Err(ConnectError::Cancelled);
        }
        append_known_hosts(&self.paths.app_known_hosts(), &lines).map_err(ConnectError::Io)
    }

    /// Avvia il master e aspetta che il socket risponda. Con `-N` e `ControlPersist` OpenSSH passa in sottofondo
    /// dopo l'autenticazione (il processo avviato esce 0); se restasse in primo piano basta il socket pronto.
    fn start_master(&self, server: &Server) -> Result<(), ConnectError> {
        use crate::claude::locate::{in_new_session, stop_group, TERM_GRACE};
        use std::process::Stdio;
        let ticket = tools::RUNNING.register().map_err(ConnectError::Io)?;
        let exe = std::env::current_exe().map_err(|e| ConnectError::Io(e.to_string()))?;
        let mut cmd = command(SSH);
        cmd.args(master_args(server, &self.paths))
            .env("SSH_ASKPASS", exe)
            .env("SSH_ASKPASS_REQUIRE", "force")
            .env(super::askpass::ACCOUNT_ENV, &server.id)
            .env_remove("DISPLAY");
        in_new_session(&mut cmd);
        let mut child = cmd
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| ConnectError::Io(format!("avvio di ssh non riuscito: {e}")))?;
        let err_buf = Arc::new(Mutex::new(Vec::<u8>::new()));
        if let Some(mut e) = child.stderr.take() {
            let buf = err_buf.clone();
            std::thread::spawn(move || {
                let mut chunk = [0u8; 4096];
                while let Ok(n) = std::io::Read::read(&mut e, &mut chunk) {
                    if n == 0 {
                        break;
                    }
                    let mut b = crate::lock(&buf);
                    let room = (8 * 1024usize).saturating_sub(b.len());
                    b.extend_from_slice(&chunk[..n.min(room)]);
                }
            });
        }
        let stderr = || String::from_utf8_lossy(&crate::lock(&err_buf)).into_owned();
        let sock = self.paths.socket(&server.id);
        let deadline = Instant::now() + CONNECT_TIMEOUT;
        let mut exited_ok_at: Option<Instant> = None;
        let mut last_check = Instant::now() - Duration::from_secs(1);
        loop {
            if ticket.stop.load(Ordering::SeqCst) {
                stop_group(&mut child, TERM_GRACE);
                return Err(ConnectError::Io(tools::STOPPED.into()));
            }
            if exited_ok_at.is_none() {
                match child.try_wait() {
                    Ok(Some(st)) if st.success() => exited_ok_at = Some(Instant::now()),
                    Ok(Some(_)) => {
                        // lascia al lettore il tempo di raccogliere le ultime righe
                        std::thread::sleep(Duration::from_millis(100));
                        return Err(ConnectError::Failed(classify(&stderr())));
                    }
                    Ok(None) => {}
                    Err(e) => return Err(ConnectError::Io(format!("attesa non riuscita: {e}"))),
                }
            }
            let ready_to_check = exited_ok_at.is_some() || sock.exists();
            if ready_to_check && last_check.elapsed() >= Duration::from_millis(300) {
                last_check = Instant::now();
                if self.control(server, "check").is_ok_and(|o| o.status == 0) {
                    // in primo piano: il processo resta vivo, il controllo periodico ne segue la fine
                    return Ok(());
                }
            }
            // uscito 0 ma il socket non risponde: errore dopo qualche secondo
            if exited_ok_at.is_some_and(|t| t.elapsed() > Duration::from_secs(5)) {
                return Err(ConnectError::Failed(match classify(&stderr()) {
                    Failure::Other(l) if l.is_empty() => Failure::Other("connessione condivisa non disponibile".into()),
                    f => f,
                }));
            }
            if Instant::now() >= deadline {
                if exited_ok_at.is_none() {
                    stop_group(&mut child, TERM_GRACE);
                }
                return Err(ConnectError::Failed(Failure::Unreachable("Server non raggiungibile: tempo scaduto")));
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// `ssh -O <op>` sul socket del server.
    fn control(&self, server: &Server, op: &str) -> Result<ToolOutput, String> {
        let mut c = command(SSH);
        c.args(control_args(server, &self.paths, op));
        tools::run_tool(c, QUICK_TIMEOUT, None)
    }

    /// Controllo periodico: il master che termina (rete, server, 10 minuti senza client) → `disconnected`.
    fn spawn_monitor(self: &Arc<Self>, server: Server, generation: u64) {
        let me = self.clone();
        std::thread::spawn(move || {
            loop {
                let until = Instant::now() + MONITOR_EVERY;
                while Instant::now() < until {
                    if me.generation_of(&server.id) != Some(generation) {
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(200));
                }
                match me.control(&server, "check") {
                    Ok(o) if o.status == 0 => {}
                    // l'app si sta chiudendo
                    Err(e) if e == tools::STOPPED => return,
                    _ => {
                        me.set_if(&server.id, generation, ConnState::Disconnected, Some(Failure::Disconnected.message()));
                        return;
                    }
                }
            }
        });
    }

    /// Chiude la connessione condivisa di `id` (`ssh -O exit`); già chiusa = nulla da fare.
    pub fn disconnect(&self, id: &str) -> Result<(), String> {
        let server = crate::lock(&self.entries).get(id).map(|e| e.server.clone());
        let Some(server) = server else { return Ok(()) };
        self.begin(&server, ConnState::Disconnected, None);
        let _ = self.control(&server, "exit");
        Ok(())
    }

    /// Esegue `script` sul server con `args` (via `remote_cmd`) sulla connessione condivisa; serve lo stato
    /// `connected`. Uscita 255 = errore di ssh (connessione persa → `disconnected`).
    pub fn run(&self, server: &Server, script: &str, args: &[&str], stdin: Option<Vec<u8>>) -> Result<ToolOutput, String> {
        self.run_with_timeout(server, script, args, stdin, RUN_TIMEOUT)
    }

    pub fn run_with_timeout(
        &self,
        server: &Server,
        script: &str,
        args: &[&str],
        stdin: Option<Vec<u8>>,
        timeout: Duration,
    ) -> Result<ToolOutput, String> {
        check_server(server)?;
        let generation = {
            let map = crate::lock(&self.entries);
            match map.get(&server.id) {
                Some(e) if e.state == ConnState::Connected => e.generation,
                _ => return Err(DISCONNECTED.into()),
            }
        };
        if !self.paths.socket(&server.id).exists() {
            self.set_if(&server.id, generation, ConnState::Disconnected, Some(Failure::Disconnected.message()));
            return Err(DISCONNECTED.into());
        }
        let mut c = command(SSH);
        c.args(run_args(server, &self.paths, script, args));
        let out = tools::run_tool(c, timeout, stdin)?;
        if out.status == 255 {
            let f = classify(&out.stderr);
            if f == Failure::Disconnected {
                self.set_if(&server.id, generation, ConnState::Disconnected, Some(f.message()));
                return Err(DISCONNECTED.into());
            }
            return Err(f.message());
        }
        Ok(out)
    }

    /// Uscita dall'app: `ssh -O exit` per ogni connessione (in parallelo), prima di fermare i processi.
    pub fn close_all(&self) {
        let servers: Vec<Server> = crate::lock(&self.entries)
            .values()
            .filter(|e| matches!(e.state, ConnState::Connected | ConnState::Connecting))
            .map(|e| e.server.clone())
            .collect();
        let paths = self.paths.clone();
        let handles: Vec<_> = servers
            .into_iter()
            .map(|s| {
                let paths = paths.clone();
                std::thread::spawn(move || {
                    let mut c = command(SSH);
                    c.args(control_args(&s, &paths, "exit"));
                    let _ = tools::run_tool(c, Duration::from_secs(3), None);
                })
            })
            .collect();
        for h in handles {
            let _ = h.join();
        }
    }
}

enum ConnectError {
    Cancelled,
    Failed(Failure),
    Io(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn saved(id: &str) -> Server {
        Server {
            id: id.into(),
            name: "Dev".into(),
            host: "example.test".into(),
            user: Some("dev".into()),
            port: Some(2222),
            identity_file: Some("/home/dev/.ssh/id_dev".into()),
            start_dir: None,
            source: ServerSource::Saved,
            has_password: false,
        }
    }

    fn config(alias: &str) -> Server {
        Server {
            id: format!("config:{alias}"),
            name: alias.into(),
            host: "w.example.test".into(),
            user: Some("ignorato".into()),
            port: Some(2200),
            identity_file: None,
            start_dir: None,
            source: ServerSource::Config,
            has_password: false,
        }
    }

    fn paths(data: &str) -> SshPaths {
        SshPaths { data_dir: data.into(), home: "/home/dev".into(), tmp_dir: "/var/folders/xy/T".into(), uid: 501 }
    }

    #[test]
    fn short_hash_is_stable_fnv1a() {
        assert_eq!(short_hash(""), "811c9dc5");
        assert_eq!(short_hash("a"), "e40c292c");
        assert_eq!(short_hash("config:web"), short_hash("config:web"));
        assert_ne!(short_hash("config:web"), short_hash("config:db"));
        assert_eq!(short_hash("x").len(), 8);
    }

    #[test]
    fn socket_short_then_tmpdir_then_tmp() {
        let p = paths("/home/dev/Library/Application Support/CodeArt");
        let h = short_hash("id-1");
        assert_eq!(p.socket("id-1"), PathBuf::from(format!("/home/dev/Library/Application Support/CodeArt/s/{h}")));
        // 100 byte compreso il nome temporaneo di OpenSSH (`.` + 16 caratteri)
        let fits = "/d".repeat((100 - 17 - "/s/".len() - 8) / 2);
        assert_eq!(fits.len() + 3 + 8 + 17, 100);
        assert_eq!(paths(&fits).socket("id-1"), PathBuf::from(format!("{fits}/s/{h}")));
        let long = format!("{fits}x");
        assert_eq!(paths(&long).socket("id-1"), PathBuf::from(format!("/var/folders/xy/T/codeart-501/{h}")));
        let mut both = paths(&long);
        both.tmp_dir = PathBuf::from(&long);
        assert_eq!(both.socket("id-1"), PathBuf::from(format!("/tmp/codeart-501/{h}")));
    }

    #[test]
    fn base_args_saved_and_config() {
        let p = paths("/home/dev/Library/Application Support/CodeArt");
        let sock = p.socket("id-1");
        let common = |sock: &Path| -> Vec<String> {
            [
                "-o",
                &format!("ControlPath=\"{}\"", sock.display()),
                "-o",
                "ControlMaster=no",
                "-o",
                "StrictHostKeyChecking=yes",
                "-o",
                "UserKnownHostsFile=\"~/.ssh/known_hosts\" \"/home/dev/Library/Application Support/CodeArt/known_hosts\"",
                "-o",
                "BatchMode=no",
                "-o",
                "ServerAliveInterval=15",
                "-o",
                "ServerAliveCountMax=3",
            ]
            .iter()
            .map(|s| s.to_string())
            .collect()
        };
        let mut want = common(&sock);
        want.extend(["-p", "2222", "-l", "dev", "-i", "/home/dev/.ssh/id_dev", "example.test"].map(String::from));
        assert_eq!(base_args(&saved("id-1"), &p), want);
        // config: solo l'alias, il resto lo applica OpenSSH dalla config
        let c = config("web");
        let mut want = common(&p.socket("config:web"));
        want.push("web".into());
        assert_eq!(base_args(&c, &p), want);
        // saved senza campi facoltativi
        let mut bare = saved("id-2");
        (bare.user, bare.port, bare.identity_file) = (None, None, Some(String::new()));
        assert_eq!(base_args(&bare, &p).last().map(String::as_str), Some("example.test"));
        assert!(!base_args(&bare, &p).iter().any(|a| a == "-p" || a == "-l" || a == "-i"));
    }

    #[test]
    fn master_control_and_run_args() {
        let p = paths("/home/dev/data");
        let s = saved("id-1");
        let m = master_args(&s, &p);
        assert!(m.contains(&"ControlMaster=yes".to_string()) && !m.contains(&"ControlMaster=no".to_string()));
        assert_eq!(&m[m.len() - 7..], ["-M", "-N", "-o", "ControlPersist=600", "-o", "ConnectTimeout=10", "example.test"].map(String::from));
        assert_eq!(m[m.len() - 8], "/home/dev/.ssh/id_dev");
        let c = control_args(&s, &p, "check");
        assert_eq!(&c[c.len() - 3..], ["-O", "check", "example.test"].map(String::from));
        let r = run_args(&s, &p, "cat -- \"$1\"", &["/home/dev/a b"]);
        assert_eq!(&r[r.len() - 5..r.len() - 1], ["-T", "-o", "ClearAllForwardings=yes", "example.test"].map(String::from));
        assert_eq!(r.last().unwrap(), "sh -c 'cat -- \"$1\"' _ '/home/dev/a b'");
    }

    /// Esegue `remote_cmd` come farebbe la shell del server (`/bin/sh -c <riga>`) e restituisce l'argv visto dallo
    /// script.
    fn remote_argv(args: &[&str]) -> Vec<String> {
        let line = remote_cmd(r#"printf '%s\0' "$@""#, args);
        let out = Command::new("/bin/sh").arg("-c").arg(&line).output().unwrap();
        assert!(out.status.success(), "{line}: {}", String::from_utf8_lossy(&out.stderr));
        let mut v: Vec<String> = out.stdout.split(|b| *b == 0).map(|b| String::from_utf8(b.to_vec()).unwrap()).collect();
        assert_eq!(v.pop().as_deref(), Some(""), "manca il separatore finale");
        v
    }

    #[test]
    fn quote_sh_survives_hostile_names() {
        let hostile = [
            "/home/dev/con spazi/file.txt",
            "it's",
            "'",
            "''",
            "\"doppi\"",
            "$(id)",
            "${HOME}",
            "a;rm -rf /home/dev/x",
            "`id`",
            "riga\nnuova",
            "-rf",
            "--",
            "àèìòù — 日本語 🚀",
            "back\\slash\\",
            "*",
            "~",
            "a|b&c>d<e",
            "tab\there",
            "!x",
        ];
        assert_eq!(remote_argv(&hostile), hostile);
        assert_eq!(remote_argv(&[""]), [""]);
        assert_eq!(quote_sh("it's"), r"'it'\''s'");
        assert_eq!(remote_cmd("echo \"$1\"", &["a b"]), r#"sh -c 'echo "$1"' _ 'a b'"#);
    }

    /// Valore di un'opzione come lo intende OpenSSH (`ssh -G` non si connette).
    fn ssh_g_value(opt: &str, key: &str) -> String {
        let out = Command::new(SSH).args(["-G", "-F", "/dev/null", "-o", opt, "example.test"]).output().unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        let text = String::from_utf8(out.stdout).unwrap();
        text.lines().find_map(|l| l.strip_prefix(&format!("{key} "))).unwrap_or_default().to_string()
    }

    #[test]
    fn option_values_with_spaces_quotes_and_percent_reach_openssh_intact() {
        for path in ["/home/dev/Library/Application Support/CodeArt/s/0a1b2c3d", "/home/dev/100%/s/x", "/home/dev/a\"b/x"] {
            let opt = format!("ControlPath={}", cfg_value(Path::new(path)));
            assert_eq!(ssh_g_value(&opt, "controlpath"), path, "{opt}");
        }
        let p = paths("/home/dev/Library/Application Support/CodeArt");
        let args = base_args(&saved("id-1"), &p);
        let kh = args.iter().find(|a| a.starts_with("UserKnownHostsFile=")).unwrap();
        let got = ssh_g_value(kh, "userknownhostsfile");
        assert!(got.ends_with(".ssh/known_hosts /home/dev/Library/Application Support/CodeArt/known_hosts"), "{got}");
        assert!(!got.starts_with('~'), "tilde espansa da OpenSSH: {got}");
    }

    #[test]
    fn classifies_real_openssh_errors() {
        let changed = "@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@\r\n\
@    WARNING: REMOTE HOST IDENTIFICATION HAS CHANGED!     @\r\n\
@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@\r\n\
IT IS POSSIBLE THAT SOMEONE IS DOING SOMETHING NASTY!\r\n\
Someone could be eavesdropping on you right now (man-in-the-middle attack)!\r\n\
It is also possible that a host key has just been changed.\r\n\
The fingerprint for the ED25519 key sent by the remote host is\nSHA256:nablSdweLY5Fmu2WO6ZkW6syzNI6e+OTdmGqA5Dvsg8.\r\n\
Please contact your system administrator.\r\n\
Add correct host key in /home/dev/.ssh/known_hosts to get rid of this message.\r\n\
Offending ED25519 key in /home/dev/.ssh/known_hosts:3\r\n\
Host key for [example.test]:2222 has changed and you have requested strict checking.\r\n\
Host key verification failed.\r\n";
        assert_eq!(classify(changed), Failure::HostKeyChanged);
        assert_eq!(classify("Host key verification failed.\r\n"), Failure::HostKeyChanged);
        assert_eq!(
            classify("No ED25519 host key is known for example.test and you have requested strict checking.\r\nHost key verification failed.\r\n"),
            Failure::HostKeyUnknown
        );
        assert_eq!(classify("dev@example.test: Permission denied (publickey,password).\r\n"), Failure::Auth);
        assert_eq!(classify("Received disconnect from 192.0.2.1 port 22:2: Too many authentication failures\r\n"), Failure::Auth);
        assert_eq!(
            classify("ssh: Could not resolve hostname example.test: nodename nor servname provided, or not known\r\n"),
            Failure::Unreachable("Server non trovato: il nome non si risolve")
        );
        assert_eq!(classify("ssh: connect to host example.test port 22: Connection refused\r\n"), Failure::Unreachable("Connessione rifiutata dal server"));
        for t in ["ssh: connect to host example.test port 22: Operation timed out\r\n", "Connection timed out during banner exchange\r\n"] {
            assert_eq!(classify(t), Failure::Unreachable("Server non raggiungibile: tempo scaduto"), "{t}");
        }
        assert_eq!(classify("ssh: connect to host example.test port 22: No route to host\r\n"), Failure::Unreachable("Server non raggiungibile"));
        for t in [
            "Control socket connect(/home/dev/s/0a1b2c3d): No such file or directory\r\n",
            "mux_client_request_session: read from master failed: Broken pipe\r\n",
            "Connection to example.test closed by remote host.\r\n",
            "client_loop: send disconnect: Broken pipe\r\n",
            "Read from remote host example.test: Connection reset by peer\r\n",
        ] {
            assert_eq!(classify(t), Failure::Disconnected, "{t}");
        }
        assert_eq!(classify("Warning: Permanently added 'x'\nqualcosa \u{1b}[31mdi strano\n"), Failure::Other("qualcosa [31mdi strano".into()));
        assert_eq!(classify(""), Failure::Other(String::new()));
        assert_eq!(Failure::Auth.connect_error(), AUTH);
        assert!(Failure::HostKeyChanged.connect_error().contains("cambiata"));
        assert_eq!(Failure::Other(String::new()).message(), "Errore di ssh");
    }

    #[test]
    fn parses_ssh_g_including_real_config_aliases() {
        let r = parse_ssh_g("user dev\nhostname example.test\nport 22\nhostkeyalias none\n").unwrap();
        assert_eq!(r, Resolved { hostname: "example.test".into(), port: 22, host_key_alias: None });
        assert_eq!(r.lookup_name(), "example.test");
        let r = parse_ssh_g("hostname example.test\nport 2222\n").unwrap();
        assert_eq!(r.lookup_name(), "[example.test]:2222");
        let r = parse_ssh_g("hostname example.test\nport 2222\nhostkeyalias dev-box\n").unwrap();
        assert_eq!(r.lookup_name(), "dev-box");
        assert!(parse_ssh_g("port 22\n").is_err());
        assert!(parse_ssh_g("hostname example.test\n").is_err());
        assert!(parse_ssh_g("hostname -oProxyCommand=x\nport 22\n").is_err());
        // un alias vero della config, risolto da OpenSSH
        let d = tempfile::tempdir().unwrap();
        let cfg = d.path().join("config");
        std::fs::write(&cfg, "Host web\n  HostName W.Example.Test\n  Port 2200\n").unwrap();
        let out = Command::new(SSH).arg("-G").arg("-F").arg(&cfg).arg("web").output().unwrap();
        let r = parse_ssh_g(&String::from_utf8_lossy(&out.stdout)).unwrap();
        assert_eq!((r.hostname.as_str(), r.port), ("w.example.test", 2200));
        assert_eq!(r.lookup_name(), "[w.example.test]:2200");
    }

    /// Una chiave pubblica ed25519 vera (generata ora): `(tipo, base64)`.
    fn fake_key(dir: &Path) -> (String, String) {
        let k = dir.join("k");
        let st = Command::new(SSH_KEYGEN).args(["-q", "-t", "ed25519", "-N", "", "-C", "", "-f"]).arg(&k).status().unwrap();
        assert!(st.success());
        let pubkey = std::fs::read_to_string(dir.join("k.pub")).unwrap();
        let mut f = pubkey.split_whitespace();
        (f.next().unwrap().to_string(), f.next().unwrap().to_string())
    }

    #[test]
    fn known_hosts_lookup_append_and_hashed_entries() {
        let d = tempfile::tempdir().unwrap();
        let (kind, key) = fake_key(d.path());
        let kh = d.path().join("data/known_hosts");
        std::fs::create_dir_all(kh.parent().unwrap()).unwrap();
        assert!(!is_known("[example.test]:2222", std::slice::from_ref(&kh)).unwrap(), "file assente = sconosciuto");
        // file senza a capo finale: la riga nuova non si incolla alla precedente
        std::fs::write(&kh, format!("other.example.test {kind} {key}")).unwrap();
        append_known_hosts(&kh, &[format!("[example.test]:2222 {kind} {key}")]).unwrap();
        assert_eq!(std::fs::read_to_string(&kh).unwrap().lines().count(), 2);
        assert!(is_known("[example.test]:2222", std::slice::from_ref(&kh)).unwrap());
        assert!(!is_known("example.test", std::slice::from_ref(&kh)).unwrap(), "la porta conta");
        assert!(is_known("other.example.test", std::slice::from_ref(&kh)).unwrap());
        // nuovo file: creato 0600
        let fresh = d.path().join("data/kh2");
        append_known_hosts(&fresh, &[format!("example.test {kind} {key}")]).unwrap();
        assert_eq!(std::fs::metadata(&fresh).unwrap().permissions().mode() & 0o777, 0o600);
        // righe con hash (HashKnownHosts yes) trovate lo stesso; più file: basta uno
        let hashed = d.path().join("hashed");
        std::fs::write(&hashed, format!("h.example.test {kind} {key}\n")).unwrap();
        assert!(Command::new(SSH_KEYGEN).arg("-q").arg("-H").arg("-f").arg(&hashed).status().unwrap().success());
        assert!(!std::fs::read_to_string(&hashed).unwrap().contains("h.example.test"));
        assert!(is_known("h.example.test", &[fresh.clone(), hashed]).unwrap());
    }

    #[test]
    fn keyscan_lines_are_filtered_and_renamed() {
        let out = "# example.test:22 SSH-2.0-OpenSSH_9.6\n\
example.test ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIBuQ\n\
example.test ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIBuQ\n\
example.test ecdsa-sha2-nistp256 AAAAE2VjZHNh=\n\
other.example.test ssh-rsa AAAAB3Nza\n\
example.test ssh-dss AAAAB3NzaC1kc3M\n\
@cert-authority example.test ssh-ed25519 AAAA\n\
example.test ssh-rsa AAAA$(id)\n\
example.test ssh-rsa AAAA extra\n";
        assert_eq!(
            parse_keyscan(out, "example.test", "example.test"),
            ["example.test ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIBuQ", "example.test ecdsa-sha2-nistp256 AAAAE2VjZHNh="]
        );
        let out = "[example.test]:2222 ssh-ed25519 AAAAC3Nz\nexample.test ssh-ed25519 AAAAC3Nz\n";
        assert_eq!(parse_keyscan(out, "[example.test]:2222", "dev-box"), ["dev-box ssh-ed25519 AAAAC3Nz"]);
    }

    #[test]
    fn fingerprints_from_real_ssh_keygen() {
        let d = tempfile::tempdir().unwrap();
        let (kind, key) = fake_key(d.path());
        let mut c = Command::new(SSH_KEYGEN);
        c.args(["-lf", "-"]);
        let out = tools::run_tool(c, QUICK_TIMEOUT, Some(format!("[example.test]:2222 {kind} {key}\n").into_bytes())).unwrap();
        assert_eq!(out.status, 0, "{}", out.stderr);
        let fps = parse_fingerprints(&String::from_utf8_lossy(&out.stdout));
        assert_eq!(fps.len(), 1);
        assert_eq!(fps[0].0, "ED25519");
        assert!(fps[0].1.starts_with("SHA256:") && fps[0].1.len() > 40, "{:?}", fps[0]);
        assert!(parse_fingerprints("256 MD5:aa:bb x (ED25519)\n256 SHA256:abc x (ED 25519)\ngarbage\n").is_empty());
        let m = new_host_message("web", "[w.example.test]:2200", &fps);
        assert!(m.contains("«web»") && m.contains("Indirizzo: [w.example.test]:2200") && m.contains(&fps[0].1), "{m}");
        let m = new_host_message("example.test\u{202e}tset", "example.test\u{202e}tset", &fps);
        assert!(!m.contains('\u{202e}') && !m.contains("Indirizzo"), "{m}");
    }

    #[test]
    fn private_dir_is_0700_and_owned() {
        use std::os::unix::fs::MetadataExt;
        let d = tempfile::tempdir().unwrap();
        let uid = std::fs::metadata(d.path()).unwrap().uid();
        let dir = d.path().join("codeart-x/s");
        ensure_private_dir(&dir, uid).unwrap();
        assert_eq!(std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777, 0o700);
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        ensure_private_dir(&dir, uid).unwrap();
        assert_eq!(std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777, 0o700);
        let link = d.path().join("link");
        std::os::unix::fs::symlink(&dir, &link).unwrap();
        assert!(ensure_private_dir(&link, uid).is_err(), "un link simbolico non vale");
        assert!(ensure_private_dir(&dir, uid + 1).is_err(), "di un altro utente");
    }

    fn recorder() -> (Emitter, Arc<Mutex<Vec<StateEvent>>>) {
        let log = Arc::new(Mutex::new(Vec::new()));
        let l = log.clone();
        (Arc::new(move |e| l.lock().unwrap().push(e)), log)
    }

    fn temp_conns(d: &Path) -> (Arc<Connections>, Arc<Mutex<Vec<StateEvent>>>) {
        use std::os::unix::fs::MetadataExt;
        let (emit, log) = recorder();
        let uid = std::fs::metadata(d).unwrap().uid();
        let p = SshPaths { data_dir: d.join("data"), home: d.join("home"), tmp_dir: d.to_path_buf(), uid };
        (Arc::new(Connections::new(p, emit)), log)
    }

    #[test]
    fn run_and_disconnect_without_a_connection() {
        let d = tempfile::tempdir().unwrap();
        let (c, log) = temp_conns(d.path());
        assert_eq!(c.run(&saved("id-1"), "true", &[], None).unwrap_err(), DISCONNECTED);
        c.disconnect("id-1").unwrap();
        assert!(c.states().is_empty() && log.lock().unwrap().is_empty());
        let mut bad = saved("id-1");
        bad.host = "-oProxyCommand=touch".into();
        assert!(c.connect(&bad, &|_| panic!("nessun dialogo")).unwrap_err().contains("Host"));
        bad = saved("id-1");
        bad.user = Some("-x".into());
        assert!(c.run(&bad, "true", &[], None).unwrap_err().contains("Utente"));
        assert!(log.lock().unwrap().is_empty(), "nessuno stato per server non validi");
    }

    /// Flusso completo fino alla rete: `.test` non si risolve mai (nome riservato), quindi nessuna connessione vera.
    #[test]
    fn unknown_host_that_cannot_be_scanned_ends_in_error_without_dialog() {
        let d = tempfile::tempdir().unwrap();
        let (c, log) = temp_conns(d.path());
        let s = saved("id-1");
        let e = c.connect(&s, &|_| panic!("nessun dialogo senza chiavi")).unwrap_err();
        assert!(e.starts_with("Server non"), "{e}");
        let states: Vec<ConnState> = log.lock().unwrap().iter().map(|e| e.state).collect();
        assert_eq!(states, [ConnState::Connecting, ConnState::Error]);
        assert_eq!(c.state_of("id-1"), Some(ConnState::Error));
        assert!(!c.paths().app_known_hosts().exists(), "nulla aggiunto a known_hosts");
        // socket nella cartella privata dei dati
        assert_eq!(std::fs::metadata(d.path().join("data/s")).unwrap().permissions().mode() & 0o777, 0o700);
    }

    #[test]
    fn known_host_starts_the_master_and_classifies_its_failure() {
        let d = tempfile::tempdir().unwrap();
        let (c, log) = temp_conns(d.path());
        let (kind, key) = fake_key(d.path());
        std::fs::create_dir_all(d.path().join("data")).unwrap();
        append_known_hosts(&c.paths().app_known_hosts(), &[format!("[example.test]:2222 {kind} {key}")]).unwrap();
        let e = c.connect(&saved("id-1"), &|_| panic!("server già noto: nessun dialogo")).unwrap_err();
        assert_eq!(e, "Server non trovato: il nome non si risolve");
        let last = log.lock().unwrap().last().cloned().unwrap();
        assert_eq!(last, StateEvent { id: "id-1".into(), state: ConnState::Error, message: Some(e) });
        assert_eq!(serde_json::to_value(&c.states()[0]).unwrap()["state"], "error");
        let disc = serde_json::to_value(StateEvent { id: "x".into(), state: ConnState::Disconnected, message: None }).unwrap();
        assert_eq!(disc, serde_json::json!({ "id": "x", "state": "disconnected" }));
    }
}
