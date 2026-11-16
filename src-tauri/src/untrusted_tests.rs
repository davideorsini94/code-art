//! Prova trasversale (Task 11, tappa 2): ogni comando di git, GitHub, language server e sostituzione che avvia
//! processi o scrive sul disco rifiuta un progetto non fidato PRIMA di avviare qualunque cosa. Sta dentro il crate
//! perché `github` e `lsp` non sono pubblici. Esclusi per scelta, perché non eseguono codice di un progetto:
//! `gh_accounts`, `gh_repo_list`, `gh_clone` (finestra di benvenuto), `search_start`/`search_cancel` (sola lettura).
use crate::git::remote::{self, RemoteEvent, RemoteOp, RemoteOps};
use crate::git::write::{self, Confirm, Trasher};
use crate::github::{self, Gh};
use crate::lsp::Lsp;
use crate::store::{Project, Store};
use crate::trust::UNTRUSTED;
use std::collections::HashMap;
use std::process::Command;
use std::path::{Path, PathBuf};
use std::sync::Arc;

struct Never;
impl Confirm for Never {
    fn confirm(&self, _: &str, _: &str, _: &str) -> bool {
        panic!("nessuna conferma prima del controllo di fiducia")
    }
}
impl Trasher for Never {
    fn trash(&self, _: &Path) -> Result<(), String> {
        panic!("niente nel Cestino")
    }
}

#[test]
fn every_process_or_write_command_refuses_an_untrusted_project() {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path().canonicalize().unwrap();
    let repo = base.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    let st = Command::new("git").current_dir(&repo).env("GIT_CONFIG_GLOBAL", "/dev/null").args(["init", "-q"]).status().unwrap();
    assert!(st.success());
    let store = Store::open(&base.join("t.db")).unwrap();
    let folder = repo.to_string_lossy().into_owned();
    // progetto NON fidato (nessun `mark_trusted`)
    store.upsert_project(&Project { id: "p".into(), name: "p".into(), folders: vec![folder.clone()] }).unwrap();
    let r = folder.as_str();
    let paths = vec!["a.txt".to_string()];
    let err = |e: String| assert_eq!(e, UNTRUSTED);

    // git: lettura (esegue hook e fsmonitor)
    err(crate::git::repos_of(&store, "p").unwrap_err());
    err(crate::git::status(&store, "p", r).unwrap_err());
    err(crate::git::diff(&store, "p", r, "a.txt", false).unwrap_err());
    err(crate::git::branches(&store, "p", r).unwrap_err());
    err(crate::git::log(&store, "p", r, 5).unwrap_err());
    // git: scrittura
    err(write::stage(&store, "p", r, &paths).unwrap_err());
    err(write::unstage(&store, "p", r, &paths).unwrap_err());
    err(write::discard(&store, "p", r, &paths, &Never, &Never).unwrap_err());
    err(write::commit(&store, "p", r, "msg", false).unwrap_err());
    err(write::checkout(&store, "p", r, "main").unwrap_err());
    err(write::create_branch(&store, "p", r, "nuovo", None, true).unwrap_err());
    err(write::delete_branch(&store, "p", r, "vecchio", &Never).unwrap_err());
    err(write::merge(&store, "p", r, "altro").unwrap_err());
    err(write::merge_abort(&store, "p", r).unwrap_err());
    err(write::init(&store, "p", r).unwrap_err());
    // git: rete (fetch, pull, push)
    for op in [RemoteOp::Fetch, RemoteOp::Pull, RemoteOp::Push] {
        let e = remote::start(&store, "p", r, op, Arc::new(RemoteOps::default()), |_: RemoteEvent| panic!("non deve partire")).unwrap_err();
        err(e);
    }
    // gh: un binario inesistente prova che nulla viene avviato
    let gh = Gh::at(PathBuf::from("/Users/dev/non-esiste/gh"), HashMap::new());
    err(github::remote(&store, "p", r).unwrap_err());
    err(github::repo_create(&gh, &store, "p", r, "demo", true, "").unwrap_err());
    err(github::pr_list(&gh, &store, "p", r, "open").unwrap_err());
    err(github::pr_checks(&gh, &store, "p", r, 1).unwrap_err());
    err(github::pr_create(&gh, &store, "p", r, "t", "", "", false).unwrap_err());
    err(github::pr_merge(&gh, &store, "p", r, 1, "merge", false, &Never).unwrap_err());
    // language server
    for lang in ["python", "java", "go", "rust"] {
        let start = Arc::new(Lsp::default()).start(&store, "p", lang, |_| panic!("non deve partire"), Arc::new(|_| {}));
        err(start.unwrap_err());
    }
    let msg = r#"{"jsonrpc":"2.0","method":"initialized","params":{}}"#;
    err(Lsp::default().send(&store, "p", 1, msg).unwrap_err());
    // sostituzione nei file
    err(crate::search::replace_targets(&store, "proj-p", &[folder.clone() + "/a.txt"]).unwrap_err());
}
