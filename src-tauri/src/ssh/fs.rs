//! File sul server via SSH. Ogni operazione è un piccolo script POSIX costante, eseguito con `sh -c` dalla connessione
//! condivisa (`conn::remote_cmd`); i percorsi arrivano solo come argomenti (`$1`, `$2`), mai nel testo dello script.
//! La logica prende un `Runner` (script, argomenti, stdin → uscita): nell'app è la connessione, nei test `/bin/sh` locale.
use crate::fs_tree::FsError;
use crate::tools::ToolOutput;
use serde::Serialize;

/// Tetto dei file letti (come `fs_read`).
pub const MAX_READ_BYTES: u64 = 5 * 1024 * 1024;
/// Voci di una cartella oltre le quali l'elenco è troncato.
pub const MAX_ENTRIES: usize = 10_000;
const MAX_PATH_BYTES: usize = 4096;
pub const DELETE_TITLE: &str = "Elimina sul server";
pub const BTN_DELETE: &str = "Elimina";

// Codici di uscita degli script.
const EXIT_CONFLICT: i32 = 3;
const EXIT_EXISTS: i32 = 4;
const EXIT_NOT_FOUND: i32 = 6;
const EXIT_TOO_LARGE: i32 = 7;
const EXIT_WRITE: i32 = 8;
const EXIT_PROTECTED: i32 = 9;

// Ogni script è una riga sola (istruzioni separate da `;`) e non contiene mai `!`: la shell di login del server legge
// `sh -c '<script>'` prima di `sh`, e csh/tcsh rifiutano gli a capo dentro gli apici singoli ed espandono `!`
// (cronologia) anche lì. Lo controlla un test su tutti gli script.

/// `size:mtime` di `$1` (segue i link) in `$s`: `stat` GNU, poi BSD, poi `wc -c` con mtime 0.
macro_rules! ver_fn {
    () => {
        "ver() { if s=$(stat -L -c '%s:%Y' -- \"$1\" 2>/dev/null); then return 0; fi; \
         if s=$(stat -L -f '%z:%m' -- \"$1\" 2>/dev/null); then return 0; fi; \
         [ -e \"$1\" ] || return 1; s=\"$(wc -c < \"$1\" | tr -d ' '):0\"; }; "
    };
}

pub const HOME_SCRIPT: &str = "printf %s \"$HOME\"";

/// Voci di `$1` come `tipo\tnome\0`: `d` cartella, `f` altro, `l` link, `L` link a una cartella. Dotfile compresi
/// (`./.*` senza `.` e `..`), nessun `ls`. Si ferma a `MAX_ENTRIES + 1` voci.
pub const LIST_SCRIPT: &str = "cd -- \"$1\" || exit 5; i=0; \
for f in ./* ./.*; do \
case ${f#./} in .|..) continue;; esac; \
[ -e \"$f\" ] || [ -L \"$f\" ] || continue; \
i=$((i+1)); \
[ \"$i\" -gt 10001 ] && break; \
if [ -L \"$f\" ]; then if [ -d \"$f\" ]; then t=L; else t=l; fi; \
elif [ -d \"$f\" ]; then t=d; else t=f; fi; \
printf '%s\\t%s\\0' \"$t\" \"${f#./}\"; \
done";

pub const STAT_SCRIPT: &str = concat!(ver_fn!(), "ver \"$1\" || exit 6; printf %s \"$s\"");

/// `size:mtime`, a capo, contenuto; rifiuta oltre 5 MB (uscita 7).
pub const READ_SCRIPT: &str = concat!(
    ver_fn!(),
    "ver \"$1\" || exit 6; [ -d \"$1\" ] && exit 10; [ \"${s%%:*}\" -gt 5242880 ] && exit 7; printf '%s\\n' \"$s\"; cat -- \"$1\""
);

/// `$1` percorso, `$2` versione attesa (vuota = il file non deve esistere); contenuto da stdin. Versione diversa =
/// uscita 3. Scrive in `$1.codeart-tmp` e `mv -f` (un link si scrive attraverso); stampa la nuova versione. Il file
/// temporaneo si crea solo se non esiste (né come link: uscita 4) e in modo esclusivo (`set -C`), così in una
/// cartella condivisa nessuno può farlo puntare altrove; prende i permessi del file originale (`stat`, se c'è).
pub const WRITE_SCRIPT: &str = concat!(
    ver_fn!(),
    "tmp=\"$1.codeart-tmp\"; cur=\"\"; \
if [ -e \"$1\" ]; then ver \"$1\" || exit 6; cur=$s; fi; \
[ \"$cur\" = \"$2\" ] || exit 3; \
if [ -L \"$1\" ]; then cat > \"$1\" || exit 8; \
else \
if [ -e \"$tmp\" ] || [ -L \"$tmp\" ]; then exit 4; fi; \
m=\"\"; if [ -e \"$1\" ]; then m=$(stat -L -c %a -- \"$1\" 2>/dev/null || stat -L -f %Lp -- \"$1\" 2>/dev/null); fi; \
set -C; cat > \"$tmp\" || { rm -f -- \"$tmp\"; exit 8; }; set +C; \
case $m in [0-7][0-7][0-7]|[0-7][0-7][0-7][0-7]) chmod \"$m\" \"$tmp\" 2>/dev/null;; esac; \
mv -f -- \"$tmp\" \"$1\" || { rm -f -- \"$tmp\"; exit 8; }; \
fi; \
ver \"$1\" || exit 6; printf %s \"$s\""
);

pub const MKDIR_SCRIPT: &str = "mkdir -- \"$1\"";

/// Mai sopra una destinazione che esiste (anche un link rotto).
pub const RENAME_SCRIPT: &str = "if [ -e \"$2\" ] || [ -L \"$2\" ]; then exit 4; fi; mv -- \"$1\" \"$2\"";

/// Tipo di `$1` per la conferma: `d`, `l`, `f`. Rifiuta `/` e la home.
pub const KIND_SCRIPT: &str = "[ \"$1\" = / ] && exit 9; [ \"$1\" = \"$HOME\" ] && exit 9; \
if [ -L \"$1\" ]; then printf l; elif [ -d \"$1\" ]; then printf d; elif [ -e \"$1\" ]; then printf f; else exit 6; fi";

pub const DELETE_SCRIPT: &str = "set -e; [ \"$1\" = / ] && exit 9; [ \"$1\" = \"$HOME\" ] && exit 9; rm -rf -- \"$1\"";

/// Tutti gli script di questo modulo (per i test delle shell).
#[cfg(test)]
pub const ALL_SCRIPTS: &[&str] =
    &[HOME_SCRIPT, LIST_SCRIPT, STAT_SCRIPT, READ_SCRIPT, WRITE_SCRIPT, MKDIR_SCRIPT, RENAME_SCRIPT, KIND_SCRIPT, DELETE_SCRIPT];

pub type Runner<'a> = &'a dyn Fn(&str, &[&str], Option<Vec<u8>>) -> Result<ToolOutput, String>;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub is_link: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteListing {
    pub entries: Vec<RemoteEntry>,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteContent {
    pub text: String,
    pub version: String,
}

fn err(code: &'static str, message: impl Into<String>) -> FsError {
    FsError::new(code, message)
}

/// Percorso assoluto dopo la risoluzione di `~`, senza NUL, al massimo 4096 byte. Non si toglie `..`.
pub fn validate_path(p: &str) -> Result<(), FsError> {
    if p.contains('\0') {
        return Err(err("io", "percorso non valido"));
    }
    if p.len() > MAX_PATH_BYTES {
        return Err(err("io", "percorso troppo lungo"));
    }
    if !p.starts_with('/') {
        return Err(err("io", "il percorso deve essere assoluto"));
    }
    Ok(())
}

/// `~` e `~/…` diventano `home/…`; ogni altra forma resta com'è (`~utente` non è supportato: non è assoluto).
pub fn expand_tilde(p: &str, home: &str) -> String {
    let home = home.trim_end_matches('/');
    if p == "~" {
        if home.is_empty() { "/".into() } else { home.into() }
    } else if let Some(rest) = p.strip_prefix("~/") {
        format!("{home}/{rest}")
    } else {
        p.to_string()
    }
}

pub fn needs_home(p: &str) -> bool {
    p == "~" || p.starts_with("~/")
}

/// Senza barre finali (tranne la radice): `/a/b/` e `/a/b` sono lo stesso percorso.
fn trim_slashes(p: &str) -> String {
    let t = p.trim_end_matches('/');
    if t.is_empty() { "/".into() } else { t.into() }
}

/// Percorso senza `.` né `..` né `//`, solo per mostrarlo (mai per eseguire: con i link simbolici `..` non è lessicale).
pub fn display_path(p: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for c in p.split('/') {
        match c {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            c => parts.push(c),
        }
    }
    format!("/{}", parts.join("/"))
}

/// Testo per un dialogo nativo: i caratteri di controllo diventano `?`, al massimo 80 caratteri.
pub fn sanitize_display(s: &str) -> String {
    let clean: String = s.chars().map(|c| if c.is_control() { '?' } else { c }).collect();
    if clean.chars().count() > 80 {
        format!("{}…", clean.chars().take(80).collect::<String>())
    } else {
        clean
    }
}

fn join(dir: &str, name: &str) -> String {
    if dir.ends_with('/') { format!("{dir}{name}") } else { format!("{dir}/{name}") }
}

/// Record `tipo\tnome\0`; voci malformate (senza tipo noto, nome vuoto o con `/`) scartate, come i nomi non UTF-8
/// (un nome sostituito non si potrebbe aprire, e due diversi diventerebbero uguali); tetto `MAX_ENTRIES`.
/// Dirs prima, poi il nome senza distinguere maiuscole.
pub fn parse_list(dir: &str, out: &[u8]) -> RemoteListing {
    let mut entries = Vec::new();
    let mut truncated = false;
    for rec in out.split(|b| *b == 0).filter(|r| !r.is_empty()) {
        let Some(tab) = rec.iter().position(|b| *b == b'\t') else { continue };
        let (kind, name) = (&rec[..tab], &rec[tab + 1..]);
        let Ok(name) = String::from_utf8(name.to_vec()) else { continue };
        if name.is_empty() || name.contains('/') || name == "." || name == ".." {
            continue;
        }
        let (is_dir, is_link) = match kind {
            b"d" => (true, false),
            b"f" => (false, false),
            b"L" => (true, true),
            b"l" => (false, true),
            _ => continue,
        };
        if entries.len() >= MAX_ENTRIES {
            truncated = true;
            break;
        }
        entries.push(RemoteEntry { path: join(dir, &name), name, is_dir, is_link });
    }
    entries.sort_by(|a, b| {
        b.is_dir.cmp(&a.is_dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())).then_with(|| a.name.cmp(&b.name))
    });
    RemoteListing { entries, truncated }
}

/// Versione `size:mtime` ben formata.
fn parse_version(s: &str) -> Option<&str> {
    let (a, b) = s.split_once(':')?;
    let digits = |x: &str| !x.is_empty() && x.bytes().all(|c| c.is_ascii_digit());
    (digits(a) && digits(b)).then_some(s)
}

fn script_error(out: &ToolOutput, what: &str) -> FsError {
    let msg = match out.status {
        EXIT_CONFLICT => return err("conflict", "il file è cambiato sul server"),
        EXIT_EXISTS => return err("io", "esiste già"),
        EXIT_NOT_FOUND => "non trovato sul server".to_string(),
        EXIT_TOO_LARGE => return err("too_large", "file oltre 5 MB"),
        EXIT_WRITE => "scrittura non riuscita sul server".to_string(),
        EXIT_PROTECTED => "questa cartella non si può eliminare".to_string(),
        10 => "è una cartella".to_string(),
        5 => "cartella non leggibile sul server".to_string(),
        _ => {
            let e = sanitize_display(out.stderr.trim());
            if e.is_empty() { format!("{what} non riuscito (uscita {})", out.status) } else { format!("{what}: {e}") }
        }
    };
    err("io", msg)
}

fn run(r: Runner, script: &str, args: &[&str], stdin: Option<Vec<u8>>, what: &str) -> Result<ToolOutput, FsError> {
    let out = r(script, args, stdin).map_err(|e| err("io", e))?;
    if out.status == 0 { Ok(out) } else { Err(script_error(&out, what)) }
}

pub fn home(r: Runner) -> Result<String, FsError> {
    let out = run(r, HOME_SCRIPT, &[], None, "home")?;
    let h = String::from_utf8(out.stdout).map_err(|_| err("io", "home non valida"))?;
    validate_path(&h)?;
    Ok(h)
}

/// `~` e `~/…` risolti con la home del server (una sola chiamata, solo se serve), poi validato.
pub fn resolve(r: Runner, path: &str) -> Result<String, FsError> {
    let p = if needs_home(path) { expand_tilde(path, &home(r)?) } else { path.to_string() };
    validate_path(&p)?;
    Ok(p)
}

pub fn list(r: Runner, dir: &str) -> Result<RemoteListing, FsError> {
    validate_path(dir)?;
    let dir = trim_slashes(dir);
    let out = run(r, LIST_SCRIPT, &[&dir], None, "elenco")?;
    Ok(parse_list(&dir, &out.stdout))
}

pub fn read(r: Runner, path: &str) -> Result<RemoteContent, FsError> {
    validate_path(path)?;
    let out = run(r, READ_SCRIPT, &[path], None, "lettura")?;
    let nl = out.stdout.iter().position(|b| *b == b'\n').ok_or_else(|| err("io", "risposta non valida"))?;
    let version = std::str::from_utf8(&out.stdout[..nl]).ok().and_then(parse_version).ok_or_else(|| err("io", "risposta non valida"))?.to_string();
    let body = &out.stdout[nl + 1..];
    if body.len() as u64 > MAX_READ_BYTES {
        return Err(err("too_large", "file oltre 5 MB"));
    }
    let text = String::from_utf8(body.to_vec()).map_err(|_| err("not_text", "il file non è testo UTF-8"))?;
    Ok(RemoteContent { text, version })
}

/// `version` vuota = il file non deve esistere (crea). Restituisce la nuova versione.
pub fn write(r: Runner, path: &str, text: &str, version: &str) -> Result<String, FsError> {
    validate_path(path)?;
    if !version.is_empty() && parse_version(version).is_none() {
        return Err(err("io", "versione non valida"));
    }
    let out = r(WRITE_SCRIPT, &[path, version], Some(text.as_bytes().to_vec())).map_err(|e| err("io", e))?;
    if out.status == EXIT_EXISTS {
        let name = path.rsplit('/').next().unwrap_or(path);
        return Err(err("io", format!("sul server c'è già «{}.codeart-tmp»: eliminalo e riprova", sanitize_display(name))));
    }
    if out.status != 0 {
        return Err(script_error(&out, "scrittura"));
    }
    let v = String::from_utf8_lossy(&out.stdout).into_owned();
    parse_version(&v).map(str::to_string).ok_or_else(|| err("io", "risposta non valida"))
}

pub fn mkdir(r: Runner, path: &str) -> Result<(), FsError> {
    validate_path(path)?;
    run(r, MKDIR_SCRIPT, &[path], None, "creazione cartella").map(|_| ())
}

pub fn rename(r: Runner, from: &str, to: &str) -> Result<(), FsError> {
    validate_path(from)?;
    validate_path(to)?;
    run(r, RENAME_SCRIPT, &[from, to], None, "rinomina").map(|_| ())
}

/// Messaggio della conferma nativa: server e nome sanificati; per le cartelle "e tutto il suo contenuto".
pub fn delete_message(server_name: &str, path: &str, is_dir: bool) -> String {
    let name = path.rsplit('/').next().filter(|n| !n.is_empty()).unwrap_or(path);
    let what = if is_dir { "la cartella" } else { "il file" };
    let tail = if is_dir { " e tutto il suo contenuto" } else { "" };
    format!(
        "Eliminare {what} «{}»{tail} dal server «{}»? L'operazione non si può annullare.",
        sanitize_display(name),
        sanitize_display(server_name)
    )
}

/// Elimina dopo `confirm(messaggio)` (dialogo nativo, "Annulla" predefinito). Mai la radice né la home; `..` e `.`
/// nel percorso rifiutati, per non cancellare altro da quanto mostrato nella conferma.
pub fn delete(r: Runner, server_name: &str, path: &str, confirm: &dyn Fn(&str) -> bool) -> Result<(), FsError> {
    validate_path(path)?;
    if path.split('/').any(|c| c == ".." || c == ".") {
        return Err(err("io", "percorso non valido per l'eliminazione"));
    }
    let path = trim_slashes(path);
    let kind = run(r, KIND_SCRIPT, &[&path], None, "eliminazione")?;
    let is_dir = kind.stdout == b"d";
    if !confirm(&delete_message(server_name, &path, is_dir)) {
        return Err(err("io", "eliminazione annullata"));
    }
    run(r, DELETE_SCRIPT, &[&path], None, "eliminazione").map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;
    use std::process::{Command, Stdio};

    /// Stessa meccanica del server: `sh -c <script> _ <args>`.
    fn local(shell: &'static str) -> impl Fn(&str, &[&str], Option<Vec<u8>>) -> Result<ToolOutput, String> {
        move |script, args, stdin| {
            use std::io::Write;
            let mut c = Command::new(shell);
            c.arg("-c").arg(script).arg("_").args(args);
            c.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
            let mut child = c.spawn().map_err(|e| e.to_string())?;
            let mut si = child.stdin.take().unwrap();
            let data = stdin.unwrap_or_default();
            let t = std::thread::spawn(move || {
                let _ = si.write_all(&data);
            });
            let o = child.wait_with_output().map_err(|e| e.to_string())?;
            let _ = t.join();
            Ok(ToolOutput { status: o.status.code().unwrap_or(-1), stdout: o.stdout, stderr: String::from_utf8_lossy(&o.stderr).into_owned() })
        }
    }

    /// Come sul server con una shell di login `login` (csh/tcsh compresi): `login -c "sh -c '<script>' _ <args>"`.
    fn via_login(login: &'static str) -> impl Fn(&str, &[&str], Option<Vec<u8>>) -> Result<ToolOutput, String> {
        move |script, args, stdin| {
            use std::io::Write;
            let mut c = Command::new(login);
            c.arg("-f").arg("-c").arg(crate::ssh::conn::remote_cmd(script, args));
            c.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
            let mut child = c.spawn().map_err(|e| e.to_string())?;
            let mut si = child.stdin.take().unwrap();
            let data = stdin.unwrap_or_default();
            let t = std::thread::spawn(move || {
                let _ = si.write_all(&data);
            });
            let o = child.wait_with_output().map_err(|e| e.to_string())?;
            let _ = t.join();
            Ok(ToolOutput { status: o.status.code().unwrap_or(-1), stdout: o.stdout, stderr: String::from_utf8_lossy(&o.stderr).into_owned() })
        }
    }

    #[test]
    fn every_remote_script_is_one_line_without_bang() {
        let mut all: Vec<&str> = ALL_SCRIPTS.to_vec();
        all.extend_from_slice(crate::ssh::transfer::ALL_SCRIPTS);
        all.push(crate::pty::REMOTE_SHELL_SCRIPT);
        for s in all {
            assert!(!s.contains('\n') && !s.contains('\r'), "a capo in {s:?}");
            assert!(!s.contains('!'), "«!» in {s:?}");
        }
    }

    /// Server con csh/tcsh come shell di login: ogni operazione sui file funziona lo stesso.
    #[test]
    fn csh_login_shells_run_every_script() {
        for login in ["/bin/tcsh", "/bin/csh"] {
            if !std::path::Path::new(login).exists() {
                continue;
            }
            let r = via_login(login);
            let d = dir();
            std::fs::write(d.path().join(".hidden"), "h").unwrap();
            std::fs::write(d.path().join("..dots"), "d").unwrap();
            std::fs::create_dir(d.path().join("sub")).unwrap();
            let names: Vec<String> = list(&r, &p(&d, "")).unwrap().entries.into_iter().map(|e| e.name).collect();
            assert_eq!(names, ["sub", "..dots", ".hidden"], "{login}");
            assert!(home(&r).unwrap().starts_with('/'));
            let f = p(&d, "f.txt");
            let v1 = write(&r, &f, "uno\n", "").unwrap();
            let c = read(&r, &f).unwrap();
            assert_eq!((c.text.as_str(), c.version.as_str()), ("uno\n", v1.as_str()), "{login}");
            write(&r, &f, "due", &v1).unwrap();
            mkdir(&r, &p(&d, "nuova")).unwrap();
            rename(&r, &f, &p(&d, "g.txt")).unwrap();
            delete(&r, "Dev", &p(&d, "nuova"), &|_| true).unwrap();
            assert_eq!(std::fs::read_to_string(p(&d, "g.txt")).unwrap(), "due");
            assert!(!d.path().join("nuova").exists() && !d.path().join("f.txt").exists());
        }
    }

    #[test]
    fn write_never_follows_a_planted_temp_file() {
        for sh in ["/bin/sh", "/bin/dash"] {
            if !std::path::Path::new(sh).exists() {
                continue;
            }
            let r = local(sh);
            let d = dir();
            let f = p(&d, "conf");
            let v = write(&r, &f, "a", "").unwrap();
            // qualcun altro ha messo `conf.codeart-tmp` come link verso un suo file: rifiutato, nulla scritto
            let target = p(&d, "altrove");
            std::fs::write(&target, "intatto").unwrap();
            symlink(&target, p(&d, "conf.codeart-tmp")).unwrap();
            let e = write(&r, &f, "nuovo", &v).unwrap_err();
            assert!(e.message.contains("conf.codeart-tmp"), "{sh}: {}", e.message);
            assert_eq!(std::fs::read_to_string(&target).unwrap(), "intatto");
            assert_eq!(std::fs::read_to_string(&f).unwrap(), "a");
            // anche un link rotto o un file normale
            std::fs::remove_file(p(&d, "conf.codeart-tmp")).unwrap();
            symlink(p(&d, "manca"), p(&d, "conf.codeart-tmp")).unwrap();
            assert!(write(&r, &f, "nuovo", &v).is_err());
            assert!(!d.path().join("manca").exists());
            std::fs::remove_file(p(&d, "conf.codeart-tmp")).unwrap();
            std::fs::write(p(&d, "conf.codeart-tmp"), "suo").unwrap();
            assert!(write(&r, &f, "nuovo", &v).is_err());
            assert_eq!(std::fs::read_to_string(p(&d, "conf.codeart-tmp")).unwrap(), "suo");
            std::fs::remove_file(p(&d, "conf.codeart-tmp")).unwrap();
            write(&r, &f, "nuovo", &v).unwrap();
            assert_eq!(std::fs::read_to_string(&f).unwrap(), "nuovo");
        }
    }

    fn p(d: &tempfile::TempDir, rel: &str) -> String {
        format!("{}/{rel}", d.path().canonicalize().unwrap().display())
    }

    fn dir() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    fn hostile(d: &tempfile::TempDir) -> Vec<&'static str> {
        let names = vec![
            "a b", "it's", "$(id)", "x;y", "new\nline", "-rf", "-n", "è.txt", ".hidden", "..dots", "`id`", "a\"b", "*", "back\\slash", "~",
        ];
        for n in &names {
            std::fs::write(d.path().join(n), "x").unwrap();
        }
        std::fs::create_dir(d.path().join("sub dir")).unwrap();
        std::fs::create_dir(d.path().join(".git")).unwrap();
        symlink("sub dir", d.path().join("link-dir")).unwrap();
        symlink("a b", d.path().join("link-file")).unwrap();
        symlink("manca", d.path().join("link-broken")).unwrap();
        names
    }

    fn check_list(shell: &'static str) {
        let d = dir();
        let names = hostile(&d);
        let l = list(&local(shell), &p(&d, "")).unwrap();
        assert!(!l.truncated);
        let got: std::collections::HashMap<_, _> = l.entries.iter().map(|e| (e.name.as_str(), (e.is_dir, e.is_link))).collect();
        for n in &names {
            assert_eq!(got.get(n), Some(&(false, false)), "{n:?} in {got:?}");
        }
        assert_eq!(got["sub dir"], (true, false));
        assert_eq!(got[".git"], (true, false));
        assert_eq!(got["link-dir"], (true, true));
        assert_eq!(got["link-file"], (false, true));
        assert_eq!(got["link-broken"], (false, true));
        assert_eq!(got.len(), names.len() + 5, "{got:?}");
        // cartelle prima, poi nome senza maiuscole
        let first_files = l.entries.iter().position(|e| !e.is_dir).unwrap();
        assert!(l.entries[..first_files].iter().all(|e| e.is_dir));
        assert!(l.entries[first_files..].iter().all(|e| !e.is_dir));
        // il percorso di ogni voce è quello vero
        for e in &l.entries {
            assert!(std::fs::symlink_metadata(&e.path).is_ok(), "{}", e.path);
        }
    }

    #[test]
    fn list_survives_hostile_names() {
        check_list("/bin/sh");
    }

    #[test]
    fn list_empty_and_missing_dir() {
        let d = dir();
        let l = list(&local("/bin/sh"), &p(&d, "")).unwrap();
        assert!(l.entries.is_empty());
        let e = list(&local("/bin/sh"), &p(&d, "manca")).unwrap_err();
        assert_eq!(e.code, "io");
        let f = p(&d, "f");
        std::fs::write(&f, "x").unwrap();
        assert!(list(&local("/bin/sh"), &f).is_err());
    }

    #[test]
    fn list_stops_at_the_cap() {
        let d = dir();
        for i in 0..(MAX_ENTRIES + 3) {
            std::fs::write(d.path().join(format!("f{i:05}")), "").unwrap();
        }
        let l = list(&local("/bin/sh"), &p(&d, "")).unwrap();
        assert_eq!(l.entries.len(), MAX_ENTRIES);
        assert!(l.truncated);
    }

    #[test]
    fn parser_handles_nul_records() {
        let l = parse_list("/home/dev", b"f\tb.txt\0d\tZed\0f\ta\nb\0L\tlnk\0l\tbad\0x\tweird\0f\t\0f\ta/b\0nonewline\0f\t..\0d\talfa\0f\t-x\0");
        let names: Vec<_> = l.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["alfa", "lnk", "Zed", "-x", "a\nb", "b.txt", "bad"]);
        assert_eq!(l.entries[1].path, "/home/dev/lnk");
        assert!(l.entries[1].is_dir && l.entries[1].is_link);
        assert!(!l.entries[6].is_dir && l.entries[6].is_link);
        assert_eq!(parse_list("/", b"f\tx\0").entries[0].path, "/x");
        assert!(parse_list("/", b"").entries.is_empty());
        // UTF-8 non valido: scartato (non si potrebbe aprire; due nomi diversi diventerebbero uguali)
        let l = parse_list("/", b"f\tb\xffd\0f\tb\xfed\0f\tok\0");
        assert_eq!(l.entries.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["ok"]);
        let many: Vec<u8> = (0..MAX_ENTRIES + 1).flat_map(|i| format!("f\tn{i}\0").into_bytes()).collect();
        let l = parse_list("/", &many);
        assert_eq!((l.entries.len(), l.truncated), (MAX_ENTRIES, true));
    }

    #[test]
    fn path_rules() {
        for bad in ["rel", "", "a/b", "/a\0b", &format!("/{}", "x".repeat(4096))] {
            assert!(validate_path(bad).is_err(), "{bad:?}");
        }
        for ok in ["/", "/a/../b", "/home/dev/a b", &format!("/{}", "x".repeat(4095))] {
            assert!(validate_path(ok).is_ok(), "{ok:?}");
        }
        assert_eq!(expand_tilde("~", "/home/dev"), "/home/dev");
        assert_eq!(expand_tilde("~/a/b", "/home/dev/"), "/home/dev/a/b");
        assert_eq!(expand_tilde("/x/~", "/home/dev"), "/x/~");
        assert_eq!(expand_tilde("~x", "/home/dev"), "~x");
        assert!(needs_home("~") && needs_home("~/a") && !needs_home("~a") && !needs_home("/~"));
        assert_eq!(display_path("/a/./b/../c//d/"), "/a/c/d");
        assert_eq!(display_path("/../.."), "/");
        assert_eq!(trim_slashes("/a/b//"), "/a/b");
        assert_eq!(trim_slashes("///"), "/");
    }

    #[test]
    fn home_comes_from_the_server() {
        let h = home(&local("/bin/sh")).unwrap();
        assert!(h.starts_with('/'));
    }

    fn check_stat(shell: &'static str) {
        let d = dir();
        let f = p(&d, "f");
        std::fs::write(&f, "ciao").unwrap();
        let out = local(shell)(STAT_SCRIPT, &[&f], None).unwrap();
        assert_eq!(out.status, 0);
        let v = String::from_utf8(out.stdout).unwrap();
        assert!(v.starts_with("4:") && parse_version(&v).is_some(), "{v}");
        symlink("f", p(&d, "l")).unwrap();
        let lv = local(shell)(STAT_SCRIPT, &[&p(&d, "l")], None).unwrap();
        assert_eq!(String::from_utf8(lv.stdout).unwrap(), v);
        assert_eq!(local(shell)(STAT_SCRIPT, &[&p(&d, "manca")], None).unwrap().status, 6);
    }

    #[test]
    fn stat_script() {
        check_stat("/bin/sh");
    }

    #[test]
    fn posix_shells_agree() {
        for sh in ["/bin/dash", "/bin/sh"] {
            if std::path::Path::new(sh).exists() {
                check_list(sh);
                check_stat(sh);
                let d = dir();
                let f = p(&d, "w");
                let v = write(&local(sh), &f, "x", "").unwrap();
                write(&local(sh), &f, "yy", &v).unwrap();
                assert_eq!(std::fs::read_to_string(&f).unwrap(), "yy", "{sh}");
            }
        }
    }

    #[test]
    fn read_returns_text_and_version() {
        let d = dir();
        let r = local("/bin/sh");
        std::fs::write(p(&d, "a b"), "riga 1\nè due\n\n").unwrap();
        let c = read(&r, &p(&d, "a b")).unwrap();
        assert_eq!(c.text, "riga 1\nè due\n\n");
        assert!(c.version.starts_with("15:"), "{}", c.version);
        std::fs::write(p(&d, "vuoto"), "").unwrap();
        assert_eq!(read(&r, &p(&d, "vuoto")).unwrap().text, "");
        std::fs::write(p(&d, "bin"), [0x66, 0xff, 0xfe]).unwrap();
        assert_eq!(read(&r, &p(&d, "bin")).unwrap_err().code, "not_text");
        std::fs::write(p(&d, "big"), vec![b'a'; 5 * 1024 * 1024 + 1]).unwrap();
        assert_eq!(read(&r, &p(&d, "big")).unwrap_err().code, "too_large");
        std::fs::write(p(&d, "limit"), vec![b'a'; 5 * 1024 * 1024]).unwrap();
        assert_eq!(read(&r, &p(&d, "limit")).unwrap().text.len(), 5 * 1024 * 1024);
        assert_eq!(read(&r, &p(&d, "manca")).unwrap_err().code, "io");
        std::fs::create_dir(p(&d, "cart")).unwrap();
        assert_eq!(read(&r, &p(&d, "cart")).unwrap_err().message, "è una cartella");
        assert!(read(&r, "relativo").is_err());
    }

    #[test]
    fn write_creates_updates_and_detects_conflicts() {
        let d = dir();
        let r = local("/bin/sh");
        let f = p(&d, "-n file;$(x)");
        // crea: versione vuota
        let v1 = write(&r, &f, "uno\n", "").unwrap();
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "uno\n");
        assert_eq!(read(&r, &f).unwrap().version, v1);
        // crea su un file che esiste: conflitto
        assert_eq!(write(&r, &f, "x", "").unwrap_err().code, "conflict");
        // aggiorna con la versione giusta
        let v2 = write(&r, &f, "due è\n", &v1).unwrap();
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "due è\n");
        assert_eq!(read(&r, &f).unwrap().version, v2);
        // versione vecchia o sbagliata: conflitto, il file non cambia
        assert_eq!(write(&r, &f, "tre", "99:1").unwrap_err().code, "conflict");
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "due è\n");
        // file sparito: conflitto
        std::fs::remove_file(&f).unwrap();
        assert_eq!(write(&r, &f, "x", &v2).unwrap_err().code, "conflict");
        // nessun file temporaneo avanzato
        assert_eq!(std::fs::read_dir(d.path()).unwrap().count(), 0);
        assert!(write(&r, &f, "x", "abc").is_err());
        assert!(write(&r, &p(&d, "no/dir/f"), "x", "").is_err());
        assert_eq!(std::fs::read_dir(d.path()).unwrap().count(), 0);
    }

    #[test]
    fn write_keeps_mode_and_goes_through_links() {
        use std::os::unix::fs::PermissionsExt;
        let d = dir();
        let r = local("/bin/sh");
        let f = p(&d, "run.sh");
        std::fs::write(&f, "a").unwrap();
        std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o750)).unwrap();
        let v = read(&r, &f).unwrap().version;
        write(&r, &f, "bb", &v).unwrap();
        assert_eq!(std::fs::metadata(&f).unwrap().permissions().mode() & 0o777, 0o750);
        symlink("run.sh", p(&d, "lnk")).unwrap();
        let lv = read(&r, &p(&d, "lnk")).unwrap().version;
        write(&r, &p(&d, "lnk"), "ccc", &lv).unwrap();
        assert!(std::fs::symlink_metadata(p(&d, "lnk")).unwrap().file_type().is_symlink());
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "ccc");
    }

    #[test]
    fn mkdir_and_rename() {
        let d = dir();
        let r = local("/bin/sh");
        mkdir(&r, &p(&d, "nuova cartella")).unwrap();
        assert!(d.path().join("nuova cartella").is_dir());
        assert!(mkdir(&r, &p(&d, "nuova cartella")).is_err());
        assert!(mkdir(&r, &p(&d, "a/b/c")).is_err());
        std::fs::write(p(&d, "a"), "A").unwrap();
        std::fs::write(p(&d, "b"), "B").unwrap();
        let e = rename(&r, &p(&d, "a"), &p(&d, "b")).unwrap_err();
        assert_eq!(e.message, "esiste già");
        assert_eq!(std::fs::read_to_string(p(&d, "b")).unwrap(), "B");
        symlink("manca", p(&d, "rotto")).unwrap();
        assert_eq!(rename(&r, &p(&d, "a"), &p(&d, "rotto")).unwrap_err().message, "esiste già");
        rename(&r, &p(&d, "a"), &p(&d, "-c d'e")).unwrap();
        assert_eq!(std::fs::read_to_string(p(&d, "-c d'e")).unwrap(), "A");
        assert!(rename(&r, &p(&d, "a"), &p(&d, "z")).is_err());
        assert!(rename(&r, "a", &p(&d, "z")).is_err());
    }

    #[test]
    fn delete_confirms_first_and_removes_recursively() {
        let d = dir();
        let r = local("/bin/sh");
        std::fs::create_dir_all(d.path().join("cart/in/side")).unwrap();
        std::fs::write(d.path().join("cart/in/side/f"), "x").unwrap();
        std::fs::write(d.path().join("-f"), "x").unwrap();
        let seen = std::cell::RefCell::new(Vec::new());
        let no = |m: &str| {
            seen.borrow_mut().push(m.to_string());
            false
        };
        let yes = |m: &str| {
            seen.borrow_mut().push(m.to_string());
            true
        };
        let e = delete(&r, "Dev\nbox", &p(&d, "cart"), &no).unwrap_err();
        assert!(e.message.contains("annullata"));
        assert!(d.path().join("cart/in/side/f").exists());
        assert!(seen.borrow()[0].contains("e tutto il suo contenuto") && seen.borrow()[0].contains("«Dev?box»"));
        delete(&r, "Dev", &format!("{}/", p(&d, "cart")), &yes).unwrap();
        assert!(!d.path().join("cart").exists());
        delete(&r, "Dev", &p(&d, "-f"), &yes).unwrap();
        assert!(!d.path().join("-f").exists());
        assert!(!seen.borrow()[2].contains("contenuto"));
        // un link si toglie senza toccare il bersaglio
        std::fs::create_dir(d.path().join("t")).unwrap();
        std::fs::write(d.path().join("t/f"), "x").unwrap();
        symlink("t", p(&d, "l")).unwrap();
        delete(&r, "Dev", &p(&d, "l"), &yes).unwrap();
        assert!(d.path().join("t/f").exists() && !d.path().join("l").exists());
        // inesistente: errore senza dialogo
        let n = seen.borrow().len();
        assert!(delete(&r, "Dev", &p(&d, "manca"), &yes).is_err());
        assert_eq!(seen.borrow().len(), n);
        // radice, home e `..` rifiutati senza dialogo
        for bad in ["/", "/a/../b", "/a/./b", "relativo"] {
            assert!(delete(&r, "Dev", bad, &yes).is_err(), "{bad}");
        }
        assert_eq!(seen.borrow().len(), n);
        let home = std::env::var("HOME").unwrap();
        assert!(delete(&r, "Dev", &home, &yes).is_err());
        assert_eq!(seen.borrow().len(), n);
    }

    #[test]
    fn delete_message_is_sanitized() {
        let m = delete_message("S\u{7}rv", "/a/b\nc\u{1b}[31m", false);
        assert!(!m.chars().any(|c| c.is_control()), "{m:?}");
        assert!(m.contains("il file") && m.contains("«b?c?[31m»"));
        assert!(delete_message("s", "/a/d", true).contains("la cartella «d» e tutto il suo contenuto"));
        assert!(sanitize_display(&"x".repeat(200)).chars().count() <= 81);
    }

    #[test]
    fn errors_from_the_connection_pass_through() {
        let fail = |_: &str, _: &[&str], _: Option<Vec<u8>>| -> Result<ToolOutput, String> { Err("Non connesso al server".into()) };
        assert_eq!(list(&fail, "/a").unwrap_err().message, "Non connesso al server");
        let odd = |_: &str, _: &[&str], _: Option<Vec<u8>>| -> Result<ToolOutput, String> {
            Ok(ToolOutput { status: 255, stdout: vec![], stderr: "boom\u{7}".into() })
        };
        let e = mkdir(&odd, "/a").unwrap_err();
        assert!(e.message.contains("boom?"), "{}", e.message);
    }
}
