//! L'app come `SSH_ASKPASS` di sé stessa: quando `ssh` chiede una password rilancia l'eseguibile dell'app con il
//! prompt in `argv[1]` e `CODEART_ASKPASS_ACCOUNT=<id del server>` nell'ambiente. In quel caso l'app non avvia Tauri
//! (nessuna finestra): risponde con la password del Portachiavi su stdout ed esce 0, oppure esce 1 senza stampare
//! nulla (passphrase di una chiave, domande sì/no sulla chiave del server, password assente, qualunque altro prompt,
//! padre che non è `/usr/bin/ssh`).
use super::keychain;
use std::io::Write;

/// L'unico processo a cui si risponde: `ssh` di sistema (il padre di askpass quando lo rilancia `ssh`).
pub const SSH_BIN: &str = "/usr/bin/ssh";

/// Variabile d'ambiente con l'account (id del server) da leggere nel Portachiavi; la imposta il core avviando `ssh`.
pub const ACCOUNT_ENV: &str = "CODEART_ASKPASS_ACCOUNT";

/// Solo prompt di password: il testo, senza spazi ai bordi, termina con `password:` (maiuscole indifferenti),
/// come `dev@example.test's password: ` o `Password:`.
pub fn is_password_prompt(prompt: &str) -> bool {
    prompt.trim().to_lowercase().ends_with("password:")
}

/// Risposta da dare a `ssh` per `prompt`: la password, solo se il prompt la chiede e se ce n'è una.
pub fn answer_for(prompt: &str, secret: Option<&str>) -> Option<String> {
    if !is_password_prompt(prompt) {
        return None;
    }
    secret.filter(|s| !s.is_empty()).map(str::to_string)
}

/// Da chiamare per prima cosa in `main()`: `false` = avvio normale dell'app. Con `CODEART_ASKPASS_ACCOUNT`
/// nell'ambiente non ritorna mai: risponde a `ssh` ed esce.
pub fn maybe_run() -> bool {
    let Some(account) = std::env::var_os(ACCOUNT_ENV) else {
        return false;
    };
    let prompt = std::env::args_os().nth(1).and_then(|p| p.into_string().ok()).unwrap_or_default();
    std::process::exit(run(account.to_str(), &prompt))
}

/// Il padre è davvero `/usr/bin/ssh`? Un altro processo che imposti `CODEART_ASKPASS_ACCOUNT` e rilanci l'app
/// non ottiene la password.
pub fn parent_ok(path: &str) -> bool {
    path == SSH_BIN
}

/// Percorso dell'eseguibile del processo padre (`proc_pidpath`); `None` se non si può leggere.
fn parent_path() -> Option<String> {
    // PROC_PIDPATHINFO_MAXSIZE = 4 * MAXPATHLEN
    let mut buf = vec![0u8; 4096];
    // SAFETY: semplici chiamate di sistema; `buf` è valido per `buf.len()` byte e `proc_pidpath` ne scrive al
    // massimo tanti, restituendo la lunghezza scritta (≤ 0 = errore).
    let n = unsafe {
        let ppid = libc::getppid();
        libc::proc_pidpath(ppid, buf.as_mut_ptr().cast(), buf.len() as u32)
    };
    let n = usize::try_from(n).ok().filter(|n| *n > 0 && *n <= buf.len())?;
    buf.truncate(n);
    String::from_utf8(buf).ok()
}

fn run(account: Option<&str>, prompt: &str) -> i32 {
    let Some(account) = account.filter(|a| !a.is_empty()) else {
        return 1;
    };
    // il Portachiavi si legge solo per un prompt di password
    if !is_password_prompt(prompt) {
        return 1;
    }
    // né il Portachiavi si legge se a chiedere non è `ssh` di sistema
    if !parent_path().is_some_and(|p| parent_ok(&p)) {
        return 1;
    }
    let secret = keychain::get(account).ok().flatten();
    let Some(answer) = answer_for(prompt, secret.as_deref()) else {
        return 1;
    };
    let mut out = std::io::stdout().lock();
    match writeln!(out, "{answer}").and_then(|()| out.flush()) {
        Ok(()) => 0,
        Err(_) => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers_only_password_prompts() {
        let s = Some("segreto");
        assert_eq!(answer_for("dev@example.test's password: ", s).as_deref(), Some("segreto"));
        assert_eq!(answer_for("Password:", s).as_deref(), Some("segreto"));
        assert_eq!(answer_for("(dev@example.test) PASSWORD:  \n", s).as_deref(), Some("segreto"));
        assert_eq!(answer_for("Enter passphrase for key '/home/dev/.ssh/id_ed25519': ", s), None);
        assert_eq!(
            answer_for("Are you sure you want to continue connecting (yes/no/[fingerprint])? ", s),
            None
        );
        assert_eq!(answer_for("password: please confirm (yes/no)?", s), None);
        assert_eq!(answer_for("", s), None);
        assert_eq!(answer_for("   ", s), None);
        assert_eq!(answer_for("dev@example.test's password: ", None), None);
        assert_eq!(answer_for("dev@example.test's password: ", Some("")), None);
    }

    #[test]
    fn run_refuses_without_account_or_password_prompt() {
        // nessuna lettura del Portachiavi in questi casi: escono 1 prima
        assert_eq!(run(None, "Password:"), 1);
        assert_eq!(run(Some(""), "Password:"), 1);
        assert_eq!(run(Some("codeart-test-x"), "Enter passphrase for key '/home/dev/.ssh/id_x': "), 1);
        // il padre qui è `cargo test`, non `/usr/bin/ssh`: esce 1 senza toccare il Portachiavi
        assert_eq!(run(Some("codeart-test-x"), "Password:"), 1);
    }

    #[test]
    fn only_system_ssh_may_ask() {
        assert!(parent_ok("/usr/bin/ssh"));
        for bad in ["", "/usr/bin/ssh2", "/usr/local/bin/ssh", "/opt/homebrew/bin/ssh", "/tmp/usr/bin/ssh", "usr/bin/ssh", "/usr/bin/ssh "] {
            assert!(!parent_ok(bad), "{bad:?}");
        }
        // il padre del test è leggibile e non è ssh
        let p = parent_path().expect("percorso del padre");
        assert!(p.starts_with('/') && !parent_ok(&p), "{p}");
    }
}
