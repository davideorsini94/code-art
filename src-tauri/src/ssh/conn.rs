//! Connessione condivisa per server: un master `ssh` (ControlMaster) per server, socket nella cartella dati dell'app,
//! e le operazioni (`run`) come client di quel master. Sempre `/usr/bin/ssh`, `/usr/bin/ssh-keygen`,
//! `/usr/bin/ssh-keyscan` con lista di argomenti; l'unica shell è quella del server, con `remote_cmd` (script
//! costante, argomenti passati da `quote_sh`).
//!
//! Chiavi dei server: `StrictHostKeyChecking=yes` con `~/.ssh/known_hosts` e `<dati app>/known_hosts`. Server
//! sconosciuto: `ssh-keyscan`, impronte in un dialogo nativo (il chiamante fornisce `confirm`), solo con conferma la
//! chiave si aggiunge a `<dati app>/known_hosts` (mai in `~/.ssh`). Chiave cambiata: errore, nessuna conferma.
use super::{is_host_byte, validate_host, validate_target, Server, ServerSource};
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

    /// Socket del master: `<dati app>/s/<16 hex>`; se troppo lungo `$TMPDIR/codeart-<uid>/<16 hex>`, poi
    /// `/tmp/codeart-<uid>/<16 hex>` (cartelle 0700, vedi `ensure_private_dir`). L'ultimo sta sempre nel limite (al
    /// massimo 23 + 1 + 16 + 17 = 57 byte), quindi non serve un nome più corto. Due server con lo stesso socket:
    /// `connect` rifiuta il secondo.
    pub fn socket(&self, id: &str) -> PathBuf {
        let h = short_hash(id);
        let tmp = PathBuf::from(format!("/tmp/codeart-{}", self.uid)).join(&h);
        [self.data_dir.join("s"), self.tmp_dir.join(format!("codeart-{}", self.uid))]
            .into_iter()
            .map(|d| d.join(&h))
            .find(|p| socket_fits(p))
            .unwrap_or(tmp)
    }
}

fn socket_fits(p: &Path) -> bool {
    p.as_os_str().len() + SOCKET_TMP_SUFFIX <= SOCKET_MAX
}

/// Hash stabile (FNV-1a a 64 bit) dell'id del server, in 16 cifre esadecimali: non cambia tra versioni di Rust.
pub fn short_hash(id: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in id.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{h:016x}")
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

/// Destinazione e utente validi (`validate_host`, `validate_target`): l'unica difesa contro l'iniezione di opzioni.
pub fn check_server(server: &Server) -> Result<(), String> {
    validate_host("Host", &destination(server))?;
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
    args_with(server, paths, Mode::Client, &[])
}

/// Per chi sono gli argomenti. OpenSSH tiene il PRIMO valore di ogni `-o`, quindi le varianti si generano qui
/// invece di aggiungere opzioni in coda.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Client del master (`-G`, `-O`).
    Client,
    /// Il master: `ControlMaster=yes`.
    Master,
    /// Operazioni (`run`): `BatchMode=yes` e `PreferredAuthentications=none`. Un client del master non si autentica
    /// mai; se il master non c'è, OpenSSH ripiegherebbe su una connessione diretta, che così fallisce sempre.
    Run,
}

/// Argomenti per `mode`, con `extra` subito prima della destinazione.
fn args_with(server: &Server, paths: &SshPaths, mode: Mode, extra: &[&str]) -> Vec<String> {
    let mut a: Vec<String> = vec![
        "-o".into(),
        format!("ControlPath={}", cfg_value(&paths.socket(&server.id))),
        "-o".into(),
        format!("ControlMaster={}", if mode == Mode::Master { "yes" } else { "no" }),
        "-o".into(),
        "StrictHostKeyChecking=yes".into(),
        "-o".into(),
        format!("UserKnownHostsFile=\"~/.ssh/known_hosts\" {}", cfg_value(&paths.app_known_hosts())),
        "-o".into(),
        format!("BatchMode={}", if mode == Mode::Run { "yes" } else { "no" }),
        "-o".into(),
        "ServerAliveInterval=15".into(),
        "-o".into(),
        "ServerAliveCountMax=3".into(),
    ];
    if mode == Mode::Run {
        a.extend(["-o".into(), "PreferredAuthentications=none".into()]);
    }
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
    args_with(server, paths, Mode::Master, &["-M", "-N", "-o", "ControlPersist=600", "-o", "ConnectTimeout=10"])
}

/// `ssh -O <op>` sul socket del server (`check`, `exit`).
pub fn control_args(server: &Server, paths: &SshPaths, op: &str) -> Vec<String> {
    args_with(server, paths, Mode::Client, &["-O", op])
}

/// Argomenti di `run`: mai un'autenticazione (vedi `Mode::Run`), niente terminale (i byte passano intatti), niente
/// inoltri della config, poi il comando.
pub fn run_args(server: &Server, paths: &SshPaths, script: &str, args: &[&str]) -> Vec<String> {
    let mut a = args_with(server, paths, Mode::Run, &["-T", "-o", "ClearAllForwardings=yes"]);
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
    /// Il master è partito ma il socket non risponde.
    NoMaster,
    /// Nessun messaggio riconosciuto: lo stderr grezzo (banner del server compresi) non arriva mai all'interfaccia.
    Other,
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
    Failure::Other
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
            Failure::NoMaster => "Connessione condivisa non disponibile".into(),
            Failure::Other => "Connessione non riuscita (errore di ssh)".into(),
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
    /// Host che OpenSSH scrive nei prompt di password (`utente@host's password:`): `HostKeyAlias` o il nome reale.
    pub fn prompt_host(&self) -> &str {
        self.host_key_alias.as_deref().unwrap_or(&self.hostname)
    }

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
    validate_host("Host", &hostname)?;
    if let Some(a) = &alias {
        validate_host("HostKeyAlias", a)?;
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

/// Chiavi al massimo accettate da `ssh-keyscan` (una per tipo basta) e loro dimensione totale.
const MAX_KEYS: usize = 8;
const MAX_KEYS_BYTES: usize = 16 * 1024;

/// Nome valido in known_hosts: `host` o `[host]:porta`, con `host` di soli caratteri di `validate_host`. Niente
/// `*`/`?`/`!` (jolly e negazioni), `,` (liste di host), `@` iniziale (marcatori), `|` (hash).
pub fn valid_lookup_name(name: &str) -> bool {
    let host_ok = |h: &str| !h.is_empty() && !h.starts_with('-') && h.bytes().all(is_host_byte);
    match name.strip_prefix('[').and_then(|r| r.split_once("]:")) {
        Some((h, port)) => host_ok(h) && port.parse::<u16>().is_ok_and(|p| p > 0) && port.bytes().all(|b| b.is_ascii_digit()),
        None => host_ok(name),
    }
}

/// Righe di `ssh-keyscan` per `expected` (host o `[host]:porta`), riscritte come `<name> <tipo> <chiave>`: solo
/// tipi noti e chiavi in base64, nient'altro entra in known_hosts. Più di `MAX_KEYS` chiavi o di 16 KB: rifiutate.
pub fn parse_keyscan(out: &str, expected: &str, name: &str) -> Result<Vec<String>, String> {
    if !valid_lookup_name(name) {
        return Err("Nome del server non valido per known_hosts".into());
    }
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
    if lines.len() > MAX_KEYS || lines.iter().map(|l| l.len() + 1).sum::<usize>() > MAX_KEYS_BYTES {
        return Err("Il server ha inviato troppe chiavi: connessione rifiutata".into());
    }
    Ok(lines)
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
    socket: PathBuf,
}

pub type Emitter = Arc<dyn Fn(StateEvent) + Send + Sync>;

/// Esito di `ssh -O check`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Check {
    Alive,
    Dead,
    /// L'app si sta chiudendo (nessun processo parte più).
    Stopped,
}

enum ConnectError {
    Cancelled,
    Failed(Failure),
    Io(String),
}

/// I processi `ssh` dietro le connessioni: quelli veri nell'app, uno finto nei test della macchina a stati.
trait SshOps: Send + Sync {
    /// Verifica la chiave del server e avvia (o riusa) il master. `current()` dice se questa connessione è ancora
    /// quella valida (dopo il dialogo, prima di avviare il master).
    fn establish(&self, server: &Server, current: &dyn Fn() -> bool, confirm: &dyn Fn(&str) -> bool) -> Result<(), ConnectError>;
    fn check(&self, server: &Server) -> Check;
    fn exit(&self, server: &Server);
}

/// Connessioni condivise dell'app (una per server, valide per tutte le finestre).
pub struct Connections {
    paths: SshPaths,
    ops: Arc<dyn SshOps>,
    entries: Mutex<HashMap<String, Entry>>,
    next: AtomicU64,
    emit: Emitter,
    monitor_every: Duration,
}

impl Connections {
    pub fn new(paths: SshPaths, emit: Emitter) -> Self {
        let ops = Arc::new(SystemSsh { paths: paths.clone() });
        Self::with_ops(paths, ops, emit, MONITOR_EVERY)
    }

    fn with_ops(paths: SshPaths, ops: Arc<dyn SshOps>, emit: Emitter, monitor_every: Duration) -> Self {
        Connections { paths, ops, entries: Mutex::new(HashMap::new()), next: AtomicU64::new(1), emit, monitor_every }
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

    fn emit(&self, id: &str, state: ConnState, message: Option<String>) {
        (self.emit)(StateEvent { id: id.to_string(), state, message });
    }

    /// Nuova generazione per `server` nello stato `state` (chiamante con il lucchetto preso).
    fn insert_locked(&self, map: &mut HashMap<String, Entry>, server: &Server, state: ConnState) -> u64 {
        let generation = self.next.fetch_add(1, Ordering::Relaxed);
        let socket = self.paths.socket(&server.id);
        map.insert(server.id.clone(), Entry { state, message: None, generation, server: server.clone(), socket });
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
        self.emit(id, state, message);
        true
    }

    fn generation_of(&self, id: &str) -> Option<u64> {
        crate::lock(&self.entries).get(id).map(|e| e.generation)
    }

    /// Connette `server`. Già connesso o in connessione: `Ok` subito, senza un secondo dialogo né un secondo master
    /// (lo stato arriva con `ssh:state`); la decisione e l'iscrizione avvengono sotto lo stesso lucchetto.
    /// `confirm` mostra il dialogo nativo per un server nuovo e dice se l'utente l'ha accettato.
    /// Da chiamare fuori dal thread principale.
    pub fn connect(self: &Arc<Self>, server: &Server, confirm: &dyn Fn(&str) -> bool) -> Result<(), String> {
        check_server(server)?;
        let socket = self.paths.socket(&server.id);
        let generation = {
            let mut map = crate::lock(&self.entries);
            let live = |e: &Entry| matches!(e.state, ConnState::Connecting | ConnState::Connected);
            if map.get(&server.id).is_some_and(live) {
                return Ok(());
            }
            if map.iter().any(|(id, e)| id != &server.id && live(e) && e.socket == socket) {
                return Err("Un altro server connesso usa lo stesso socket: disconnettilo prima".into());
            }
            self.insert_locked(&mut map, server, ConnState::Connecting)
        };
        self.emit(&server.id, ConnState::Connecting, None);
        let current = || self.generation_of(&server.id) == Some(generation);
        match self.ops.establish(server, &current, confirm) {
            Ok(()) => {
                if self.set_if(&server.id, generation, ConnState::Connected, None) {
                    self.spawn_monitor(server.clone(), generation);
                    Ok(())
                } else {
                    // superata: il master si chiude solo se nessuna connessione più recente lo sta usando
                    let owned = crate::lock(&self.entries)
                        .get(&server.id)
                        .is_some_and(|e| matches!(e.state, ConnState::Connecting | ConnState::Connected));
                    if !owned {
                        self.ops.exit(server);
                    }
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

    /// Controllo periodico: il master che termina (rete, server, 10 minuti senza client) → `disconnected`. Servono
    /// due controlli falliti di fila: un intoppo isolato (processo lento, avvio non riuscito) non chiude la sessione.
    fn spawn_monitor(self: &Arc<Self>, server: Server, generation: u64) {
        let me = self.clone();
        std::thread::spawn(move || {
            let step = me.monitor_every.min(Duration::from_millis(200));
            let mut failures = 0;
            loop {
                let until = Instant::now() + me.monitor_every;
                while Instant::now() < until {
                    if me.generation_of(&server.id) != Some(generation) {
                        return;
                    }
                    std::thread::sleep(step);
                }
                match me.ops.check(&server) {
                    Check::Alive => failures = 0,
                    Check::Stopped => return,
                    Check::Dead => {
                        failures += 1;
                        if failures >= 2 {
                            me.set_if(&server.id, generation, ConnState::Disconnected, Some(Failure::Disconnected.message()));
                            return;
                        }
                    }
                }
            }
        });
    }

    /// Chiude la connessione condivisa di `id` (`ssh -O exit`); già chiusa = nulla da fare.
    pub fn disconnect(&self, id: &str) -> Result<(), String> {
        let server = {
            let mut map = crate::lock(&self.entries);
            let Some(server) = map.get(id).map(|e| e.server.clone()) else { return Ok(()) };
            self.insert_locked(&mut map, &server, ConnState::Disconnected);
            server
        };
        self.emit(id, ConnState::Disconnected, None);
        self.ops.exit(&server);
        Ok(())
    }

    /// Esegue `script` sul server con `args` (via `remote_cmd`) sulla connessione condivisa; serve lo stato
    /// `connected`. Mai una connessione diretta (vedi `Mode::Run`).
    pub fn run(&self, server: &Server, script: &str, args: &[&str], stdin: Option<Vec<u8>>) -> Result<ToolOutput, String> {
        self.run_with_timeout(server, script, args, stdin, RUN_TIMEOUT)
    }

    /// Come `run`. Uscita 255 con un messaggio di ssh riconosciuto E master che non risponde più = connessione persa
    /// (`disconnected`); altrimenti è l'uscita dello script e torna al chiamante così com'è.
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
        if lost_connection(out.status, &out.stderr, || self.ops.check(server)) {
            self.set_if(&server.id, generation, ConnState::Disconnected, Some(Failure::Disconnected.message()));
            return Err(DISCONNECTED.into());
        }
        Ok(out)
    }

    /// Comando `ssh` per uno script di lunga durata (trasferimenti): stessi argomenti di `run`, ma lo stream lo
    /// gestisce il chiamante (stdin/stdout a pezzi). Serve lo stato `connected`; mai una connessione diretta.
    pub fn stream_command(&self, server: &Server, script: &str, args: &[&str]) -> Result<Command, String> {
        check_server(server)?;
        let connected = matches!(crate::lock(&self.entries).get(&server.id), Some(e) if e.state == ConnState::Connected);
        if !connected || !self.paths.socket(&server.id).exists() {
            return Err(DISCONNECTED.into());
        }
        let mut c = command(SSH);
        c.args(run_args(server, &self.paths, script, args));
        Ok(c)
    }

    /// Uscita dall'app: `ssh -O exit` per ogni connessione (in parallelo), prima di fermare i processi.
    pub fn close_all(&self) {
        let servers: Vec<Server> = crate::lock(&self.entries)
            .values()
            .filter(|e| matches!(e.state, ConnState::Connected | ConnState::Connecting))
            .map(|e| e.server.clone())
            .collect();
        let handles: Vec<_> = servers
            .into_iter()
            .map(|s| {
                let ops = self.ops.clone();
                std::thread::spawn(move || ops.exit(&s))
            })
            .collect();
        for h in handles {
            let _ = h.join();
        }
    }
}

/// Uscita di `run` dovuta alla connessione persa? Solo con 255, un messaggio di ssh riconosciuto E il master che non
/// risponde più (`check` si chiama solo in quel caso); altrimenti è l'uscita dello script remoto.
fn lost_connection(status: i32, stderr: &str, check: impl FnOnce() -> Check) -> bool {
    status == 255 && classify(stderr) != Failure::Other && check() == Check::Dead
}

/// I processi veri: `/usr/bin/ssh`, `ssh-keygen`, `ssh-keyscan`.
struct SystemSsh {
    paths: SshPaths,
}

impl SshOps for SystemSsh {
    fn establish(&self, server: &Server, current: &dyn Fn() -> bool, confirm: &dyn Fn(&str) -> bool) -> Result<(), ConnectError> {
        let sock = self.paths.socket(&server.id);
        if let Some(dir) = sock.parent() {
            ensure_private_dir(dir, self.paths.uid).map_err(ConnectError::Io)?;
        }
        ensure_private_dir(&self.paths.data_dir, self.paths.uid).map_err(ConnectError::Io)?;
        // un master ancora vivo (avvio precedente dell'app terminato male) si riusa
        if self.check(server) == Check::Alive {
            return Ok(());
        }
        // socket rimasto da un master morto: OpenSSH altrimenti disattiverebbe la condivisione
        let _ = std::fs::remove_file(&sock);
        let resolved = self.verify_host_key(server, confirm)?;
        if !current() {
            return Err(ConnectError::Cancelled);
        }
        self.start_master(server, &resolved)
    }

    fn check(&self, server: &Server) -> Check {
        match self.control(server, "check") {
            Ok(o) if o.status == 0 => Check::Alive,
            Err(e) if e == tools::STOPPED => Check::Stopped,
            _ => Check::Dead,
        }
    }

    fn exit(&self, server: &Server) {
        let mut c = command(SSH);
        c.args(control_args(server, &self.paths, "exit"));
        let _ = tools::run_tool(c, Duration::from_secs(3), None);
    }
}

impl SystemSsh {
    /// `ssh -O <op>` sul socket del server.
    fn control(&self, server: &Server, op: &str) -> Result<ToolOutput, String> {
        let mut c = command(SSH);
        c.args(control_args(server, &self.paths, op));
        tools::run_tool(c, QUICK_TIMEOUT, None)
    }

    /// Server già in un known_hosts: nulla da fare. Sconosciuto: `ssh-keyscan`, impronte, conferma, aggiunta a
    /// `<dati app>/known_hosts`. Una chiave cambiata la scopre poi il master (errore, nessuna conferma).
    /// Restituisce l'indirizzo reale (per askpass).
    fn verify_host_key(&self, server: &Server, confirm: &dyn Fn(&str) -> bool) -> Result<Resolved, ConnectError> {
        let mut c = command(SSH);
        c.arg("-G").args(base_args(server, &self.paths));
        let out = tools::run_tool(c, QUICK_TIMEOUT, None).map_err(ConnectError::Io)?;
        if out.status != 0 {
            return Err(ConnectError::Failed(classify(&out.stderr)));
        }
        let r = parse_ssh_g(&String::from_utf8_lossy(&out.stdout)).map_err(ConnectError::Io)?;
        let name = r.lookup_name();
        if !valid_lookup_name(&name) {
            return Err(ConnectError::Io("Nome del server non valido per known_hosts".into()));
        }
        if is_known(&name, &known_hosts_files(&self.paths)).map_err(ConnectError::Io)? {
            return Ok(r);
        }
        let mut c = command(SSH_KEYSCAN);
        c.args(["-T", "5", "-p", &r.port.to_string(), "--", &r.hostname]);
        let out = tools::run_tool(c, Duration::from_secs(15), None).map_err(ConnectError::Io)?;
        let lines = parse_keyscan(&String::from_utf8_lossy(&out.stdout), &scan_name(&r.hostname, r.port), &name).map_err(ConnectError::Io)?;
        if lines.is_empty() {
            let f = match classify(&out.stderr) {
                Failure::Other => Failure::Unreachable("Server non raggiungibile: nessuna chiave ricevuta"),
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
        append_known_hosts(&self.paths.app_known_hosts(), &lines).map_err(ConnectError::Io)?;
        Ok(r)
    }

    /// Avvia il master e aspetta che il socket risponda. Con `-N` e `ControlPersist` OpenSSH passa in sottofondo
    /// dopo l'autenticazione (il processo avviato esce 0); se restasse in primo piano basta il socket pronto.
    fn start_master(&self, server: &Server, resolved: &Resolved) -> Result<(), ConnectError> {
        use crate::claude::locate::{in_new_session, stop_group, TERM_GRACE};
        use std::process::Stdio;
        let ticket = tools::RUNNING.register().map_err(ConnectError::Io)?;
        let exe = std::env::current_exe().map_err(|e| ConnectError::Io(e.to_string()))?;
        let mut cmd = command(SSH);
        cmd.args(master_args(server, &self.paths))
            .env("SSH_ASKPASS", exe)
            .env("SSH_ASKPASS_REQUIRE", "force")
            .env(super::askpass::ACCOUNT_ENV, &server.id)
            // askpass risponde solo a un prompt che nomina questo host (non a un salto ProxyJump)
            .env(super::askpass::HOST_ENV, resolved.prompt_host())
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
                if self.check(server) == Check::Alive {
                    // in primo piano: il processo resta vivo, il controllo periodico ne segue la fine
                    return Ok(());
                }
            }
            // uscito 0 ma il socket non risponde: errore dopo qualche secondo
            if exited_ok_at.is_some_and(|t| t.elapsed() > Duration::from_secs(5)) {
                return Err(ConnectError::Failed(match classify(&stderr()) {
                    Failure::Other => Failure::NoMaster,
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
        assert_eq!(short_hash(""), "cbf29ce484222325");
        assert_eq!(short_hash("a"), "af63dc4c8601ec8c");
        assert_eq!(short_hash("config:web"), short_hash("config:web"));
        assert_ne!(short_hash("config:web"), short_hash("config:db"));
        assert_eq!(short_hash("x").len(), 16);
    }

    #[test]
    fn socket_short_then_tmpdir_then_tmp() {
        let p = paths("/home/dev/Library/Application Support/CodeArt");
        let h = short_hash("id-1");
        assert_eq!(p.socket("id-1"), PathBuf::from(format!("/home/dev/Library/Application Support/CodeArt/s/{h}")));
        // 100 byte compreso il nome temporaneo di OpenSSH (`.` + 16 caratteri)
        let fits = "/d".repeat((100 - 17 - "/s/".len() - 16) / 2);
        assert_eq!(fits.len() + 3 + 16 + 17, 100);
        assert_eq!(paths(&fits).socket("id-1"), PathBuf::from(format!("{fits}/s/{h}")));
        let long = format!("{fits}x");
        assert_eq!(paths(&long).socket("id-1"), PathBuf::from(format!("/var/folders/xy/T/codeart-501/{h}")));
        let mut both = paths(&long);
        both.tmp_dir = PathBuf::from(&long);
        assert_eq!(both.socket("id-1"), PathBuf::from(format!("/tmp/codeart-501/{h}")));
        // il caso peggiore (uid a 10 cifre) sta comunque nel limite
        let mut worst = both.clone();
        worst.uid = u32::MAX;
        assert!(socket_fits(&worst.socket("id-1")));
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
        // run non si autentica mai (niente ripiego su una connessione diretta); il master e i client sì
        assert!(r.contains(&"BatchMode=yes".to_string()) && !r.contains(&"BatchMode=no".to_string()));
        let pa = r.iter().position(|a| a == "PreferredAuthentications=none").expect("PreferredAuthentications=none");
        assert_eq!(r[pa - 1], "-o");
        assert!(pa < r.len() - 2, "prima della destinazione");
        for other in [&m, &c, &base_args(&s, &p)] {
            assert!(other.contains(&"BatchMode=no".to_string()));
            assert!(!other.iter().any(|a| a.starts_with("PreferredAuthentications")));
        }
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
        // lo stderr grezzo (banner del server compreso) non arriva mai all'interfaccia
        let banner = "Benvenuto su example.test! Visita http://evil.test\n";
        assert_eq!(classify(banner), Failure::Other);
        assert_eq!(classify(""), Failure::Other);
        assert!(!Failure::Other.message().contains("evil"));
        assert_eq!(Failure::Auth.connect_error(), AUTH);
        assert!(Failure::HostKeyChanged.connect_error().contains("cambiata"));
        assert_eq!(Failure::Other.message(), "Connessione non riuscita (errore di ssh)");
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
        for bad in ["*", "a,b.example.test", "dev@example.test", "a/b", "!x", "bücher.test"] {
            assert!(parse_ssh_g(&format!("hostname {bad}\nport 22\n")).is_err(), "{bad}");
            assert!(parse_ssh_g(&format!("hostname example.test\nport 22\nhostkeyalias {bad}\n")).is_err(), "{bad}");
        }
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
            parse_keyscan(out, "example.test", "example.test").unwrap(),
            ["example.test ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIBuQ", "example.test ecdsa-sha2-nistp256 AAAAE2VjZHNh="]
        );
        let out = "[example.test]:2222 ssh-ed25519 AAAAC3Nz\nexample.test ssh-ed25519 AAAAC3Nz\n";
        assert_eq!(parse_keyscan(out, "[example.test]:2222", "dev-box").unwrap(), ["dev-box ssh-ed25519 AAAAC3Nz"]);
        // nomi che in known_hosts sarebbero jolly, liste, marcatori o hash
        for bad in ["*", "a,b", "@revoked", "|1|abc", "!x", "-x", "[a]:0", "[a]:x", "[a,b]:22", "", "a b"] {
            assert!(!valid_lookup_name(bad), "{bad:?}");
            assert!(parse_keyscan(out, "[example.test]:2222", bad).is_err(), "{bad:?}");
        }
        for ok in ["example.test", "[example.test]:2222", "dev-box", "fe80::1", "[fe80::1]:22"] {
            assert!(valid_lookup_name(ok), "{ok}");
        }
        // al massimo 8 chiavi e 16 KB: oltre, rifiutate
        let many: String = (0..9).map(|i| format!("example.test ssh-ed25519 AAAA{i}\n")).collect();
        assert!(parse_keyscan(&many, "example.test", "example.test").unwrap_err().contains("troppe chiavi"));
        let eight: String = (0..8).map(|i| format!("example.test ssh-ed25519 AAAA{i}\n")).collect();
        assert_eq!(parse_keyscan(&eight, "example.test", "example.test").unwrap().len(), 8);
        let big: String = (0..2).map(|i| format!("example.test ssh-rsa {}{i}\n", "A".repeat(9000))).collect();
        assert!(parse_keyscan(&big, "example.test", "example.test").is_err());
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

    /// `ssh` finto per la macchina a stati: la prima `establish` può restare ferma finché il test non la sblocca.
    #[derive(Default)]
    struct FakeSsh {
        establish_calls: std::sync::atomic::AtomicUsize,
        exits: std::sync::atomic::AtomicUsize,
        checks_done: std::sync::atomic::AtomicUsize,
        entered: Mutex<Option<std::sync::mpsc::Sender<()>>>,
        gate: Mutex<Option<std::sync::mpsc::Receiver<()>>>,
        result: Mutex<Option<ConnectError>>,
        checks: Mutex<std::collections::VecDeque<Check>>,
    }

    impl SshOps for FakeSsh {
        fn establish(&self, _: &Server, _: &dyn Fn() -> bool, _: &dyn Fn(&str) -> bool) -> Result<(), ConnectError> {
            self.establish_calls.fetch_add(1, Ordering::SeqCst);
            let entered = self.entered.lock().unwrap().take();
            if let Some(tx) = entered {
                tx.send(()).unwrap();
            }
            let gate = self.gate.lock().unwrap().take();
            if let Some(rx) = gate {
                rx.recv().unwrap();
            }
            match self.result.lock().unwrap().take() {
                Some(e) => Err(e),
                None => Ok(()),
            }
        }
        fn check(&self, _: &Server) -> Check {
            self.checks_done.fetch_add(1, Ordering::SeqCst);
            self.checks.lock().unwrap().pop_front().unwrap_or(Check::Alive)
        }
        fn exit(&self, _: &Server) {
            self.exits.fetch_add(1, Ordering::SeqCst);
        }
    }

    /// Connessioni con il finto; `gated`: la prima `establish` si ferma (restituisce il segnale di ingresso e lo sblocco).
    #[allow(clippy::type_complexity)]
    fn fake_conns(
        gated: bool,
        monitor_every: Duration,
    ) -> (Arc<Connections>, Arc<FakeSsh>, Arc<Mutex<Vec<StateEvent>>>, Option<(std::sync::mpsc::Receiver<()>, std::sync::mpsc::Sender<()>)>) {
        let fake = Arc::new(FakeSsh::default());
        let ctl = gated.then(|| {
            let (etx, erx) = std::sync::mpsc::channel();
            let (gtx, grx) = std::sync::mpsc::channel();
            *fake.entered.lock().unwrap() = Some(etx);
            *fake.gate.lock().unwrap() = Some(grx);
            (erx, gtx)
        });
        let (emit, log) = recorder();
        let c = Arc::new(Connections::with_ops(paths("/home/dev/data"), fake.clone(), emit, monitor_every));
        (c, fake, log, ctl)
    }

    fn states_of(log: &Arc<Mutex<Vec<StateEvent>>>) -> Vec<ConnState> {
        log.lock().unwrap().iter().map(|e| e.state).collect()
    }

    const SLOW: Duration = Duration::from_secs(600);

    #[test]
    fn second_connect_while_connecting_is_a_no_op() {
        let (c, fake, log, ctl) = fake_conns(true, SLOW);
        let (entered, release) = ctl.unwrap();
        let s = saved("id-1");
        let (c2, s2) = (c.clone(), s.clone());
        let first = std::thread::spawn(move || c2.connect(&s2, &|_| true));
        entered.recv().unwrap();
        // doppio clic o seconda finestra: nessun secondo dialogo né secondo master
        assert_eq!(c.connect(&s, &|_| panic!("nessun secondo dialogo")), Ok(()));
        assert_eq!(fake.establish_calls.load(Ordering::SeqCst), 1);
        release.send(()).unwrap();
        assert_eq!(first.join().unwrap(), Ok(()));
        assert_eq!(c.connect(&s, &|_| panic!("già connesso")), Ok(()));
        assert_eq!(fake.establish_calls.load(Ordering::SeqCst), 1);
        assert_eq!(states_of(&log), [ConnState::Connecting, ConnState::Connected]);
        assert_eq!(fake.exits.load(Ordering::SeqCst), 0);
        c.disconnect("id-1").unwrap();
    }

    #[test]
    fn superseded_connect_closes_its_master_only_if_nobody_owns_it() {
        // disconnesso durante la connessione: il master appena nato si chiude
        let (c, fake, log, ctl) = fake_conns(true, SLOW);
        let (entered, release) = ctl.unwrap();
        let s = saved("id-1");
        let (c2, s2) = (c.clone(), s.clone());
        let first = std::thread::spawn(move || c2.connect(&s2, &|_| true));
        entered.recv().unwrap();
        c.disconnect("id-1").unwrap();
        assert_eq!(fake.exits.load(Ordering::SeqCst), 1);
        release.send(()).unwrap();
        assert_eq!(first.join().unwrap().unwrap_err(), CANCELLED);
        assert_eq!(fake.exits.load(Ordering::SeqCst), 2);
        assert_eq!(c.state_of("id-1"), Some(ConnState::Disconnected));
        assert_eq!(states_of(&log), [ConnState::Connecting, ConnState::Disconnected]);

        // disconnesso e poi riconnesso da un'altra chiamata: il master è di quella, non si tocca
        let (c, fake, _log, ctl) = fake_conns(true, SLOW);
        let (entered, release) = ctl.unwrap();
        let (c2, s2) = (c.clone(), s.clone());
        let first = std::thread::spawn(move || c2.connect(&s2, &|_| true));
        entered.recv().unwrap();
        c.disconnect("id-1").unwrap();
        assert_eq!(c.connect(&s, &|_| true), Ok(()));
        release.send(()).unwrap();
        assert_eq!(first.join().unwrap().unwrap_err(), CANCELLED);
        assert_eq!(fake.exits.load(Ordering::SeqCst), 1, "solo il -O exit della disconnessione");
        assert_eq!(c.state_of("id-1"), Some(ConnState::Connected));
        c.disconnect("id-1").unwrap();
    }

    #[test]
    fn connect_failures_map_to_states_and_errors() {
        let (c, fake, log, _) = fake_conns(false, SLOW);
        *fake.result.lock().unwrap() = Some(ConnectError::Failed(Failure::Auth));
        assert_eq!(c.connect(&saved("id-1"), &|_| true).unwrap_err(), AUTH);
        assert_eq!(c.states(), [StateEvent { id: "id-1".into(), state: ConnState::Error, message: Some("Autenticazione non riuscita".into()) }]);
        *fake.result.lock().unwrap() = Some(ConnectError::Cancelled);
        assert_eq!(c.connect(&saved("id-1"), &|_| true).unwrap_err(), CANCELLED);
        assert_eq!(c.state_of("id-1"), Some(ConnState::Disconnected));
        assert_eq!(states_of(&log), [ConnState::Connecting, ConnState::Error, ConnState::Connecting, ConnState::Disconnected]);
    }

    #[test]
    fn two_servers_sharing_a_socket_are_refused() {
        let (c, fake, _log, _) = fake_conns(false, SLOW);
        let s = saved("id-1");
        // un altro server connesso con lo stesso socket (collisione dell'hash)
        crate::lock(&c.entries).insert(
            "altro".into(),
            Entry { state: ConnState::Connected, message: None, generation: 0, server: saved("altro"), socket: c.paths().socket("id-1") },
        );
        assert!(c.connect(&s, &|_| true).unwrap_err().contains("stesso socket"));
        assert_eq!(fake.establish_calls.load(Ordering::SeqCst), 0);
        assert_eq!(c.state_of("id-1"), None);
        // se l'altro non è più connesso, nessun conflitto
        crate::lock(&c.entries).get_mut("altro").unwrap().state = ConnState::Disconnected;
        assert_eq!(c.connect(&s, &|_| true), Ok(()));
        c.disconnect("id-1").unwrap();
    }

    #[test]
    fn monitor_needs_two_failed_checks_in_a_row() {
        let (c, fake, log, _) = fake_conns(false, Duration::from_millis(10));
        fake.checks.lock().unwrap().extend([Check::Dead, Check::Alive, Check::Dead, Check::Dead]);
        assert_eq!(c.connect(&saved("id-1"), &|_| true), Ok(()));
        let t0 = Instant::now();
        while c.state_of("id-1") != Some(ConnState::Disconnected) {
            assert!(t0.elapsed() < Duration::from_secs(5), "mai disconnesso");
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(fake.checks_done.load(Ordering::SeqCst), 4, "disconnesso solo al secondo fallimento di fila");
        assert_eq!(states_of(&log), [ConnState::Connecting, ConnState::Connected, ConnState::Disconnected]);
        // l'app che si chiude ferma il controllo senza cambiare stato
        let (c, fake, log, _) = fake_conns(false, Duration::from_millis(10));
        fake.checks.lock().unwrap().extend([Check::Stopped]);
        assert_eq!(c.connect(&saved("id-1"), &|_| true), Ok(()));
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(fake.checks_done.load(Ordering::SeqCst), 1);
        assert_eq!(states_of(&log), [ConnState::Connecting, ConnState::Connected]);
    }

    #[test]
    fn run_blames_the_connection_only_when_ssh_says_so_and_the_master_is_gone() {
        let closed = "Connection to example.test closed by remote host.\r\n";
        assert!(lost_connection(255, closed, || Check::Dead));
        assert!(lost_connection(255, "Control socket connect(/home/dev/s/x): No such file or directory\n", || Check::Dead));
        // master vivo: lo script ha stampato qualcosa che somiglia a ssh ed è uscito 255
        assert!(!lost_connection(255, closed, || Check::Alive));
        assert!(!lost_connection(255, "cat: /home/dev/x: Permission denied\n", || Check::Alive));
        // nessun messaggio di ssh: niente controllo, è l'uscita dello script
        assert!(!lost_connection(255, "errore dello script\n", || panic!("nessun controllo")));
        assert!(!lost_connection(1, closed, || panic!("nessun controllo")));
        assert!(!lost_connection(255, closed, || Check::Stopped));
    }
}
