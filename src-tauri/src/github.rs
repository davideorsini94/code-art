//! GitHub tramite la CLI `gh`: account, elenco e clone dei repository, pubblicazione, pull request.
//! `gh` parte sempre con lista di argomenti (`run_tool`: sessione propria, timeout, output limitato), senza
//! domande (`GH_PROMPT_DISABLED`) e con i JSON letti su campi espliciti (`--json` con elenco di campi).
//! Account: quello attivo di `gh`, oppure per progetto l'impostazione `github_account`; per un account non attivo
//! il token (`gh auth token --user`) va solo nell'ambiente del figlio come `GH_TOKEN`. Mai `gh auth switch`.
//! Gli errori mostrati non contengono token né percorsi della home.
use crate::git::cmd::{git, READ_TIMEOUT, REMOTE_TIMEOUT};
use crate::git::resolve_repo;
use crate::git::write::{lock_repo, Confirm, NativeConfirm};
use crate::store::{Result, Store};
use crate::tools::{command_in, run_tool, ToolOutput};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;
use tauri::{AppHandle, Manager, WebviewWindow};

pub const GH_MISSING: &str = "GitHub CLI (`gh`) non trovata: `brew install gh`";
/// Impostazione di progetto: login dell'account da usare (vuoto = account attivo di `gh`).
pub const ACCOUNT_KEY: &str = "github_account";
const HOST: &str = "github.com";
/// Timeout delle chiamate all'API (le operazioni con push o clone usano `REMOTE_TIMEOUT`).
const GH_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_REPO_LIST: usize = 200;
const PR_LIMIT: &str = "100";
const PR_STATES: &[&str] = &["open", "closed", "all"];
const PR_FIELDS: &str = "number,title,author,headRefName,baseRefName,state,isDraft,url,statusCheckRollup";
const REPO_FIELDS: &str = "nameWithOwner,description,isPrivate,updatedAt";
const CHECK_FIELDS: &str = "name,state,link,bucket";
const MAX_TITLE_CHARS: usize = 256;
const MAX_BODY_BYTES: usize = 64 * 1024;
const MAX_DESCRIPTION_CHARS: usize = 350;
/// Prefissi dei token di GitHub, nascosti in ogni messaggio anche se non sono quello in uso.
const TOKEN_PREFIXES: &[&str] = &["github_pat_", "ghp_", "gho_", "ghu_", "ghs_", "ghr_"];

/// Token di un account: mai in `Debug`, mai in un messaggio.
pub struct Token(pub(crate) String);

impl std::fmt::Debug for Token {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Token(***)")
    }
}

/// Binario `gh` e ambiente dei figli (quello di login). Niente `Debug`: l'ambiente contiene i segreti dell'utente.
pub struct Gh {
    bin: PathBuf,
    env: HashMap<String, String>,
}

impl Gh {
    pub fn locate() -> Result<Gh> {
        Self::locate_in(crate::tools::env(), crate::tools::SYSTEM_DIRS)
    }

    pub fn locate_in(env: &HashMap<String, String>, system_dirs: &[&str]) -> Result<Gh> {
        let bin = crate::tools::find_tool_in(env, "gh", system_dirs).ok_or(GH_MISSING)?;
        Ok(Gh { bin, env: env.clone() })
    }

    #[cfg(test)]
    pub fn at(bin: PathBuf, env: HashMap<String, String>) -> Gh {
        Gh { bin, env }
    }

    fn command(&self, cwd: &Path, token: Option<&Token>) -> Command {
        let mut cmd = command_in(&self.bin, &self.env);
        cmd.current_dir(cwd)
            .env("GH_PROMPT_DISABLED", "1")
            .env("GH_NO_UPDATE_NOTIFIER", "1")
            .env("GH_SPINNER_DISABLED", "1")
            .env("NO_COLOR", "1")
            .env("GIT_TERMINAL_PROMPT", "0");
        if let Some(t) = token {
            cmd.env("GH_TOKEN", &t.0);
        }
        cmd
    }

    fn run(&self, cwd: &Path, token: Option<&Token>, args: &[&str], input: Option<Vec<u8>>, timeout: Duration) -> Result<ToolOutput> {
        let mut cmd = self.command(cwd, token);
        cmd.args(args);
        run_tool(cmd, timeout, input).map_err(|e| format!("gh: {}", self.clean(&e, token)))
    }

    /// Come `run`, ma un'uscita diversa da 0 è un errore con il messaggio (ripulito) di `gh`.
    fn run_ok(&self, cwd: &Path, token: Option<&Token>, args: &[&str], input: Option<Vec<u8>>, timeout: Duration) -> Result<Vec<u8>> {
        let out = self.run(cwd, token, args, input, timeout)?;
        if out.status == 0 {
            Ok(out.stdout)
        } else {
            Err(self.failure(&out, token))
        }
    }

    fn failure(&self, out: &ToolOutput, token: Option<&Token>) -> String {
        let why = out.stderr.trim();
        if why.is_empty() {
            format!("gh: uscita {}", out.status)
        } else {
            format!("gh: {}", self.clean(why, token))
        }
    }

    /// JSON di `gh`; l'errore di serde può citare valori della risposta, quindi passa da `clean`.
    pub fn parse<T: DeserializeOwned>(&self, bytes: &[u8]) -> Result<T> {
        serde_json::from_slice(bytes).map_err(|e| format!("gh: risposta non valida ({})", self.clean(&e.to_string(), None)))
    }

    /// Testo da mostrare: senza il token in uso, senza altri token di GitHub, senza percorsi della home.
    pub fn clean(&self, text: &str, token: Option<&Token>) -> String {
        let mut s = text.trim().to_string();
        if let Some(t) = token.filter(|t| !t.0.is_empty()) {
            s = s.replace(&t.0, "***");
        }
        // token già nell'ambiente di login, anche senza prefisso
        for k in ["GH_TOKEN", "GITHUB_TOKEN"] {
            if let Some(t) = self.env.get(k).filter(|t| t.len() >= 8) {
                s = s.replace(t.as_str(), "***");
            }
        }
        s = hide_user_dirs(&hide_token_words(&s));
        // una home fuori da /Users
        match self.env.get("HOME").map(|h| h.trim_end_matches('/')).filter(|h| h.len() > 1) {
            Some(home) => s.replace(home, "~"),
            None => s,
        }
    }
}

/// Nasconde le parole che iniziano con un prefisso di token di GitHub.
fn hide_token_words(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some((at, p)) = TOKEN_PREFIXES.iter().filter_map(|p| rest.find(p).map(|i| (i, *p))).min_by_key(|(i, _)| *i) {
        out.push_str(&rest[..at]);
        out.push_str("***");
        let tail = &rest[at + p.len()..];
        let end = tail.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).unwrap_or(tail.len());
        rest = &tail[end..];
    }
    out.push_str(rest);
    out
}

/// `/Users/<nome>` → `~` (anche per una home diversa da quella dell'ambiente).
fn hide_user_dirs(s: &str) -> String {
    const USERS: &str = "/Users/";
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(at) = rest.find(USERS) {
        out.push_str(&rest[..at]);
        let tail = &rest[at + USERS.len()..];
        let end = tail.find(|c: char| c == '/' || c.is_whitespace() || "'\"`)]>,;:".contains(c)).unwrap_or(tail.len());
        out.push('~');
        rest = &tail[end..];
    }
    out.push_str(rest);
    out
}



fn last_url(stdout: &[u8]) -> Option<String> {
    String::from_utf8_lossy(stdout).lines().map(str::trim).rfind(|l| l.starts_with("https://")).map(String::from)
}

// ---------- tipi ----------

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GhAccount {
    pub login: String,
    pub active: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteRepo {
    pub name_with_owner: String,
    pub description: String,
    pub private: bool,
    pub updated_at: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChecksSummary {
    pub passing: u32,
    pub failing: u32,
    pub pending: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Pr {
    pub number: u64,
    pub title: String,
    pub author: String,
    pub head: String,
    pub base: String,
    /// `open` | `closed` | `merged`.
    pub state: String,
    pub is_draft: bool,
    pub url: String,
    pub checks: ChecksSummary,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Check {
    pub name: String,
    /// `passing` | `failing` | `pending` | `skipped`.
    pub state: String,
    pub url: String,
}

#[derive(Deserialize)]
struct RawAuthStatus {
    hosts: HashMap<String, Vec<RawAccount>>,
}

#[derive(Deserialize)]
struct RawAccount {
    login: String,
    #[serde(default)]
    active: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawRepo {
    name_with_owner: String,
    description: Option<String>,
    is_private: bool,
    updated_at: String,
}

#[derive(Deserialize)]
struct RawAuthor {
    login: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawPr {
    number: u64,
    title: String,
    author: Option<RawAuthor>,
    head_ref_name: String,
    base_ref_name: String,
    state: String,
    is_draft: bool,
    url: String,
    status_check_rollup: Option<Vec<RawRollup>>,
}

/// Voce di `statusCheckRollup`: `CheckRun` (status/conclusion) o `StatusContext` (state).
#[derive(Deserialize)]
struct RawRollup {
    #[serde(rename = "__typename", default)]
    typename: String,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    conclusion: Option<String>,
    #[serde(default)]
    state: Option<String>,
}

#[derive(Deserialize)]
struct RawCheck {
    name: String,
    #[serde(default)]
    bucket: String,
    #[serde(default)]
    link: String,
}

#[derive(Clone, Copy)]
enum Outcome {
    Passing,
    Failing,
    Pending,
}

fn rollup_outcome(r: &RawRollup) -> Outcome {
    let up = |s: &Option<String>| s.as_deref().unwrap_or("").to_ascii_uppercase();
    if r.typename == "StatusContext" || (r.status.is_none() && r.state.is_some()) {
        return match up(&r.state).as_str() {
            "SUCCESS" => Outcome::Passing,
            "FAILURE" | "ERROR" => Outcome::Failing,
            _ => Outcome::Pending,
        };
    }
    if up(&r.status) != "COMPLETED" {
        return Outcome::Pending;
    }
    match up(&r.conclusion).as_str() {
        "SUCCESS" | "NEUTRAL" | "SKIPPED" => Outcome::Passing,
        "" => Outcome::Pending,
        _ => Outcome::Failing,
    }
}

impl From<RawPr> for Pr {
    fn from(r: RawPr) -> Pr {
        let mut checks = ChecksSummary::default();
        for c in r.status_check_rollup.iter().flatten() {
            match rollup_outcome(c) {
                Outcome::Passing => checks.passing += 1,
                Outcome::Failing => checks.failing += 1,
                Outcome::Pending => checks.pending += 1,
            }
        }
        Pr {
            number: r.number,
            title: r.title,
            author: r.author.map(|a| a.login).unwrap_or_default(),
            head: r.head_ref_name,
            base: r.base_ref_name,
            state: r.state.to_ascii_lowercase(),
            is_draft: r.is_draft,
            url: r.url,
            checks,
        }
    }
}

// ---------- validazione ----------

/// Login di GitHub (1–39 caratteri alfanumerici o `-`, mai `-` iniziale) oppure vuoto (= account attivo).
pub fn valid_account(s: &str) -> bool {
    s.is_empty() || valid_login(s)
}

fn valid_login(s: &str) -> bool {
    !s.is_empty() && s.len() <= 39 && !s.starts_with('-') && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// Nome di repository: lettere, cifre, `.`, `_`, `-`; mai `-` iniziale né `.`/`..`.
fn valid_repo_name(s: &str) -> bool {
    !s.is_empty() && s.len() <= 100 && !s.starts_with('-') && s != "." && s != ".." && s.chars().all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
}

/// `proprietario/nome`.
fn check_name_with_owner(s: &str) -> Result<(&str, &str)> {
    match s.split_once('/') {
        Some((o, n)) if valid_login(o) && valid_repo_name(n) => Ok((o, n)),
        _ => Err("repository GitHub non valido (atteso proprietario/nome)".into()),
    }
}

/// `nome` oppure `proprietario/nome` (per `gh repo create`).
fn check_new_repo_name(s: &str) -> Result<()> {
    let ok = match s.split_once('/') {
        Some(_) => check_name_with_owner(s).is_ok(),
        None => valid_repo_name(s),
    };
    if ok { Ok(()) } else { Err("nome del repository non valido: lettere, cifre, «.», «_» e «-»".into()) }
}

fn has_control(s: &str) -> bool {
    s.chars().any(char::is_control)
}

fn check_title(t: &str) -> Result<()> {
    if t.trim().is_empty() || t.chars().count() > MAX_TITLE_CHARS || has_control(t) {
        return Err(format!("titolo non valido: una riga, al massimo {MAX_TITLE_CHARS} caratteri"));
    }
    Ok(())
}

fn check_body(b: &str) -> Result<()> {
    if b.len() > MAX_BODY_BYTES || b.contains('\0') {
        return Err("descrizione della pull request troppo lunga".into());
    }
    Ok(())
}

/// Branch di destinazione: vuoto (= predefinito del repository) o un nome senza spazi né `-` iniziale.
fn check_base(b: &str) -> Result<()> {
    if b.is_empty() {
        return Ok(());
    }
    if b.len() > 255 || b.starts_with('-') || b.contains("..") || b.chars().any(|c| c.is_whitespace() || c.is_control() || "~^:?*[\\".contains(c)) {
        return Err("branch di destinazione non valido".into());
    }
    Ok(())
}

fn check_description(d: &str) -> Result<()> {
    if d.chars().count() > MAX_DESCRIPTION_CHARS || has_control(d) {
        return Err(format!("descrizione non valida: una riga, al massimo {MAX_DESCRIPTION_CHARS} caratteri"));
    }
    Ok(())
}

// ---------- account ----------

/// Account del testo di `gh auth status` (versioni senza `--json`): "Logged in to github.com account X" o
/// "… as X", seguiti da "Active account: true|false"; senza quella riga (un solo account) è l'attivo.
pub fn parse_accounts_text(text: &str) -> Vec<GhAccount> {
    let mut list: Vec<GhAccount> = Vec::new();
    let mut saw_active_line = false;
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.split_once(&format!("Logged in to {HOST} ")).map(|(_, r)| r) {
            let rest = rest.strip_prefix("account ").or_else(|| rest.strip_prefix("as ")).unwrap_or(rest);
            let login = rest.split_whitespace().next().unwrap_or("");
            if valid_login(login) {
                list.push(GhAccount { login: login.into(), active: false });
            }
        } else if let Some(v) = line.strip_prefix("- Active account:") {
            saw_active_line = true;
            if let Some(last) = list.last_mut() {
                last.active = v.trim() == "true";
            }
        }
    }
    if let (false, Some(first)) = (saw_active_line, list.first_mut()) {
        first.active = true;
    }
    list
}

/// Account di `gh` su github.com (da `gh auth status --json hosts`, altrimenti dal testo).
pub fn accounts(gh: &Gh) -> Result<Vec<GhAccount>> {
    let cwd = Path::new("/");
    let out = gh.run(cwd, None, &["auth", "status", "--hostname", HOST, "--json", "hosts"], None, GH_TIMEOUT)?;
    if out.status == 0 {
        let status: RawAuthStatus = gh.parse(&out.stdout)?;
        return Ok(status
            .hosts
            .get(HOST)
            .into_iter()
            .flatten()
            .filter(|a| valid_login(&a.login))
            .map(|a| GhAccount { login: a.login.clone(), active: a.active })
            .collect());
    }
    if !out.stderr.contains("unknown flag") {
        return Err(gh.failure(&out, None));
    }
    let out = gh.run(cwd, None, &["auth", "status", "--hostname", HOST], None, GH_TIMEOUT)?;
    let text = format!("{}\n{}", String::from_utf8_lossy(&out.stdout), out.stderr);
    let list = parse_accounts_text(&text);
    if list.is_empty() && out.status != 0 && !text.contains("not logged") {
        return Err(gh.failure(&out, None));
    }
    Ok(list)
}

/// Token per `account`: `None` per l'account attivo (o vuoto), altrimenti `gh auth token --user`.
fn token_for(gh: &Gh, account: &str) -> Result<Option<Token>> {
    if account.is_empty() {
        return Ok(None);
    }
    if !valid_login(account) {
        return Err("account GitHub non valido".into());
    }
    match accounts(gh)?.into_iter().find(|a| a.login == account) {
        None => Err(format!("L'account GitHub «{account}» non è collegato a gh (gh auth login)")),
        Some(a) if a.active => Ok(None),
        Some(_) => {
            let out = gh.run_ok(Path::new("/"), None, &["auth", "token", "--hostname", HOST, "--user", account], None, GH_TIMEOUT)?;
            let token = String::from_utf8_lossy(&out).trim().to_string();
            if token.is_empty() {
                return Err(format!("gh: nessun token per «{account}»"));
            }
            Ok(Some(Token(token)))
        }
    }
}

/// Token per l'account scelto nel progetto (impostazione `github_account`).
fn project_token(gh: &Gh, store: &Store, project_id: &str) -> Result<Option<Token>> {
    let account = crate::projects::project_setting_get_for(store, project_id, ACCOUNT_KEY)?.unwrap_or_default();
    token_for(gh, &account)
}

// ---------- repository ----------

/// `proprietario/nome` se `url` punta a github.com (https, ssh o forma `git@github.com:`).
pub fn parse_github_remote(url: &str) -> Option<String> {
    let url = url.trim();
    let (host, path) = if let Some(rest) = ["https://", "http://", "ssh://"].iter().find_map(|s| url.strip_prefix(s)) {
        rest.split_once('/')?
    } else {
        url.split_once(':')?
    };
    let host = host.rsplit('@').next()?;
    if !host.eq_ignore_ascii_case(HOST) {
        return None;
    }
    let path = path.trim_end_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    check_name_with_owner(path).ok().map(|(o, n)| format!("{o}/{n}"))
}

/// Remoto `origin`: se esiste e, se è su github.com, il suo `proprietario/nome`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GhRemote {
    pub origin: bool,
    pub github: Option<String>,
}

pub fn remote(store: &Store, project_id: &str, repo: &str) -> Result<GhRemote> {
    let dir = resolve_repo(store, project_id, repo)?;
    let out = git(&dir, &["remote", "get-url", "origin"], READ_TIMEOUT)?;
    if out.status != 0 {
        return Ok(GhRemote { origin: false, github: None });
    }
    Ok(GhRemote { origin: true, github: parse_github_remote(&String::from_utf8_lossy(&out.stdout)) })
}

/// Repository di `account` (vuoto = account attivo), al massimo 200.
pub fn repo_list(gh: &Gh, account: &str, limit: usize) -> Result<Vec<RemoteRepo>> {
    let token = token_for(gh, account)?;
    let n = limit.clamp(1, MAX_REPO_LIST).to_string();
    let mut args = vec!["repo", "list"];
    if !account.is_empty() {
        args.push(account);
    }
    args.extend(["--limit", &n, "--json", REPO_FIELDS]);
    let out = gh.run_ok(Path::new("/"), token.as_ref(), &args, None, GH_TIMEOUT)?;
    let raw: Vec<RawRepo> = gh.parse(&out)?;
    Ok(raw
        .into_iter()
        .map(|r| RemoteRepo { name_with_owner: r.name_with_owner, description: r.description.unwrap_or_default(), private: r.is_private, updated_at: r.updated_at })
        .collect())
}

/// `gh repo clone` in `dest` (che non deve esistere; la cartella che lo contiene sì).
pub fn clone(gh: &Gh, account: &str, name_with_owner: &str, dest: &Path) -> Result<()> {
    check_name_with_owner(name_with_owner)?;
    let name = dest.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    if dest.exists() {
        return Err(format!("La cartella «{name}» esiste già in quella posizione"));
    }
    let parent = dest.parent().filter(|p| p.is_dir() && dest.is_absolute()).ok_or("cartella di destinazione non valida")?;
    let token = token_for(gh, account)?;
    let dest_s = dest.to_string_lossy();
    gh.run_ok(parent, token.as_ref(), &["repo", "clone", name_with_owner, &dest_s], None, REMOTE_TIMEOUT).map(drop)
}

/// Crea il repository su GitHub da `repo` (remoto `origin`, push). Restituisce l'URL.
pub fn repo_create(gh: &Gh, store: &Store, project_id: &str, repo: &str, name: &str, private: bool, description: &str) -> Result<String> {
    let dir = resolve_repo(store, project_id, repo)?;
    check_new_repo_name(name)?;
    check_description(description)?;
    let token = project_token(gh, store, project_id)?;
    let _g = lock_repo(&dir)?;
    let src = dir.to_string_lossy();
    let mut args = vec!["repo", "create", name, "--source", &src, "--remote", "origin", "--push", if private { "--private" } else { "--public" }];
    let desc = format!("--description={}", description.trim());
    if !description.trim().is_empty() {
        args.push(&desc);
    }
    let out = gh.run_ok(&dir, token.as_ref(), &args, None, REMOTE_TIMEOUT)?;
    last_url(&out).ok_or_else(|| "gh: URL del repository non ricevuto".into())
}

// ---------- pull request ----------

pub fn pr_list(gh: &Gh, store: &Store, project_id: &str, repo: &str, state: &str) -> Result<Vec<Pr>> {
    let dir = resolve_repo(store, project_id, repo)?;
    if !PR_STATES.contains(&state) {
        return Err("stato delle pull request non valido".into());
    }
    let token = project_token(gh, store, project_id)?;
    let out = gh.run_ok(&dir, token.as_ref(), &["pr", "list", "--state", state, "--limit", PR_LIMIT, "--json", PR_FIELDS], None, GH_TIMEOUT)?;
    let raw: Vec<RawPr> = gh.parse(&out)?;
    Ok(raw.into_iter().map(Pr::from).collect())
}

fn pr_view(gh: &Gh, dir: &Path, token: Option<&Token>, number: u64) -> Result<Pr> {
    let n = number.to_string();
    let out = gh.run_ok(dir, token, &["pr", "view", &n, "--json", PR_FIELDS], None, GH_TIMEOUT)?;
    Ok(gh.parse::<RawPr>(&out)?.into())
}

/// Controlli di una pull request. `gh pr checks` esce con 8 se ci sono controlli in corso e 1 se qualcuno
/// fallisce: conta il JSON, quando c'è.
pub fn pr_checks(gh: &Gh, store: &Store, project_id: &str, repo: &str, number: u64) -> Result<Vec<Check>> {
    let dir = resolve_repo(store, project_id, repo)?;
    if number == 0 {
        return Err("numero di pull request non valido".into());
    }
    let token = project_token(gh, store, project_id)?;
    let n = number.to_string();
    let out = gh.run(&dir, token.as_ref(), &["pr", "checks", &n, "--json", CHECK_FIELDS], None, GH_TIMEOUT)?;
    if out.stdout.iter().all(u8::is_ascii_whitespace) {
        return if out.status == 0 || out.stderr.contains("no checks reported") { Ok(Vec::new()) } else { Err(gh.failure(&out, token.as_ref())) };
    }
    let raw: Vec<RawCheck> = gh.parse(&out.stdout)?;
    Ok(raw
        .into_iter()
        .map(|c| {
            let state = match c.bucket.as_str() {
                "pass" => "passing",
                "fail" | "cancel" => "failing",
                "skipping" => "skipped",
                _ => "pending",
            };
            Check { name: c.name, state: state.into(), url: c.link }
        })
        .collect())
}

/// Crea una pull request dal branch corrente (inviato prima, se non ha upstream). `base` vuota = branch
/// predefinito del repository. Corpo da stdin (`--body-file -`).
#[allow(clippy::too_many_arguments)]
pub fn pr_create(gh: &Gh, store: &Store, project_id: &str, repo: &str, title: &str, body: &str, base: &str, draft: bool) -> Result<Pr> {
    let dir = resolve_repo(store, project_id, repo)?;
    check_title(title)?;
    check_body(body)?;
    check_base(base)?;
    let token = project_token(gh, store, project_id)?;
    let _g = lock_repo(&dir)?;
    let head = git(&dir, &["symbolic-ref", "--quiet", "--short", "HEAD"], READ_TIMEOUT)?;
    if head.status != 0 {
        return Err("HEAD staccato: passa a un branch prima di creare la pull request".into());
    }
    let branch = String::from_utf8_lossy(&head.stdout).trim_end_matches('\n').to_string();
    let upstream = git(&dir, &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"], READ_TIMEOUT)?;
    if upstream.status != 0 {
        // limite: il push usa le credenziali git dell'utente (helper/portachiavi), non l'account `github_account`
        crate::git::remote::push_blocking(store, &dir).map_err(|e| gh.clean(&e, None))?;
    }
    let (t, h, b) = (format!("--title={title}"), format!("--head={branch}"), format!("--base={base}"));
    let mut args = vec!["pr", "create", &t, "--body-file", "-", &h];
    if !base.is_empty() {
        args.push(&b);
    }
    if draft {
        args.push("--draft");
    }
    let out = gh.run_ok(&dir, token.as_ref(), &args, Some(body.as_bytes().to_vec()), GH_TIMEOUT)?;
    let url = last_url(&out).ok_or("gh: URL della pull request non ricevuto")?;
    let number = url.rsplit_once("/pull/").and_then(|(_, n)| n.trim_end_matches('/').parse::<u64>().ok()).ok_or("gh: URL della pull request non riconosciuto")?;
    pr_view(gh, &dir, token.as_ref(), number)
}

/// Unisce una pull request dopo la conferma. `Ok(false)` = annullato.
#[allow(clippy::too_many_arguments)]
pub fn pr_merge(gh: &Gh, store: &Store, project_id: &str, repo: &str, number: u64, method: &str, delete_branch: bool, confirm: &dyn Confirm) -> Result<bool> {
    let dir = resolve_repo(store, project_id, repo)?;
    let (flag, label) = match method {
        "merge" => ("--merge", "commit di unione"),
        "squash" => ("--squash", "squash"),
        "rebase" => ("--rebase", "rebase"),
        _ => return Err("metodo di unione non valido".into()),
    };
    if number == 0 {
        return Err("numero di pull request non valido".into());
    }
    let tail = if delete_branch { "\n\nIl branch della pull request verrà eliminato." } else { "" };
    if !confirm.confirm("Unire la pull request?", &format!("Pull request #{number}, metodo: {label}.{tail}"), "Unisci") {
        return Ok(false);
    }
    let token = project_token(gh, store, project_id)?;
    let _g = lock_repo(&dir)?;
    let n = number.to_string();
    let mut args = vec!["pr", "merge", &n, flag];
    if delete_branch {
        args.push("--delete-branch");
    }
    gh.run_ok(&dir, token.as_ref(), &args, None, GH_TIMEOUT)?;
    Ok(true)
}

// ---------- comandi ----------

use crate::projects::{off_main, window_project_id};

/// Account di `gh` (anche dalla finestra di benvenuto: non esegue codice di un progetto).
#[tauri::command]
pub async fn gh_accounts() -> Result<Vec<GhAccount>> {
    off_main(|| accounts(&Gh::locate()?)).await
}

#[tauri::command]
pub async fn gh_repo_list(account: String, limit: usize) -> Result<Vec<RemoteRepo>> {
    off_main(move || repo_list(&Gh::locate()?, &account, limit)).await
}

/// Clona `name_with_owner` in una cartella scelta col selettore nativo e la apre come progetto in una nuova
/// finestra, già fidata (l'ha appena clonata l'utente). `account` vuoto o assente = account attivo.
/// `false` = selettore annullato.
#[tauri::command]
pub async fn gh_clone(app: AppHandle, window: WebviewWindow, name_with_owner: String, account: Option<String>) -> Result<bool> {
    let (_, name) = check_name_with_owner(&name_with_owner)?;
    let name = name.to_string();
    let account = account.unwrap_or_default();
    if !valid_account(&account) {
        return Err("account GitHub non valido".into());
    }
    let title = format!("Dove clonare {name_with_owner}?");
    let Some(parent) = crate::projects::pick_folder(&app, &window, &title).await? else { return Ok(false) };
    let dest = Path::new(&parent).join(&name);
    let dest = off_main(move || {
        clone(&Gh::locate()?, &account, &name_with_owner, &dest)?;
        Ok(dest.to_string_lossy().into_owned())
    })
    .await?;
    crate::trust::mark_trusted(&app.state::<Store>(), &dest)?;
    crate::projects::open_path(&app, &dest)?;
    Ok(true)
}

#[tauri::command]
pub async fn gh_remote(app: AppHandle, window: WebviewWindow, repo: String) -> Result<GhRemote> {
    let id = window_project_id(&window)?;
    off_main(move || remote(&app.state::<Store>(), &id, &repo)).await
}

#[tauri::command]
pub async fn gh_repo_create(app: AppHandle, window: WebviewWindow, repo: String, name: String, private: bool, description: String) -> Result<String> {
    let id = window_project_id(&window)?;
    off_main(move || repo_create(&Gh::locate()?, &app.state::<Store>(), &id, &repo, &name, private, &description)).await
}

#[tauri::command]
pub async fn gh_pr_list(app: AppHandle, window: WebviewWindow, repo: String, state: String) -> Result<Vec<Pr>> {
    let id = window_project_id(&window)?;
    off_main(move || pr_list(&Gh::locate()?, &app.state::<Store>(), &id, &repo, &state)).await
}

#[tauri::command]
pub async fn gh_pr_checks(app: AppHandle, window: WebviewWindow, repo: String, number: u64) -> Result<Vec<Check>> {
    let id = window_project_id(&window)?;
    off_main(move || pr_checks(&Gh::locate()?, &app.state::<Store>(), &id, &repo, number)).await
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn gh_pr_create(app: AppHandle, window: WebviewWindow, repo: String, title: String, body: String, base: String, draft: bool) -> Result<Pr> {
    let id = window_project_id(&window)?;
    off_main(move || pr_create(&Gh::locate()?, &app.state::<Store>(), &id, &repo, &title, &body, &base, draft)).await
}

/// `false` = annullato dall'utente.
#[tauri::command]
pub async fn gh_pr_merge(app: AppHandle, window: WebviewWindow, repo: String, number: u64, method: String, delete_branch: bool) -> Result<bool> {
    let id = window_project_id(&window)?;
    // una sola conferma per finestra (come `git_delete_branch`)
    let Some(guard) = app.state::<crate::claude::delete::DeletePrompts>().acquire(window.label()) else {
        return Err("Un'altra conferma è già aperta".into());
    };
    off_main(move || {
        let _guard = guard;
        let c = NativeConfirm { app: app.clone(), window };
        pr_merge(&Gh::locate()?, &app.state::<Store>(), &id, &repo, number, &method, delete_branch, &c)
    })
    .await
}

#[cfg(test)]
mod tests;
