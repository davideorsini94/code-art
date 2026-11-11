# Code Art — Tappa 2 "Strumenti da dev" Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Aggiungere a Code Art ricerca e sostituzione nel progetto, Git completo, GitHub tramite `gh` e completamento intelligente per Python, Java, Go e Rust tramite language server.

**Architecture:** Tutto il lavoro con processi e disco resta nel core Rust (moduli nuovi `tools`, `search`, `git`, `github`, `lsp`), esposto con comandi Tauri confinati alla finestra del progetto, come nella tappa 1. L'interfaccia aggiunge tre viste al pannello laterale (Esplora, Cerca, Git) scelte dalla barra icone, un pannello GitHub dentro la vista Git e un client LSP minimo in TypeScript collegato a Monaco.

**Tech Stack:** Tauri 2, Rust (crate `grep-searcher`, `grep-regex`, `grep-matcher`, `ignore` già presente), `git` e `gh` di sistema, language server installati sulla macchina (`pylsp`/`pyright-langserver`, `jdtls`, `gopls`, `rust-analyzer`), Svelte 5 + TypeScript, Monaco, Vitest.

**Spec:** `docs/superpowers/specs/2026-11-11-code-art-ide-design.md` (§9 tappa 2, §11 bis sicurezza). Linguaggi per il completamento scelti dall'utente all'inizio della tappa: **Python, Java, Go, Rust** (oltre a TS/JS, JSON, HTML, CSS già in Monaco).

## Global Constraints

- Ogni nuovo `#[tauri::command]` è registrato in `lib.rs`, elencato in `docs/sicurezza.md` (tabella comandi) e ricava il progetto **dalla finestra chiamante** (`window_project_id`), mai da un argomento della pagina. Il test `every_command_is_registered_and_listed` deve restare verde.
- Percorsi dalla pagina sempre passati per `readable_path` / `mutable_path` (confinamento, link simbolici rifiutati).
- **Git, `gh` e i language server partono solo in progetti fidati** (`ensure_project_trusted`): `git` esegue hook e `core.fsmonitor` del repository, i language server eseguono codice del progetto (build.rs, `go list`, Gradle). In un progetto non fidato le viste Git e completamento mostrano "Cartella non fidata" con il pulsante "Mi fido" già esistente. La ricerca (sola lettura, nessun processo) funziona anche nei progetti non fidati; la **sostituzione** no.
- Processi sempre con lista di argomenti (`std::process::Command`), mai `sh -c`; ambiente = `locate::login_env()` (già in cache) + `GIT_TERMINAL_PROMPT=0`, `GIT_OPTIONAL_LOCKS=0` per le letture, `LC_ALL=C` per output analizzati; nessun argomento della pagina può iniziare con `-` dove git lo interpreterebbe come opzione (usare `--` prima dei percorsi; nomi di branch validati con `git check-ref-format --branch`).
- Ogni processo ha un timeout (letture git 15 s; fetch/pull/push/clone 10 min, annullabili; `gh` 60 s) e passa da `off_main`/thread dedicato: niente I/O sul thread principale.
- Token: il token di un account `gh` diverso dall'attivo si ottiene con `gh auth token --user <login>` e passa **solo** come `GH_TOKEN` nell'ambiente del processo figlio; mai in log, eventi, errori, SQLite.
- Conferma nativa dal core (dialogo con «Annulla» predefinito, come `conv_delete`) prima di: scartare modifiche, sostituire in più file, eliminare un branch, unire una pull request, forzare un push (non offerto in questa tappa).
- Stile: testo dell'interfaccia in italiano, tooltip su ogni controllo cliccabile (azione `use:tooltip`), colori solo dai token del tema (`src/styles/glass.css`), nessun `{@html}` fuori da `renderMarkdown`.
- Moduli pesanti caricati al primo uso (vista Git, client LSP): `import()` dinamico come Monaco.
- Commit: messaggi in italiano nello stile del repo, firmati con l'identità già configurata nel repo.

## Review Focus

1. **Repository grandi** (es. 50 000 file, `node_modules` non ignorato, monorepo): `git status` e la ricerca non devono bloccare l'interfaccia né saturare la memoria → limiti (10 000 risultati di ricerca, file > 2 MB e binari saltati, stato git troncato a 5 000 voci con avviso) e streaming a blocchi. Test nel Task 2 e nel Task 4.
2. **Rete assente o credenziali mancanti** su fetch/pull/push/clone: il processo non resta appeso aspettando una password (`GIT_TERMINAL_PROMPT=0`, `GIT_SSH_COMMAND` con `-o BatchMode=yes`), l'errore di git arriva leggibile e il pulsante torna attivo. Test nel Task 5 con un remote inesistente.
3. **Nomi strani**: file con spazi, accenti (NFC/NFD), a capo nel nome, branch con `/`; un nome di branch che inizia con `-` non diventa un'opzione. `status --porcelain=v2 -z` e `--` ovunque. Test nel Task 4 e 5.
4. **Merge con conflitti** e file modificato nell'editor mentre git lo cambia: la vista mostra lo stato "unione in corso" con "Annulla unione", i file in conflitto aprono l'editor con i pulsanti di risoluzione, e il rilevamento "cambiato su disco" della tappa 1 non perde modifiche locali. Test nel Task 7.
5. **Language server assente, lento o che muore**: il completamento di Monaco continua a funzionare con le parole del file, appare una riga "Completamento Python non attivo: installa con `pip install python-lsp-server`" e il processo morto viene riavviato al massimo 3 volte in 5 minuti. Test nel Task 10.

---

### Task 1: Strumenti di sistema e viste del pannello laterale

**Files:**
- Create: `src-tauri/src/tools.rs`
- Modify: `src-tauri/src/lib.rs` (mod + registrazione), `src/components/Rail.svelte`, `src/App.svelte`, `src/lib/layout.ts`
- Create: `src/components/SidebarView.svelte`, `src/lib/stores/sidebar.svelte.ts`
- Test: `src-tauri/src/tools.rs` (mod tests), `src/lib/stores/sidebar.test.ts`, `src/components/Rail.test.ts`

**Interfaces:**
- Produces (Rust): `pub fn find_tool(env: &HashMap<String,String>, name: &str) -> Option<PathBuf>` (cerca in `PATH` dell'ambiente di login, poi `/opt/homebrew/bin`, `/usr/local/bin`, `~/.cargo/bin`, `~/go/bin`); `pub fn tool_command(name: &str) -> Result<Command, String>` (Command con ambiente di login, `in_new_session`, errore "`git` non trovato" se assente); `pub fn run_tool(cmd: Command, timeout: Duration, input: Option<Vec<u8>>) -> Result<ToolOutput, String>` con `pub struct ToolOutput { pub status: i32, pub stdout: Vec<u8>, pub stderr: String }` (stderr troncato a 8 KB). Comando `tools_check() -> ToolsStatus { git: Option<String>, gh: Option<String>, lsp: Vec<LspAvail> }` dove `LspAvail { lang: String, found: bool, install_hint: String }`.
- Produces (TS): store `sidebar` con `view: 'files' | 'search' | 'git'` persistito per progetto (chiave `localStorage` `codeart.sidebar.<projectId>`, letture/scritture in try/catch); `SidebarView.svelte` che rende la vista corrente (per ora `FileTree` e due segnaposto).

- [ ] **Step 1: Test Rust** `find_tool_prefers_login_path`, `find_tool_falls_back_to_homebrew` (cartelle temporanee con eseguibili finti), `run_tool_times_out` (`sleep 5` con timeout 200 ms → Err contenente "tempo scaduto"), `run_tool_truncates_stderr`. Eseguire `cargo test tools` → FAIL.
- [ ] **Step 2: Implementare `tools.rs`** riusando `locate::login_env`, `locate::in_new_session`, `locate::kill_group`. `cargo test tools` → PASS.
- [ ] **Step 3: Test TS** `sidebar.test.ts`: default `files`, cambio vista persistito, `localStorage` che lancia non rompe. `Rail.test.ts`: tre pulsanti vista (Esplora ⌘⇧E, Cerca ⌘⇧F, Git ⌘⇧G) con tooltip, il pulsante della vista attiva ha `aria-pressed="true"`; cliccare la vista attiva con il pannello aperto lo chiude, cliccarne un'altra lo apre su quella vista. → FAIL.
- [ ] **Step 4: Implementare** store, Rail (le icone vista sostituiscono il solo pulsante "Progetto"; restano Conversazioni e Conversazione), scorciatoie da tastiera a livello di finestra, `SidebarView`. Registrare `tools_check`, riga in `docs/sicurezza.md`. `pnpm typecheck && pnpm vitest run` e `cargo test` → PASS.
- [ ] **Step 5: Commit** `feat: viste del pannello laterale e ricerca degli strumenti di sistema`

### Task 2: Ricerca nel progetto (core)

**Files:**
- Create: `src-tauri/src/search.rs`
- Modify: `src-tauri/Cargo.toml` (`grep-searcher`, `grep-regex`, `grep-matcher`), `src-tauri/src/lib.rs`, `docs/sicurezza.md`
- Test: `src-tauri/src/search.rs` (mod tests)

**Interfaces:**
- Produces: `pub struct SearchQuery { pub text: String, pub regex: bool, pub case_sensitive: bool, pub whole_word: bool, pub include: Vec<String>, pub exclude: Vec<String> }` (glob stile gitignore); `pub struct SearchHit { pub path: String, pub line: u32, pub col: u32, pub len: u32, pub preview: String }` (preview ≤ 240 caratteri attorno alla corrispondenza, UTF-8 valido); `pub fn search(roots: &[PathBuf], q: &SearchQuery, cancel: &AtomicBool, sink: impl FnMut(Vec<SearchHit>)) -> SearchSummary` con `SearchSummary { files: u32, hits: u32, truncated: bool, error: Option<String> }`.
- Comandi: `search_start(query) -> u64` (id ricerca; una ricerca nuova annulla la precedente della stessa finestra; risultati a blocchi di ≤ 200 sull'evento di progetto `search:hits` `{ id, hits }`, fine su `search:done` `{ id, summary }`); `search_cancel(id)`.
- Limiti: 10 000 risultati poi `truncated: true`; file > 2 MB e binari saltati; rispetta `.gitignore` e il filtro rumore `fs_tree::is_noise`; nessun link simbolico seguito.

- [ ] **Step 1: Test** `finds_literal_with_line_and_col`, `regex_and_whole_word`, `case_insensitive_default`, `respects_gitignore_and_noise`, `include_exclude_globs`, `skips_binary_and_large_files`, `stops_at_limit_with_truncated`, `cancel_stops_early`, `invalid_regex_reports_error` (errore in `summary.error`, nessun panic), `non_utf8_file_does_not_panic`, `nfd_filename_reported_as_nfc`. → FAIL.
- [ ] **Step 2: Implementare** con `ignore::WalkBuilder` (parallelo) + `grep_searcher::Searcher` con `BinaryDetection::quit`. → PASS.
- [ ] **Step 3: Comandi** con thread dedicato per ricerca, `AtomicBool` di annullamento per finestra in uno stato `Searches`. Test di sicurezza in `tests/security.rs`: la ricerca non restituisce file fuori dalle cartelle del progetto anche con un link simbolico verso `/etc`. Righe in `docs/sicurezza.md`. `cargo test` → PASS.
- [ ] **Step 4: Commit** `feat: ricerca nel progetto nel core, a blocchi e annullabile`

### Task 3: Vista Cerca e sostituzione

**Files:**
- Create: `src/components/SearchPanel.svelte`, `src/lib/stores/search.svelte.ts`
- Modify: `src-tauri/src/search.rs` (sostituzione), `src/lib/ipc.ts`, `src/components/SidebarView.svelte`, `src/lib/stores/editor.svelte.ts` (`openAt(path, line, col)`), `src/components/EditorPane.svelte` (rivelare la riga), `docs/sicurezza.md`
- Test: `src/lib/stores/search.test.ts`, `src/components/SearchPanel.test.ts`, test Rust di `replace_in_files`

**Interfaces:**
- Consumes: `search_start`, `search_cancel`, eventi `search:hits`/`search:done` (Task 2).
- Produces: comando `search_replace(query, replacement, paths: Vec<String>) -> ReplaceOutcome { files: u32, replacements: u32, skipped: Vec<String> }`: solo progetto fidato; conferma nativa dal core ("Sostituire N occorrenze in M file?", «Annulla» predefinito); ricalcola le corrispondenze sul disco (non si fida dei risultati della pagina), scrive con `fs_tree::write` atomico; con `regex` i gruppi `$1` sono permessi; file aperti con modifiche non salvate nell'editor sono saltati (la pagina li passa esclusi e il core li salta comunque se l'mtime è cambiato dall'avvio).
- Store `search`: `query`, opzioni, risultati raggruppati per file (ordine di arrivo, conteggio), stato `running|done|error`, debounce 250 ms sull'input.

- [ ] **Step 1: Test Rust** `replace_rewrites_only_matches`, `replace_with_regex_groups`, `replace_refused_when_untrusted`, `replace_skips_file_changed_since` (mtime). Test TS: debounce, nuova query annulla la vecchia, raggruppamento, click su un risultato chiama `editor.openAt`, toggle Aa / ab / .* con tooltip, campo sostituzione nascosto se progetto non fidato, troncamento mostrato ("Mostrati i primi 10 000 risultati"). → FAIL.
- [ ] **Step 2: Implementare** core, store, pannello (campo, opzioni, includi/escludi ripiegabili, elenco per file ripiegabile con icona `FileIcon`, corrispondenza evidenziata con `<mark>` costruito da testo, mai HTML). `openAt` apre il file e porta il cursore su riga/colonna. ⌘⇧F apre la vista col testo selezionato nell'editor. → PASS.
- [ ] **Step 3: Commit** `feat: vista Cerca con sostituzione nel progetto`

### Task 4: Git in lettura (core)

**Files:**
- Create: `src-tauri/src/git/mod.rs`, `src-tauri/src/git/status.rs`, `src-tauri/src/git/cmd.rs`
- Modify: `src-tauri/src/lib.rs`, `docs/sicurezza.md`
- Test: mod tests con repository temporanei creati con `git init` (helper `fn repo() -> TempDir` che configura `user.name`/`user.email` locali fittizi)

**Interfaces:**
- Produces: `pub fn git(repo: &Path, args: &[&str], timeout: Duration) -> Result<ToolOutput, String>` in `cmd.rs` (aggiunge sempre `-c core.quotepath=off -c color.ui=never`, ambiente del Global Constraints); `pub struct RepoStatus { pub root: String, pub branch: Option<String>, pub upstream: Option<String>, pub ahead: u32, pub behind: u32, pub detached: bool, pub merging: bool, pub entries: Vec<StatusEntry>, pub truncated: bool }`, `pub struct StatusEntry { pub path: String, pub orig_path: Option<String>, pub staged: Option<char>, pub unstaged: Option<char>, pub conflict: bool, pub untracked: bool }`; `pub fn parse_porcelain_v2(z: &[u8]) -> (header, Vec<StatusEntry>)`.
- Comandi (progetto fidato): `git_repos() -> Vec<String>` (cartelle del progetto che sono radici di repository, in ordine; la principale prima); `git_status(repo) -> RepoStatus` (`repo` deve essere una di `git_repos()`); `git_diff(repo, path, staged: bool) -> DiffText { before: String, after: String, binary: bool }` (prima = `HEAD` o indice, dopo = indice o disco; file > 2 MB → `binary: true`); `git_branches(repo) -> Vec<Branch { name, remote: bool, current: bool, upstream: Option<String> }>`; `git_log(repo, limit ≤ 200) -> Vec<Commit { hash, short, subject, author, date_iso }>`.
- Evento di progetto `git:changed` `{ repo }` emesso (con debounce 300 ms) quando il watcher della tappa 1 vede cambiare file del progetto o `.git/HEAD`, `.git/index`, `.git/refs`; il filtro rumore oggi esclude `.git`: aggiungere un'eccezione mirata per quei tre percorsi.

- [ ] **Step 1: Test** `porcelain_v2_parses_all_kinds` (modificato, aggiunto, rinominato con orig_path, eliminato, non tracciato, conflitto `u`), `names_with_spaces_accents_newline`, `ahead_behind_from_header`, `detached_head`, `merging_detected` (`MERGE_HEAD`), `status_truncated_over_5000`, `diff_staged_vs_unstaged`, `diff_new_file_has_empty_before`, `branches_lists_local_and_remote`, `log_respects_limit`, `repo_arg_must_be_project_repo` (percorso fuori → errore), `refused_when_untrusted`. → FAIL.
- [ ] **Step 2: Implementare.** `git_status` usa `status --porcelain=v2 -z --branch --untracked-files=all`; il branch in barra del titolo (`projects::branch`) resta la lettura diretta di `.git/HEAD` (funziona anche non fidato). → PASS.
- [ ] **Step 3: Evento `git:changed`** con test sul filtro rumore (`.git/index` passa, `.git/objects/…` no). Righe in `docs/sicurezza.md`. → PASS.
- [ ] **Step 4: Commit** `feat: stato, diff, branch e cronologia git nel core`

### Task 5: Git in scrittura (core)

**Files:**
- Create: `src-tauri/src/git/write.rs`, `src-tauri/src/git/remote.rs`
- Modify: `src-tauri/src/git/mod.rs`, `src-tauri/src/lib.rs`, `docs/sicurezza.md`
- Test: mod tests (repository temporanei; per il remoto un repository `--bare` locale come `origin`)

**Interfaces:**
- Consumes: `git()`, `RepoStatus` (Task 4).
- Produces comandi (tutti: progetto fidato, `repo` ∈ `git_repos()`, percorsi confinati e passati dopo `--`):
  - `git_stage(repo, paths)`, `git_unstage(repo, paths)` (`restore --staged`), `git_discard(repo, paths)` → conferma nativa ("Scartare le modifiche a N file? Non si possono recuperare.", «Annulla» predefinito); file non tracciati scartati = spostati nel Cestino con `trasher`, non cancellati.
  - `git_commit(repo, message, amend: bool) -> String` (hash breve; messaggio passato via stdin con `commit -F -`; messaggio vuoto rifiutato; hook del repository eseguiti normalmente: il progetto è fidato).
  - `git_checkout(repo, branch)`, `git_create_branch(repo, name, from: Option<String>, checkout: bool)`, `git_delete_branch(repo, name)` (conferma nativa; mai il branch corrente; `-d`, non `-D`: se non unito, errore leggibile). Nomi validati con `git check-ref-format --branch`.
  - `git_merge(repo, branch) -> MergeOutcome { kind: 'fast_forward'|'merged'|'conflicts'|'up_to_date', conflicts: Vec<String> }`, `git_merge_abort(repo)`.
  - Operazioni remote lunghe: `git_remote_op(repo, op: 'fetch'|'pull'|'push', ...) -> u64` (id); avanzamento da `--progress` (stderr) sull'evento `git:progress` `{ id, line }`, fine su `git:op_done` `{ id, ok, message }`; `git_cancel(id)` uccide il gruppo di processi. `pull` = `pull --ff-only` di default (impostazione `git_pull_mode`: `ff-only` | `rebase` | `merge`, validata in `WEB_SETTINGS`); `push` imposta l'upstream se manca (`-u origin <branch>`). Mai `--force`.
  - Ambiente remoto: `GIT_TERMINAL_PROMPT=0`, `GIT_SSH_COMMAND="ssh -o BatchMode=yes"` (se l'utente non ha già un `GIT_SSH_COMMAND`), helper di credenziali dell'utente lasciati attivi (portachiavi, `gh auth git-credential`).
- Una sola operazione di scrittura alla volta per repository (mutex per repo); le altre ricevono "Un'altra operazione git è in corso".

- [ ] **Step 1: Test** `stage_unstage_roundtrip`, `commit_creates_commit_with_message_from_stdin`, `commit_empty_message_refused`, `amend_changes_last_commit`, `branch_name_starting_with_dash_refused`, `create_checkout_delete_branch`, `delete_current_branch_refused`, `merge_fast_forward`, `merge_conflicts_reported_and_abort`, `push_sets_upstream_on_bare_origin`, `fetch_unreachable_remote_fails_fast_without_prompt` (remote `https://127.0.0.1:9/x.git`, deve finire entro 20 s con errore), `cancel_kills_long_op`, `concurrent_write_refused`, `discard_moves_untracked_to_trash` (con `trasher` sostituito da un finto in test), `paths_outside_project_refused`. Le conferme native passano da un trait `Confirm` (finto nei test, dialogo vero nell'app) come in `conv_delete`. → FAIL.
- [ ] **Step 2: Implementare.** → PASS. Righe in `docs/sicurezza.md`; aggiungere a `tests/security.rs` un test che nessun comando git accetta un argomento che inizia con `-` come percorso o branch.
- [ ] **Step 3: Commit** `feat: stage, commit, branch, merge, fetch, pull e push nel core`

### Task 6: Vista Git

**Files:**
- Create: `src/components/git/GitPanel.svelte`, `src/components/git/ChangeList.svelte`, `src/components/git/CommitBox.svelte`, `src/components/git/BranchPicker.svelte`, `src/components/git/SyncBar.svelte`, `src/lib/stores/git.svelte.ts`
- Modify: `src/lib/ipc.ts`, `src/lib/types.ts`, `src/components/SidebarView.svelte`, `src/components/Titlebar.svelte` (il branch diventa un pulsante che apre `BranchPicker`), `src/lib/palette/*` (comandi: Commit, Fetch, Pull, Push, Cambia branch, Crea branch, Unisci branch…)
- Test: `src/lib/stores/git.test.ts`, `src/components/git/GitPanel.test.ts`, `BranchPicker.test.ts`, `CommitBox.test.ts`

**Interfaces:**
- Consumes: comandi ed eventi dei Task 4–5; `editor.openDiff(path, before, after)` (tappa 1).
- Produces: store `git` per finestra: `repos`, `status` per repo, `busy` (operazione in corso con ultima riga di avanzamento), `error` leggibile, `refresh()` chiamato su `git:changed` (debounce già nel core) e al focus della finestra.
- UI: per ogni repo una sezione (titolo = nome cartella, nascosto se c'è un solo repo) con: barra di sincronizzazione (branch, ↑ahead ↓behind, Fetch, Pull, Push con avanzamento e Annulla), campo di commit (⌘↵ conferma; "Modifica ultimo commit" come casella), "Modifiche in stage" e "Modifiche" con azioni per riga al passaggio del mouse (stage ＋, unstage −, scarta ↶, apri file) e per gruppo; click su una riga apre il diff nell'editor (diff di sola lettura per lo stage, `after` modificabile per le modifiche non in stage); banner "Unione in corso: N conflitti" con "Annulla unione"; stato "non è un repository" con pulsante "Inizializza" (`git init`, da aggiungere al Task 5 come `git_init(folder)` se manca) e "Pubblica su GitHub" (Task 8). Progetto non fidato: messaggio + "Mi fido".
- `BranchPicker`: elenco filtrabile di branch locali e remoti, "Crea branch da qui…" con validazione del nome in tempo reale, "Unisci in quello corrente…".

- [ ] **Step 1: Test** store (refresh su evento, busy/annulla, errore mostrato e azzerato al successivo successo, operazioni bloccate mentre `busy`), pannello (raggruppamento, azioni di riga chiamano i comandi giusti, click apre diff, commit disabilitato con messaggio vuoto o niente in stage, ⌘↵), BranchPicker (filtro, nome non valido mostra errore e disabilita "Crea"), titolo: il branch è un pulsante con tooltip "Cambia branch". → FAIL.
- [ ] **Step 2: Implementare** (vista caricata con `import()` al primo uso). → PASS.
- [ ] **Step 3: Commit** `feat: vista Git con modifiche, commit, branch e sincronizzazione`

### Task 7: Risoluzione dei conflitti nell'editor

**Files:**
- Create: `src/lib/conflicts.ts`
- Modify: `src/components/EditorPane.svelte`, `src/lib/stores/editor.svelte.ts`, `src/components/git/ChangeList.svelte`
- Test: `src/lib/conflicts.test.ts`, `src/components/EditorPane.test.ts` (estensione)

**Interfaces:**
- Produces: `parseConflicts(text: string): Conflict[]` con `Conflict { start: number; mid: number; end: number; base?: number; ours: string; theirs: string }` (righe 1-based; supporta lo stile `diff3` con `|||||||`); `resolve(text: string, c: Conflict, pick: 'ours' | 'theirs' | 'both'): string`.
- Editor: per i file con marcatori, decorazioni colorate (token del tema) e una riga di azioni sopra ogni conflitto ("Tieni la tua · Tieni l'altra · Tieni entrambe") resa con un view zone/content widget di Monaco costruito con nodi DOM (niente HTML da testo); contatore "N conflitti" nella barra della scheda con ↑/↓ per saltare. Quando non restano marcatori e il file è salvato, nella vista Git il file mostra "Segna come risolto" (= `git_stage`).

- [ ] **Step 1: Test** `parses_two_conflicts`, `parses_diff3_base`, `ignores_unbalanced_markers`, `resolve_each_pick`, `resolve_keeps_line_endings` (CRLF), editor: azione su un conflitto modifica il testo e segna la scheda come modificata; il ricaricamento da disco della tappa 1 con modifiche locali chiede invece di sovrascrivere. → FAIL.
- [ ] **Step 2: Implementare.** → PASS.
- [ ] **Step 3: Commit** `feat: conflitti di unione risolvibili nell'editor`

### Task 8: GitHub tramite `gh`

**Files:**
- Create: `src-tauri/src/github.rs`, `src/components/git/GitHubPanel.svelte`, `src/components/git/PrCreate.svelte`, `src/components/CloneDialog.svelte`, `src/lib/stores/github.svelte.ts`
- Modify: `src-tauri/src/lib.rs`, `src-tauri/src/projects.rs` (impostazione di progetto `github_account`), `src/lib/ipc.ts`, `src/components/git/GitPanel.svelte`, schermata di benvenuto (pulsante "Clona da GitHub…"), palette, `docs/sicurezza.md`
- Test: mod tests Rust con un `gh` finto (script in `src-tauri/tests/fake_gh.sh` che risponde a `auth status`, `repo list`, `pr list`, `pr view`, `pr checks`, `pr create`, `pr merge`, `repo create`, `repo clone` con JSON fissi e registra gli argomenti ricevuti), test TS dei componenti

**Interfaces:**
- Produces (Rust, progetto fidato salvo `gh_accounts` e `gh_repo_list`/`gh_clone` che partono dalla finestra di benvenuto o da qualunque finestra):
  - `gh_accounts() -> Vec<GhAccount { login, active }>` (da `gh auth status --json hosts` se disponibile, altrimenti analisi del testo); impostazione di progetto `github_account` (login o vuoto = account attivo di `gh`); per un account diverso dall'attivo, `GH_TOKEN` = `gh auth token --user <login>` solo nell'ambiente del figlio. Mai `gh auth switch`.
  - `gh_repo_create(repo, name, private: bool, description) -> String` (URL; `gh repo create <name> --source <repo> --remote origin --push` più `--private`/`--public`).
  - `gh_repo_list(account, limit ≤ 200) -> Vec<RemoteRepo { name_with_owner, description, private, updated_at }>`; `gh_clone(name_with_owner) -> ()`: la cartella di destinazione si sceglie con dialogo nativo dal core, poi il progetto si apre in una nuova finestra (già fidato: l'utente l'ha appena clonato lui).
  - `gh_pr_list(repo, state: 'open'|'closed'|'all') -> Vec<Pr { number, title, author, head, base, state, is_draft, url, checks: ChecksSummary { passing, failing, pending } }>`; `gh_pr_checks(repo, number) -> Vec<Check { name, state, url }>`; `gh_pr_create(repo, title, body, base, draft) -> Pr` (push del branch corrente prima, se manca l'upstream); `gh_pr_merge(repo, number, method: 'merge'|'squash'|'rebase', delete_branch: bool)` con conferma nativa.
  - Tutti i JSON di `gh` analizzati con `serde` su campi espliciti (`--json` con elenco di campi), errori di `gh` mostrati senza token né percorsi della home.
- UI: sezione "GitHub" nella vista Git: account in uso (scelta tra gli account di `gh`), elenco PR con stato dei controlli (pallino verde/rosso/ambra), apertura del link nel browser di sistema (`openExternal` esistente), "Nuova pull request" (titolo precompilato dall'ultimo commit, corpo in markdown con anteprima sanificata, base = branch predefinito), "Unisci" con metodo. Repo senza remote GitHub: "Pubblica su GitHub" (nome = nome cartella, privato di default).

- [ ] **Step 1: Test Rust** con `gh` finto: argomenti esatti per ogni comando, `GH_TOKEN` presente solo per account non attivo e mai in `Debug`/errori (test che formatta l'errore e cerca il token), `pr_merge` chiede conferma (trait `Confirm` del Task 5), JSON malformato → errore leggibile, `gh` assente → "GitHub CLI (`gh`) non trovata: `brew install gh`". → FAIL.
- [ ] **Step 2: Implementare** core. → PASS.
- [ ] **Step 3: Test TS + implementazione** pannello, creazione PR, clone dalla schermata di benvenuto. → PASS.
- [ ] **Step 4: Commit** `feat: GitHub con gh — repository, clone e pull request`

### Task 9: Ponte LSP nel core

**Files:**
- Create: `src-tauri/src/lsp/mod.rs`, `src-tauri/src/lsp/servers.rs`, `src-tauri/src/lsp/framing.rs`
- Modify: `src-tauri/src/lib.rs`, `docs/sicurezza.md`
- Test: mod tests (framing; server finto `src-tauri/tests/fake_lsp.sh` che risponde a `initialize` con JSON fisso e ripete le notifiche ricevute)

**Interfaces:**
- Produces: tabella `servers.rs`: per `python` → `pyright-langserver --stdio`, altrimenti `pylsp`; `java` → `jdtls -data <AppSupport>/CodeArt/jdtls/<projectId>`; `go` → `gopls`; `rust` → `rust-analyzer`; ognuno con `install_hint` (`npm i -g pyright` o `pip install python-lsp-server`; `brew install jdtls`; `go install golang.org/x/tools/gopls@latest`; `rustup component add rust-analyzer`) usato anche da `tools_check` (Task 1).
- `framing.rs`: `encode(json: &[u8]) -> Vec<u8>` (`Content-Length`), `Decoder::push(&mut self, bytes) -> Vec<Vec<u8>>` (messaggi completi; header oltre 8 KB o lunghezza > 64 MB → errore e chiusura).
- Comandi (progetto fidato): `lsp_start(lang) -> LspStart { id: u64, root_uri: String }` (un server per progetto+linguaggio, riusato; radice = cartella principale; avvio pigro al primo file di quel linguaggio); `lsp_send(id, message: String)` (JSON validato come oggetto JSON-RPC; i metodi `workspace/executeCommand` e notifiche arbitrarie passano, ma ogni URI `file://` nei parametri deve stare dentro le cartelle del progetto); evento di progetto `lsp:message` `{ id, message }`; `lsp_stop(id)`; evento `lsp:exit` `{ id, code }`. Riavvio automatico al massimo 3 volte in 5 minuti, poi stato "fermo" fino a riapertura. Tutti i server fermati alla chiusura della finestra del progetto (`lifecycle`).

- [ ] **Step 1: Test** framing (messaggi spezzati, due in un pacchetto, header malformato), `server_for_lang_prefers_pyright`, `missing_server_returns_install_hint`, `file_uri_outside_project_refused`, `restart_budget_three_in_five_minutes`, `refused_when_untrusted`, `stop_kills_process_group`, roundtrip con server finto. → FAIL.
- [ ] **Step 2: Implementare** (lettura stdout su thread dedicato, stderr scartato salvo ultime 4 KB per l'errore di avvio). → PASS. Righe in `docs/sicurezza.md`.
- [ ] **Step 3: Commit** `feat: ponte verso i language server nel core`

### Task 10: Client LSP in Monaco

**Files:**
- Create: `src/lib/lsp/client.ts`, `src/lib/lsp/monaco-bridge.ts`, `src/lib/lsp/languages.ts`
- Modify: `src/components/EditorPane.svelte`, `src/lib/monaco.ts`, `src/lib/ipc.ts`, `src/components/Titlebar.svelte` o barra di stato dell'editor (indicatore "Python ●")
- Test: `src/lib/lsp/client.test.ts`, `src/lib/lsp/monaco-bridge.test.ts`

**Interfaces:**
- Consumes: `lsp_start`, `lsp_send`, `lsp_stop`, eventi `lsp:message`/`lsp:exit` (Task 9).
- Produces: `class LspClient { request(method, params, timeoutMs = 5000): Promise<unknown>; notify(method, params): void; onNotification(method, cb) }` (id incrementali, richieste annullate con `$/cancelRequest` quando Monaco annulla); `languages.ts`: estensioni → linguaggio (`.py` python, `.java` java, `.go` go, `.rs` rust); `monaco-bridge.ts`: `initialize`/`initialized` con capacità minime, `textDocument/didOpen|didChange` (sincronizzazione completa, debounce 150 ms)`|didSave|didClose`, e provider Monaco per `completion` (con `resolve`), `hover` (markdown via `renderMarkdown`, mai HTML grezzo), `signatureHelp`, `definition` (apre il file nell'editor se dentro il progetto), `publishDiagnostics` → marker Monaco.
- Comportamento: progetto non fidato o server mancante → nessun errore bloccante, riga informativa una volta per sessione con il suggerimento d'installazione; indicatore di stato per linguaggio (avvio, pronto, fermo) con tooltip.

- [ ] **Step 1: Test** client (correlazione risposte, timeout, annullamento, notifiche), bridge con un trasporto finto (didOpen alla prima apertura, didChange con debounce, conversione completion item → suggerimento Monaco con range e kind, diagnostica → marker con severità, hover resa sanificata, definition fuori progetto ignorata, server mancante → messaggio unico). → FAIL.
- [ ] **Step 2: Implementare** (modulo caricato con `import()` alla prima apertura di un file di quei linguaggi). → PASS.
- [ ] **Step 3: Commit** `feat: completamento, hover, definizioni e diagnostica per Python, Java, Go e Rust`

### Task 11: Chiusura della tappa — sicurezza, misure, documenti

**Files:**
- Modify: `docs/sicurezza.md`, `src-tauri/tests/security.rs`, `docs/verifica-tappa-2.md` (nuovo), `README.md` (roadmap: tappa 2 fatta)
- Test: `src-tauri/tests/security.rs`

- [ ] **Step 1: Test di sicurezza trasversali**: ogni nuovo comando che avvia processi rifiuta un progetto non fidato (elenco esplicito: git_*, gh_* tranne quelli della finestra di benvenuto, lsp_*, search_replace); nessun file sorgente nuovo usa `Command::new("sh")`/`"bash"`/`"zsh"` o `-c`; la stringa `GH_TOKEN` compare solo in `github.rs`. → PASS.
- [ ] **Step 2: `cargo audit` e `pnpm audit`** (installare `cargo-audit` solo se manca, in `~/.cargo/bin`); annotare i risultati in `docs/sicurezza.md` (rischi residui) e correggere le vulnerabilità con aggiornamenti minori dei lockfile.
- [ ] **Step 3: Misure** con `pnpm tauri build`: dimensione app e dmg, avvio a freddo, RAM con un progetto aperto; ricerca su un repo grande (es. questo repo con `node_modules`) sotto 2 s per una parola comune; scritte in `docs/verifica-tappa-2.md` con la lista "Da verificare a vista".
- [ ] **Step 4: Commit** `docs: verifica e sicurezza della tappa 2`
