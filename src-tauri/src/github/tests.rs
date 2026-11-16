use super::*;
use crate::git::testutil::*;
use std::sync::atomic::{AtomicUsize, Ordering};

const TOKEN: &str = "gho_FAKEtoken0000000000000000000000000";
const PR_JSON: &str = "number,title,author,headRefName,baseRefName,state,isDraft,url,statusCheckRollup";

/// `gh` finto (`tests/fake_gh.sh`) con un ambiente minimo: niente configurazione vera di gh, HOME fittizia.
struct Fake {
    dir: tempfile::TempDir,
    gh: Gh,
}

fn fake_with(mode: &str) -> Fake {
    let dir = tempfile::tempdir().unwrap();
    let bin = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fake_gh.sh");
    let env: HashMap<String, String> = [
        ("PATH", "/usr/bin:/bin".to_string()),
        ("HOME", "/Users/dev".to_string()),
        ("FAKE_GH_LOG", dir.path().join("log").to_string_lossy().into_owned()),
        ("FAKE_GH_STDIN", dir.path().join("stdin").to_string_lossy().into_owned()),
        ("FAKE_GH_MODE", mode.to_string()),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v))
    .collect();
    Fake { gh: Gh::at(bin, env), dir }
}

fn fake() -> Fake {
    fake_with("")
}

impl Fake {
    /// Chiamate ricevute: (argomenti, valore di `GH_TOKEN` se presente).
    fn calls(&self) -> Vec<(Vec<String>, Option<String>)> {
        let text = std::fs::read_to_string(self.dir.path().join("log")).unwrap_or_default();
        let mut out = Vec::new();
        let mut cur: Vec<String> = Vec::new();
        for line in text.lines() {
            if line == "--" {
                let tok = cur.pop().unwrap_or_default();
                let tok = if tok == "GH_TOKEN-" { None } else { Some(tok.trim_start_matches("GH_TOKEN=").to_string()) };
                out.push((std::mem::take(&mut cur), tok));
            } else {
                cur.push(line.to_string());
            }
        }
        out
    }

    /// Argomenti dell'unica chiamata che inizia con `a b`.
    fn call(&self, a: &str, b: &str) -> (Vec<String>, Option<String>) {
        let found: Vec<_> = self.calls().into_iter().filter(|(args, _)| args.len() >= 2 && args[0] == a && args[1] == b).collect();
        assert_eq!(found.len(), 1, "chiamate `{a} {b}`: {found:?}");
        found.into_iter().next().unwrap()
    }

    fn stdin(&self) -> String {
        std::fs::read_to_string(self.dir.path().join("stdin")).unwrap()
    }
}

fn v(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

struct FakeConfirm {
    answer: bool,
    asked: AtomicUsize,
}

impl Confirm for FakeConfirm {
    fn confirm(&self, _title: &str, _message: &str, _action: &str) -> bool {
        self.asked.fetch_add(1, Ordering::SeqCst);
        self.answer
    }
}

/// Repository con un commit e il progetto fidato "p" che lo contiene.
fn seeded() -> (tempfile::TempDir, tempfile::TempDir, Store, String) {
    let d = repo();
    std::fs::write(d.path().join("a.txt"), "uno\n").unwrap();
    commit_all(d.path(), "init");
    let (sd, st) = project(&[d.path()]);
    let r = root(&d);
    (d, sd, st, r)
}

#[test]
fn missing_gh_has_install_hint() {
    let env: HashMap<String, String> = [("PATH".to_string(), "/nonexistent".to_string()), ("HOME".to_string(), "/Users/dev".to_string())].into();
    assert_eq!(Gh::locate_in(&env, &[]).err().as_deref(), Some(GH_MISSING));
    assert_eq!(GH_MISSING, "GitHub CLI (`gh`) non trovata: `brew install gh`");
}

#[test]
fn accounts_from_json() {
    let f = fake();
    let list = accounts(&f.gh).unwrap();
    assert_eq!(list, vec![GhAccount { login: "octo-user".into(), active: true }, GhAccount { login: "octo-work".into(), active: false }]);
    let (args, tok) = f.call("auth", "status");
    assert_eq!(args, v(&["auth", "status", "--hostname", "github.com", "--json", "hosts"]));
    assert_eq!(tok, None);
}

#[test]
fn accounts_from_text_when_json_is_unsupported() {
    let f = fake_with("text_auth");
    let list = accounts(&f.gh).unwrap();
    assert_eq!(list, vec![GhAccount { login: "octo-user".into(), active: true }, GhAccount { login: "octo-work".into(), active: false }]);
    // vecchio formato, un solo account: è l'attivo
    assert_eq!(parse_accounts_text("github.com\n  ✓ Logged in to github.com as octo-user (oauth_token)\n"), vec![GhAccount { login: "octo-user".into(), active: true }]);
    assert!(parse_accounts_text("You are not logged into any GitHub hosts.").is_empty());
}

#[test]
fn repo_list_args_and_fields() {
    let f = fake();
    let list = repo_list(&f.gh, "", 500).unwrap();
    assert_eq!(
        list[0],
        RemoteRepo { name_with_owner: "octo-user/demo".into(), description: "Progetto di prova".into(), private: true, updated_at: "2026-11-01T10:00:00Z".into() }
    );
    assert_eq!(list[1].description, "");
    assert!(!list[1].private);
    let (args, tok) = f.call("repo", "list");
    assert_eq!(args, v(&["repo", "list", "--limit", "200", "--json", "nameWithOwner,description,isPrivate,updatedAt"]));
    assert_eq!(tok, None);
    // l'account attivo: nessun token, nessuna `auth token`
    repo_list(&f.gh, "octo-user", 0).unwrap();
    let calls = f.calls();
    let last = calls.last().unwrap();
    assert_eq!(last.0, v(&["repo", "list", "octo-user", "--limit", "1", "--json", "nameWithOwner,description,isPrivate,updatedAt"]));
    assert_eq!(last.1, None);
    assert!(calls.iter().all(|(a, _)| a[..2] != ["auth", "token"]));
}

#[test]
fn token_only_in_the_child_for_a_non_active_account() {
    let f = fake();
    repo_list(&f.gh, "octo-work", 10).unwrap();
    let (args, tok) = f.call("auth", "token");
    assert_eq!(args, v(&["auth", "token", "--hostname", "github.com", "--user", "octo-work"]));
    assert_eq!(tok, None);
    let (args, tok) = f.call("repo", "list");
    assert_eq!(args[2], "octo-work");
    assert_eq!(tok.as_deref(), Some(TOKEN));
    // mai `gh auth switch`
    assert!(f.calls().iter().all(|(a, _)| !a.contains(&"switch".to_string())));
    assert!(repo_list(&f.gh, "nessuno", 10).unwrap_err().contains("«nessuno»"));
    assert!(repo_list(&f.gh, "--user", 10).is_err());
}

#[test]
fn token_never_in_debug_or_errors() {
    assert!(!format!("{:?}", Token(TOKEN.into())).contains(TOKEN));
    let f = fake_with("fail");
    let e = repo_list(&f.gh, "octo-work", 10).unwrap_err();
    // il token è arrivato davvero al figlio, che lo ha ripetuto su stderr
    assert_eq!(f.call("repo", "list").1.as_deref(), Some(TOKEN));
    for shown in [e.clone(), format!("{e:?}")] {
        assert!(!shown.contains(TOKEN), "{shown}");
        assert!(!shown.contains("/Users/dev"), "{shown}");
        assert!(!shown.contains("gho_"), "{shown}");
    }
    assert!(e.starts_with("gh: HTTP 401"), "{e}");
    assert!(e.contains("~/.config/gh/hosts.yml"), "{e}");
}

#[test]
fn clean_hides_tokens_and_home_paths() {
    let f = fake();
    let t = Token("segreto123".into());
    let s = f.gh.clean("a segreto123 b /Users/dev/x /Users/altro/y ghp_abcDEF123 github_pat_11AB_cd fine", Some(&t));
    assert_eq!(s, "a *** b ~/x ~/y *** *** fine");
}

#[test]
fn parse_errors_are_cleaned() {
    let f = fake();
    // serde cita i valori nei messaggi di tipo sbagliato
    let e = f.gh.parse::<Vec<u64>>(b"[\"/Users/dev/ghp_abc123\"]").unwrap_err();
    assert!(e.starts_with("gh: risposta non valida"), "{e}");
    assert!(!e.contains("/Users/dev") && !e.contains("ghp_abc"), "{e}");
}

#[test]
fn login_env_tokens_are_hidden_too() {
    let mut f = fake();
    f.gh.env.insert("GH_TOKEN".into(), "senzaprefisso1".into());
    f.gh.env.insert("GITHUB_TOKEN".into(), "senzaprefisso2".into());
    assert_eq!(f.gh.clean("a senzaprefisso1 b senzaprefisso2", None), "a *** b ***");
}

#[test]
fn malformed_json_is_a_readable_error() {
    let (_d, _sd, st, r) = seeded();
    let f = fake_with("bad_json");
    let e = pr_list(&f.gh, &st, "p", &r, "open").unwrap_err();
    assert!(e.starts_with("gh: risposta non valida"), "{e}");
    assert!(repo_list(&f.gh, "", 10).unwrap_err().starts_with("gh: risposta non valida"));
    assert!(accounts(&f.gh).unwrap_err().starts_with("gh: risposta non valida"));
}

#[test]
fn clone_args() {
    let f = fake();
    let parent = tempfile::tempdir().unwrap();
    let dest = parent.path().canonicalize().unwrap().join("demo");
    clone(&f.gh, "", "octo-user/demo", &dest).unwrap();
    assert!(dest.is_dir());
    let (args, tok) = f.call("repo", "clone");
    assert_eq!(args, v(&["repo", "clone", "octo-user/demo", dest.to_str().unwrap()]));
    assert_eq!(tok, None);
    // la destinazione esiste già
    assert!(clone(&f.gh, "", "octo-user/demo", &dest).unwrap_err().contains("esiste già"));
    for bad in ["-x/y", "a/b/c", "../x", "octo", "a/..", "a/-b"] {
        assert!(clone(&f.gh, "", bad, &parent.path().join("z")).is_err(), "{bad}");
    }
}

#[test]
fn repo_create_args() {
    let (_d, _sd, st, r) = seeded();
    let f = fake();
    let url = repo_create(&f.gh, &st, "p", &r, "demo", true, "Prova").unwrap();
    assert_eq!(url, "https://github.com/octo-user/demo");
    let (args, tok) = f.call("repo", "create");
    assert_eq!(args, v(&["repo", "create", "demo", "--source", &r, "--remote", "origin", "--push", "--private", "--description=Prova"]));
    assert_eq!(tok, None);
    let f = fake();
    repo_create(&f.gh, &st, "p", &r, "octo-org/demo.js", false, "  ").unwrap();
    assert_eq!(f.call("repo", "create").0, v(&["repo", "create", "octo-org/demo.js", "--source", &r, "--remote", "origin", "--push", "--public"]));
    for bad in ["", "-x", "a b", "..", "a/b/c", "x;y"] {
        assert!(repo_create(&f.gh, &st, "p", &r, bad, true, "").is_err(), "{bad}");
    }
    assert!(repo_create(&f.gh, &st, "p", &r, "demo", true, "riga\nnuova").is_err());
}

#[test]
fn pr_list_args_and_check_summary() {
    let (_d, _sd, st, r) = seeded();
    let f = fake();
    let list = pr_list(&f.gh, &st, "p", &r, "open").unwrap();
    assert_eq!(f.call("pr", "list").0, v(&["pr", "list", "--state", "open", "--limit", "100", "--json", PR_JSON]));
    assert_eq!(
        list[0],
        Pr {
            number: 7,
            title: "Aggiunge il login".into(),
            author: "octo-user".into(),
            head: "feature/login".into(),
            base: "main".into(),
            state: "open".into(),
            is_draft: false,
            url: "https://github.com/octo-org/demo/pull/7".into(),
            checks: ChecksSummary { passing: 2, failing: 1, pending: 1 },
        }
    );
    assert!(list[1].is_draft);
    assert_eq!(list[1].checks, ChecksSummary::default());
    assert!(pr_list(&f.gh, &st, "p", &r, "merged").is_err());
}

#[test]
fn project_account_setting_selects_the_token() {
    let (_d, _sd, st, r) = seeded();
    let f = fake();
    crate::projects::project_setting_set_for(&st, "p", ACCOUNT_KEY, "octo-work").unwrap();
    pr_list(&f.gh, &st, "p", &r, "all").unwrap();
    assert_eq!(f.call("pr", "list").1.as_deref(), Some(TOKEN));
    crate::projects::project_setting_set_for(&st, "p", ACCOUNT_KEY, "").unwrap();
    let f = fake();
    pr_list(&f.gh, &st, "p", &r, "closed").unwrap();
    assert_eq!(f.call("pr", "list").1, None);
    assert!(f.calls().iter().all(|(a, _)| a[0] != "auth"));
}

#[test]
fn pr_checks_args_and_pending_exit_code() {
    let (_d, _sd, st, r) = seeded();
    let f = fake();
    let list = pr_checks(&f.gh, &st, "p", &r, 7).unwrap();
    assert_eq!(f.call("pr", "checks").0, v(&["pr", "checks", "7", "--json", "name,state,link,bucket"]));
    assert_eq!(
        list,
        vec![
            Check { name: "build".into(), state: "passing".into(), url: "https://github.com/octo-org/demo/actions/runs/1".into() },
            Check { name: "lint".into(), state: "pending".into(), url: "https://github.com/octo-org/demo/actions/runs/3".into() },
            Check { name: "test".into(), state: "failing".into(), url: "https://github.com/octo-org/demo/actions/runs/2".into() },
        ]
    );
    assert!(pr_checks(&f.gh, &st, "p", &r, 0).is_err());
}

/// Repository con origine nuda e un branch `feature` senza upstream.
fn with_origin() -> (tempfile::TempDir, tempfile::TempDir, tempfile::TempDir, Store, String) {
    let origin = tempfile::tempdir().unwrap();
    g(origin.path(), &["init", "-q", "--bare", "-b", "main"]);
    let (d, sd, st, r) = seeded();
    g(d.path(), &["remote", "add", "origin", origin.path().to_str().unwrap()]);
    g(d.path(), &["push", "-q", "-u", "origin", "main"]);
    g(d.path(), &["switch", "-q", "-c", "feature"]);
    std::fs::write(d.path().join("b.txt"), "due\n").unwrap();
    commit_all(d.path(), "feature");
    (origin, d, sd, st, r)
}

#[test]
fn pr_create_pushes_missing_upstream_then_creates() {
    let (origin, d, _sd, st, r) = with_origin();
    let f = fake();
    let pr = pr_create(&f.gh, &st, "p", &r, "Nuova funzione", "Corpo **md**\n", "main", true).unwrap();
    assert_eq!(pr.number, 9);
    assert!(pr.is_draft);
    // il branch è arrivato sull'origine, con l'upstream impostato
    assert_eq!(g_str(origin.path(), &["rev-parse", "feature"]), g_str(d.path(), &["rev-parse", "HEAD"]));
    assert_eq!(g_str(d.path(), &["rev-parse", "--abbrev-ref", "feature@{u}"]), "origin/feature");
    assert_eq!(
        f.call("pr", "create").0,
        v(&["pr", "create", "--title=Nuova funzione", "--body-file", "-", "--head=feature", "--base=main", "--draft"])
    );
    assert_eq!(f.stdin(), "Corpo **md**\n");
    assert_eq!(f.call("pr", "view").0, v(&["pr", "view", "9", "--json", PR_JSON]));
    // con l'upstream già impostato non si invia di nuovo; base vuota = predefinito di GitHub
    let f = fake();
    pr_create(&f.gh, &st, "p", &r, "-titolo", "", "", false).unwrap();
    assert_eq!(f.call("pr", "create").0, v(&["pr", "create", "--title=-titolo", "--body-file", "-", "--head=feature"]));
    for (title, base) in [("", "main"), ("a\nb", "main"), ("ok", "-x"), ("ok", "a b")] {
        assert!(pr_create(&f.gh, &st, "p", &r, title, "", base, false).is_err(), "{title:?} {base:?}");
    }
}

#[test]
fn pr_create_needs_a_branch() {
    let (_d, _sd, st, r) = seeded();
    g(Path::new(&r), &["checkout", "-q", "--detach"]);
    let f = fake();
    assert!(pr_create(&f.gh, &st, "p", &r, "Titolo", "", "", false).unwrap_err().contains("HEAD staccato"));
    assert!(f.calls().iter().all(|(a, _)| a[..2] != ["pr", "create"]));
}

#[test]
fn pr_merge_asks_for_confirmation() {
    let (_d, _sd, st, r) = seeded();
    let f = fake();
    let no = FakeConfirm { answer: false, asked: AtomicUsize::new(0) };
    assert!(!pr_merge(&f.gh, &st, "p", &r, 7, "squash", true, &no).unwrap());
    assert_eq!(no.asked.load(Ordering::SeqCst), 1);
    assert!(f.calls().iter().all(|(a, _)| a[..2] != ["pr", "merge"]));
    let yes = FakeConfirm { answer: true, asked: AtomicUsize::new(0) };
    assert!(pr_merge(&f.gh, &st, "p", &r, 7, "squash", true, &yes).unwrap());
    assert_eq!(f.call("pr", "merge").0, v(&["pr", "merge", "7", "--squash", "--delete-branch"]));
    let f = fake();
    assert!(pr_merge(&f.gh, &st, "p", &r, 8, "rebase", false, &yes).unwrap());
    assert_eq!(f.call("pr", "merge").0, v(&["pr", "merge", "8", "--rebase"]));
    // metodo sconosciuto: rifiutato prima di chiedere
    let asked = yes.asked.load(Ordering::SeqCst);
    assert!(pr_merge(&f.gh, &st, "p", &r, 7, "--admin", false, &yes).is_err());
    assert_eq!(yes.asked.load(Ordering::SeqCst), asked);
}

#[test]
fn refused_when_untrusted() {
    let d = repo();
    let dir = tempfile::tempdir().unwrap();
    let s = Store::open(&dir.path().join("t.db")).unwrap();
    let r = root(&d);
    s.upsert_project(&crate::store::Project { id: "p".into(), name: "p".into(), folders: vec![r.clone()] }).unwrap();
    let f = fake();
    let yes = FakeConfirm { answer: true, asked: AtomicUsize::new(0) };
    assert_eq!(pr_list(&f.gh, &s, "p", &r, "open").unwrap_err(), crate::trust::UNTRUSTED);
    assert_eq!(pr_checks(&f.gh, &s, "p", &r, 1).unwrap_err(), crate::trust::UNTRUSTED);
    assert_eq!(pr_create(&f.gh, &s, "p", &r, "t", "", "", false).unwrap_err(), crate::trust::UNTRUSTED);
    assert_eq!(pr_merge(&f.gh, &s, "p", &r, 1, "merge", false, &yes).unwrap_err(), crate::trust::UNTRUSTED);
    assert_eq!(repo_create(&f.gh, &s, "p", &r, "demo", true, "").unwrap_err(), crate::trust::UNTRUSTED);
    assert_eq!(remote(&s, "p", &r).unwrap_err(), crate::trust::UNTRUSTED);
    assert!(f.calls().is_empty());
    assert_eq!(yes.asked.load(Ordering::SeqCst), 0);
}

#[test]
fn github_remote_detection() {
    for url in ["https://github.com/octo-org/demo.git", "https://github.com/octo-org/demo", "git@github.com:octo-org/demo.git", "ssh://git@github.com/octo-org/demo.git", "https://octo-user@github.com/octo-org/demo/"] {
        assert_eq!(parse_github_remote(url).as_deref(), Some("octo-org/demo"), "{url}");
    }
    for url in ["https://gitlab.com/octo-org/demo.git", "/Users/dev/origin", "https://github.com/octo-org", "https://github.com.evil.invalid/a/b", "git@github.com:a/b/c"] {
        assert_eq!(parse_github_remote(url), None, "{url}");
    }
    let (d, _sd, st, r) = seeded();
    assert_eq!(remote(&st, "p", &r).unwrap(), GhRemote { origin: false, github: None });
    g(d.path(), &["remote", "add", "origin", "https://gitlab.example.invalid/octo-org/demo.git"]);
    // un `origin` non GitHub: niente "Pubblica su GitHub" (gh repo create fallirebbe)
    assert_eq!(remote(&st, "p", &r).unwrap(), GhRemote { origin: true, github: None });
    g(d.path(), &["remote", "set-url", "origin", "git@github.com:octo-org/demo.git"]);
    assert_eq!(remote(&st, "p", &r).unwrap(), GhRemote { origin: true, github: Some("octo-org/demo".into()) });
}

#[test]
fn account_values() {
    for ok in ["", "octo-user", "a", "A1-b2"] {
        assert!(valid_account(ok), "{ok}");
    }
    for bad in ["-x", "a b", "a/b", "a_b", &"x".repeat(40)] {
        assert!(!valid_account(bad), "{bad}");
    }
}
