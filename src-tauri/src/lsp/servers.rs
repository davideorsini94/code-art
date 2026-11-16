//! Language server noti: per ogni linguaggio i binari candidati in ordine di preferenza, i loro argomenti e come
//! installarli. Unica tabella, usata da `lsp_start` e da `tools_check`.
use crate::tools::find_tool_in;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub struct Candidate {
    pub bin: &'static str,
    pub args: &'static [&'static str],
    /// `-data <cartella>` in coda (jdtls: spazio di lavoro per progetto).
    pub data_dir: bool,
}

pub struct LspServer {
    pub lang: &'static str,
    pub candidates: &'static [Candidate],
    pub install_hint: &'static str,
}

pub const SERVERS: &[LspServer] = &[
    LspServer {
        lang: "python",
        candidates: &[Candidate { bin: "pyright-langserver", args: &["--stdio"], data_dir: false }, Candidate { bin: "pylsp", args: &[], data_dir: false }],
        install_hint: "npm i -g pyright  oppure  pip install python-lsp-server",
    },
    LspServer { lang: "java", candidates: &[Candidate { bin: "jdtls", args: &[], data_dir: true }], install_hint: "brew install jdtls" },
    LspServer { lang: "go", candidates: &[Candidate { bin: "gopls", args: &[], data_dir: false }], install_hint: "go install golang.org/x/tools/gopls@latest" },
    LspServer { lang: "rust", candidates: &[Candidate { bin: "rust-analyzer", args: &[], data_dir: false }], install_hint: "rustup component add rust-analyzer" },
];

/// Server trovato: binario, argomenti e cartella dati da creare (se serve).
#[derive(Debug, PartialEq)]
pub struct Resolved {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub data_dir: Option<PathBuf>,
}

pub fn server(lang: &str) -> Option<&'static LspServer> {
    SERVERS.iter().find(|s| s.lang == lang)
}

/// Primo candidato installato per `lang`; altrimenti un errore con il suggerimento d'installazione.
pub fn resolve(env: &HashMap<String, String>, system_dirs: &[&str], lang: &str, data_dir: &Path) -> Result<Resolved, String> {
    let s = server(lang).ok_or_else(|| format!("linguaggio non supportato: {lang}"))?;
    s.candidates
        .iter()
        .find_map(|c| {
            let program = find_tool_in(env, c.bin, system_dirs)?;
            let mut args: Vec<String> = c.args.iter().map(|a| a.to_string()).collect();
            if c.data_dir {
                args.extend(["-data".to_string(), data_dir.to_string_lossy().into_owned()]);
            }
            Some(Resolved { program, args, data_dir: c.data_dir.then(|| data_dir.to_path_buf()) })
        })
        .ok_or_else(|| format!("language server per {lang} non trovato: installa con `{}`", s.install_hint))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn exe(dir: &Path, name: &str) {
        std::fs::create_dir_all(dir).unwrap();
        let p = dir.join(name);
        std::fs::write(&p, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn env_with(bin: &Path) -> HashMap<String, String> {
        [("PATH".to_string(), bin.to_string_lossy().into_owned()), ("HOME".to_string(), "/nonexistent".to_string())].into()
    }

    #[test]
    fn server_for_lang_prefers_pyright() {
        let d = tempfile::tempdir().unwrap();
        let bin = d.path().join("bin");
        exe(&bin, "pylsp");
        let env = env_with(&bin);
        let data = Path::new("/Users/dev/dati");
        let r = resolve(&env, &[], "python", data).unwrap();
        assert_eq!((r.program, r.args, r.data_dir), (bin.join("pylsp"), vec![], None));
        exe(&bin, "pyright-langserver");
        let r = resolve(&env, &[], "python", data).unwrap();
        assert_eq!((r.program, r.args), (bin.join("pyright-langserver"), vec!["--stdio".to_string()]));
    }

    #[test]
    fn java_gets_its_data_dir_and_others_no_args() {
        let d = tempfile::tempdir().unwrap();
        let bin = d.path().join("bin");
        for b in ["jdtls", "gopls", "rust-analyzer"] {
            exe(&bin, b);
        }
        let env = env_with(&bin);
        let data = Path::new("/Users/dev/Library/Application Support/CodeArt/jdtls/p1");
        let j = resolve(&env, &[], "java", data).unwrap();
        assert_eq!(j.args, vec!["-data".to_string(), data.to_string_lossy().into_owned()]);
        assert_eq!(j.data_dir.as_deref(), Some(data));
        for (lang, b) in [("go", "gopls"), ("rust", "rust-analyzer")] {
            let r = resolve(&env, &[], lang, data).unwrap();
            assert_eq!((r.program, r.args, r.data_dir), (bin.join(b), vec![], None), "{lang}");
        }
    }

    #[test]
    fn missing_server_returns_install_hint() {
        let d = tempfile::tempdir().unwrap();
        let env = env_with(&d.path().join("vuota"));
        let data = Path::new("/Users/dev/dati");
        for (lang, hint) in [
            ("python", "pip install python-lsp-server"),
            ("python", "npm i -g pyright"),
            ("java", "brew install jdtls"),
            ("go", "go install golang.org/x/tools/gopls@latest"),
            ("rust", "rustup component add rust-analyzer"),
        ] {
            let e = resolve(&env, &[], lang, data).unwrap_err();
            assert!(e.contains(hint), "{lang}: {e}");
            assert!(server(lang).unwrap().install_hint.contains(hint));
        }
        assert!(resolve(&env, &[], "cobol", data).unwrap_err().contains("non supportato"));
        assert!(server("cobol").is_none());
        assert_eq!(SERVERS.len(), 4);
    }
}
