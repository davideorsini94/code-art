# Code Art — Tappa 1 "Il cuore" Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Un'app Mac installabile in cui ogni progetto ha la sua finestra (titolo = nome progetto) e le conversazioni Claude Code, con titoli vivi e stati, stanno al centro accanto ad albero file, editor e terminale.

**Architecture:** Tauri 2. Il core Rust possiede tutto ciò che ha stato o I/O: progetti e finestre, file system, PTY, processi `claude` (uno per conversazione, streaming JSON su stdin/stdout), lettura delle sessioni esistenti, SQLite. L'interfaccia Svelte riceve eventi e invoca comandi; non parla mai direttamente con disco o processi. Titoli e stati vivono solo nel nostro SQLite, mai nei file di Claude.

**Tech Stack:** Tauri 2, Rust (tokio, rusqlite bundled, notify, portable-pty, serde_json, trash, ignore), Svelte 5 + TypeScript + Vite, Monaco, xterm.js, Vitest, pnpm.

**Spec:** `docs/superpowers/specs/2026-11-11-code-art-ide-design.md` (§6, §7, §8, §10, §11, §12). Riferimento visivo: `.superpowers/brainstorm/41040-1794386226/content/main-window.html`.

## Global Constraints

- Solo macOS; identificatore app `it.codeart.app`, nome prodotto `Code Art`.
- Il titolo nativo di ogni finestra è **sempre e solo il nome del progetto**.
- I file in `~/.claude/` sono **in sola lettura**: nessun task li scrive.
- Dati dell'app in `~/Library/Application Support/CodeArt/codeart.db`.
- CLI Claude: l'eseguibile `claude` risolto dal PATH della shell di login dell'utente; nessuna chiave API.
- Modalità permessi di default per le nuove conversazioni: `bypassPermissions`, configurabile.
- Titoli: massimo 6 parole, nella lingua della conversazione; ri-titolazione solo a fine turno e solo se sono passati almeno 3 turni dall'ultimo titolo; un titolo manuale blocca l'automatismo.
- Titolazione delle sessioni importate: al massimo 20 per progetto, dalle più recenti.
- Avviso "forse aperta altrove" se il file di sessione è stato scritto negli ultimi 60 secondi da altri.
- Colori di stato: `--wait:#ffb454` (aspetta te), `--work:#5ee0c1` (al lavoro), `--done:#7d8aa6` (inattiva), accento `#8fa8ff`.
- Area codice e terminale quasi opache (alpha ≥ 0.85); animazioni 150–250 ms; rispettare `prefers-reduced-motion`.
- Prestazioni: finestra visibile entro 1 s a freddo; < 250 MB RAM con un progetto e nessun turno in corso; app < 40 MB; Monaco e xterm caricati con `import()` al primo uso.
- Scorciatoie come VS Code: ⌘P apri file, ⌘⇧P comandi, ⌘K ricerca unica, ⌘S salva, ⌘↵ invia.
- Testi dell'interfaccia in italiano.
- Commit dopo ogni task, messaggi in stile `feat: …` / `test: …`.

## Review Focus

1. **Righe di sessione sconosciute o malformate** (il formato cambia con le versioni della CLI): la riga viene saltata, la sessione si apre comunque. → test in Task 5.
2. **File di sessione molto grandi (100 MB+)**: la lettura dei metadati scorre il file riga per riga senza caricarlo intero e senza bloccare l'interfaccia. → test in Task 5.
3. **Evento della CLI di tipo non previsto** durante un turno: mostrato come blocco generico espandibile, la conversazione non si rompe e lo stato non resta bloccato su "al lavoro". → test in Task 6 e Task 10.
4. **Cartella di progetto sparita** (spostata, disco scollegato): la finestra si apre con il nome giusto e un messaggio chiaro con "Scegli nuova posizione", niente crash. → test in Task 2.
5. **File binario o enorme aperto nell'editor** (> 5 MB o non UTF-8): messaggio "file non modificabile qui" invece di blocco o testo corrotto. → test in Task 3.

## File Structure

```
src-tauri/
  Cargo.toml, tauri.conf.json, capabilities/default.json
  src/main.rs            avvio, registrazione comandi, menu
  src/store.rs           SQLite: schema e query
  src/projects.rs        progetti, finestre, ripristino sessione
  src/fs_tree.rs         elenco, CRUD, lettura/scrittura file, watcher
  src/pty.rs             terminali
  src/claude/mod.rs
  src/claude/locate.rs   trova l'eseguibile claude, verifica versione
  src/claude/sessions.rs lettura sessioni esistenti + watcher
  src/claude/protocol.rs righe stream-json -> ClaudeEvent
  src/claude/state.rs    macchina degli stati della conversazione
  src/claude/engine.rs   processi claude, comandi conv_*
  src/claude/titles.rs   regole e generazione titoli/riassunti
  src/lifecycle.rs       chiusura finestre, uscita, badge Dock, notifiche
src/
  app.css, styles/glass.css          token e materiali vetro
  lib/ipc.ts                         wrapper tipizzati di invoke/listen
  lib/types.ts                       tipi condivisi (specchio dei tipi Rust)
  lib/stores/{project,conversations,editor}.svelte.ts
  lib/conv/order.ts                  ordinamento schede
  lib/conv/reduce.ts                 eventi -> elementi della chat
  lib/conv/slash.ts                  completamento comandi slash
  lib/palette/search.ts              ricerca unica
  components/{Titlebar,Rail,FileTree,ConvList,ConvCard,ConvPane,ChatItem,
              AskBox,TodoStrip,Composer,EditorPane,TerminalPane,Palette,
              MissingFolder,ClaudeMissing}.svelte
docs/claude-protocol.md               esito dello spike (Task 4)
```

Tipi condivisi (definiti in Rust con `serde(rename_all = "camelCase")`, specchiati in `src/lib/types.ts`):

```ts
type Project = { id: string; name: string; folders: string[] }      // folders[0] = principale
type DirEntry = { name: string; path: string; isDir: boolean; ignored: boolean }
type FileContent = { text: string; mtimeMs: number }
type ConvStatus = 'working' | 'waiting' | 'idle'
type Conversation = {
  sessionId: string; projectId: string; cwd: string
  title: string; prevTitle: string | null; titleChangedAt: number | null
  titleLocked: boolean; summary: string
  status: ConvStatus; owned: boolean            // owned = pilotata da Code Art
  archived: boolean; lastActivity: number; turnCount: number
  model: string | null; permissionMode: string
  contextUsedPct: number | null; maybeOpenElsewhere: boolean
}
type ChatItem =
  | { kind: 'user'; id: string; text: string; images: string[] }
  | { kind: 'assistant'; id: string; text: string }
  | { kind: 'thinking'; id: string; text: string }
  | { kind: 'tool'; id: string; name: string; input: unknown; result: string | null;
      ok: boolean | null; parentId: string | null; filePath: string | null }
  | { kind: 'ask'; id: string; requestId: string; askType: 'permission' | 'question' | 'plan';
      prompt: string; options: string[]; answered: boolean }
  | { kind: 'raw'; id: string; label: string; json: string }
type Todo = { text: string; status: 'pending' | 'in_progress' | 'completed' }
```

---

### Task 1: Scheletro dell'app e materiale vetro

**Files:**
- Create: progetto Tauri 2 + Svelte 5 + TS nella radice (`pnpm create tauri-app` in cartella temporanea, poi spostare), `src/styles/glass.css`, `src/app.css`, `src/App.svelte`, `src/components/{Titlebar,Rail}.svelte`, `.gitignore` (includere `.superpowers/`, `node_modules`, `target`, `dist`)
- Test: `src/styles/glass.test.ts`

**Interfaces:**
- Produces: classi CSS `.glass`, `.glass-strong`, `.surface-opaque`; variabili `--wait --work --done --accent --txt --dim --faint --edge --edge-soft`; layout a griglia di `App.svelte` con slot denominati `rail | sidebar | convlist | conv | editor`, colonne ridimensionabili e richiudibili con larghezze salvate in `localStorage` chiave `codeart.layout`; funzione pura `canCollapse(col, collapsed: Set<string>): boolean` in `src/lib/layout.ts` che impedisce di richiudere `convlist` se `conv` è già chiusa e viceversa (testata in `src/lib/layout.test.ts` nei due sensi).

- [ ] **Step 1:** `git init`; generare lo scheletro; `tauri.conf.json`: `productName: "Code Art"`, `identifier: "it.codeart.app"`, finestra con `transparent: true`, `titleBarStyle: "Overlay"`, `hiddenTitle: false`, `macOSPrivateApi: true`; applicare vibrancy `NSVisualEffectMaterial::HudWindow` in `main.rs` (crate `window-vibrancy`).
- [ ] **Step 2: Test che fallisce** — `glass.test.ts` legge `glass.css` come testo e verifica: contiene le quattro variabili di colore con i valori esatti dei Global Constraints; `.surface-opaque` ha alpha ≥ 0.85; esiste un blocco `@media (prefers-reduced-motion: reduce)` che azzera `animation` e `transition`.
- [ ] **Step 3:** Run `pnpm vitest run src/styles/glass.test.ts` → FAIL (file assente).
- [ ] **Step 4:** Scrivere `glass.css` e `app.css` portando dal mockup: sfondo aurora (quattro gradienti radiali attenuati su `#0a0b12`), `.glass` (fondo `rgba(255,255,255,.055)`, bordo `--edge-soft`, luce interna in alto, `backdrop-filter: blur(22px) saturate(160%)`), raggio 14 px. `App.svelte` con la griglia e `Titlebar`/`Rail` statici come nel mockup (zona trascinabile `data-tauri-drag-region`, spazio per i semafori).
- [ ] **Step 5:** Run test → PASS. Run `pnpm tauri dev` → finestra traslucida con la griglia vuota in vetro.
- [ ] **Step 6:** Commit `feat: scheletro Tauri e materiale vetro`.

### Task 2: Store, progetti e finestre

**Files:**
- Create: `src-tauri/src/store.rs`, `src-tauri/src/projects.rs`, `src/lib/ipc.ts`, `src/lib/types.ts`, `src/lib/stores/project.svelte.ts`, `src/components/MissingFolder.svelte`
- Modify: `src-tauri/src/main.rs`, `src/components/Titlebar.svelte`

**Interfaces:**
- Produces (Rust): `Store::open(path: &Path) -> Result<Store>`; `Store::upsert_project(&Project)`, `get_project(id)`, `list_projects()`, `set_open_flag(id, bool)`, `open_projects() -> Vec<Project>`. Tabelle `projects(id TEXT PK, name, folders_json, last_opened, was_open)` e `conversations` (colonne = campi persistenti di `Conversation` più `title_turn INTEGER`, `seen_turn INTEGER`).
- Produces (comandi Tauri): `project_open_folder(path) -> Project` (riusa il progetto se `path` è già cartella principale di uno esistente; nome = ultimo segmento del percorso), `project_current() -> Project` (dal label della finestra `proj-<id>`), `project_rename(id, name)`, `project_add_folder(id, path)`, `project_relocate(id, old, new)`, `project_missing_folders(id) -> string[]`, `project_pick_and_open()` (dialogo cartella), `project_branch(id) -> string | null` (branch della cartella principale letto da `.git/HEAD`, mostrato nella barra del titolo e aggiornato su `fs:changed`), `settings_get(key) -> string | null`, `settings_set(key, value)` (tabella `settings(key TEXT PK, value)`; chiave usata in questa tappa: `default_permission_mode`, valore iniziale `bypassPermissions`).
- Regole finestra: label `proj-<id>`; titolo nativo = `project.name`, aggiornato a ogni rinomina; aprire un progetto già aperto porta la sua finestra in primo piano; all'avvio si riaprono i progetti con `was_open = 1`, e se non ce ne sono si mostra una finestra "Apri cartella"; menu File › Apri cartella (⌘O), File › Recenti.

- [ ] **Step 1: Test che falliscono** (`store.rs`, `projects.rs`, `#[cfg(test)]`, DB in `tempfile`):
  - `open_folder_names_project_after_last_segment`: `/tmp/x/demo-api` → `name == "demo-api"`, `folders == [quel percorso]`.
  - `open_same_folder_twice_returns_same_project`: stesso `id`.
  - `rename_persists`: dopo `project_rename`, `get_project` restituisce il nuovo nome.
  - `open_projects_returns_only_flagged`.
  - `missing_folder_is_reported_not_fatal` (Review Focus 4): progetto con cartella inesistente → `project_missing_folders` la elenca e `get_project` riesce.
- [ ] **Step 2:** Run `cargo test --manifest-path src-tauri/Cargo.toml projects store` → FAIL.
- [ ] **Step 3:** Implementare store, comandi e gestione finestre; la logica pura dei comandi sta in funzioni che ricevono `&Store` (testabili), i `#[tauri::command]` sono involucri sottili.
- [ ] **Step 4:** `Titlebar.svelte` mostra `project.name` (doppio click per rinominare); `MissingFolder.svelte` copre la finestra quando `project_missing_folders` non è vuoto, con pulsante "Scegli nuova posizione".
- [ ] **Step 5:** Run test → PASS. Manuale: aprire due cartelle → due finestre; in Mission Control i titoli sono i nomi dei progetti; riavviare → si riaprono.
- [ ] **Step 6:** Commit `feat: progetti, finestre per progetto, ripristino`.

### Task 3: Albero file ed editor

**Files:**
- Create: `src-tauri/src/fs_tree.rs`, `src/components/{FileTree,EditorPane}.svelte`, `src/lib/stores/editor.svelte.ts`
- Test: in `fs_tree.rs`; `src/lib/stores/editor.test.ts`

**Interfaces:**
- Produces (comandi): `fs_list(path) -> DirEntry[]` (cartelle prima, poi alfabetico senza distinzione maiuscole; `ignored` da regole git via crate `ignore`; `.git` nascosta), `fs_create(path, isDir)`, `fs_rename(from, to)`, `fs_trash(path)` (Cestino, crate `trash`), `fs_read(path) -> FileContent`, `fs_write(path, text, expectedMtimeMs: number | null) -> number` (nuovo mtime). Errori serializzati come `{ code: 'conflict' | 'too_large' | 'not_text' | 'io', message }`; `too_large` oltre 5 MB, `not_text` se non UTF-8 valido.
- Produces (evento): `fs:changed` con `{ paths: string[] }`, emesso alla finestra del progetto, raggruppato a 150 ms (crate `notify`), su tutte le cartelle del progetto.
- Produces (TS): `editor` store con `open(path)`, `openDiff(path, before, after)`, `save(path)`, `tabs`, `active`, e funzione pura `onDiskChange(tab, disk: FileContent): 'reload' | 'prompt' | 'none'` (`reload` se la tab non ha modifiche locali e il testo differisce, `prompt` se ne ha, `none` se il testo è uguale).

- [ ] **Step 1: Test che falliscono (Rust):** `list_sorts_dirs_first_case_insensitive`; `list_marks_gitignored` (repo temporaneo con `.gitignore` → `ignored == true`); `write_with_stale_mtime_is_conflict`; `read_over_5mb_is_too_large` e `read_invalid_utf8_is_not_text` (Review Focus 5); `trash_removes_path`.
- [ ] **Step 2: Test che falliscono (TS):** `onDiskChange` nei tre casi sopra.
- [ ] **Step 3:** Run `cargo test … fs_tree` e `pnpm vitest run src/lib/stores/editor.test.ts` → FAIL.
- [ ] **Step 4:** Implementare `fs_tree.rs` e lo store. `FileTree.svelte`: una radice per ogni `project.folders`, espansione pigra, menu contestuale (nuovo file/cartella, rinomina, sposta nel Cestino), trascinamento per spostare, icone per tipo di file, voci `ignored` attenuate, ricarica dei rami toccati su `fs:changed`. `EditorPane.svelte`: Monaco via `import()` al primo file aperto, tema scuro personalizzato su `.surface-opaque`, tab, vista divisa, minimap, ⌘S, diff con `openDiff`; per `too_large`/`not_text` mostra "File non modificabile qui"; su `fs:changed` applica `onDiskChange` (in caso `prompt`: barra "Cambiato su disco — Ricarica / Tieni il mio").
- [ ] **Step 5:** Run test → PASS. Manuale: modificare un file da fuori → la tab si ricarica; con modifiche locali → compare la barra.
- [ ] **Step 6:** Commit `feat: albero file ed editor Monaco`.

### Task 4: Spike del protocollo Claude (documento, non codice di prodotto)

**Files:**
- Create: `docs/claude-protocol.md`, `src-tauri/tests/fixtures/stream/*.jsonl`

**Interfaces:**
- Produces: il documento che i Task 5–7 e 10–11 usano come fonte; file di esempio reali catturati dalla CLI, usati come fixture.

Fatti già verificati su `claude` 2.1.319: esistono `-p`, `--output-format stream-json`, `--input-format stream-json`, `--include-partial-messages`, `--replay-user-messages`, `--verbose`, `--resume <id>`, `--session-id <uuid>`, `--fork-session`, `--add-dir`, `--model`, `--permission-mode <mode>`, `--permission-prompts <host|none>`, `--dangerously-skip-permissions`. Le sessioni sono in `~/.claude/projects/<cartella-codificata>/<sessionId>.jsonl`; le righe hanno un campo `type` (tra cui `user`, `assistant`, `system`, `custom-title`) e le righe `user`/`assistant` portano `cwd`, `sessionId`, `timestamp`, `isSidechain`, `message`.

- [ ] **Step 1:** In una cartella di prova, avviare `claude -p --input-format stream-json --output-format stream-json --include-partial-messages --verbose --permission-prompts host --permission-mode default` e, scrivendo messaggi su stdin, catturare l'output di questi scenari in fixture separate: `simple-turn`, `tool-use-edit`, `permission-request` (con risposta consenti e nega), `ask-user-question`, `plan-mode` (con approvazione), `subagent`, `todo-update`, `interrupt`, `slash-command` (`/compact`), `image-input`, `model-switch`, `unknown` (una riga inventata `{"type":"future-thing"}` aggiunta a mano).
- [ ] **Step 2:** Scrivere `docs/claude-protocol.md` rispondendo, con l'esempio JSON per ciascuna voce: riga di avvio (dove stanno `session_id`, modello, elenco comandi slash, elenco strumenti); forma dei messaggi utente in ingresso (testo, immagini base64); forma di testo parziale, thinking, uso strumento e risultato; come si riconosce un subagent (campo che lega allo strumento padre); forma di richiesta di permesso / domanda / approvazione piano e della risposta da scrivere su stdin; come interrompere un turno; come cambiare modello e modalità permessi a processo vivo (messaggio di controllo oppure riavvio con `--resume`); riga di fine turno e dove legge l'uso del contesto; regola esatta di codifica del nome cartella (verificare con percorsi che contengono spazio, punto, underscore, lettera accentata).
- [ ] **Step 3:** Criterio di scelta, da scrivere in testa al documento: se richieste di permesso, domande e interruzione sono tutte realizzabili via stdin/stdout → **motore CLI diretto**. Se anche una sola non lo è → **sidecar Node** con `@anthropic-ai/claude-agent-sdk` che espone lo stesso flusso di righe JSON su stdout e accetta gli stessi comandi su stdin; in tal caso i Task 6–7 restano identici salvo il comando lanciato.
- [ ] **Step 4:** Commit `docs: esito spike protocollo Claude e fixture`.

### Task 5: Sessioni esistenti

**Files:**
- Create: `src-tauri/src/claude/{mod,sessions}.rs`
- Test: in `sessions.rs`, fixture in `src-tauri/tests/fixtures/sessions/`

**Interfaces:**
- Consumes: `Store` (Task 2); regola di codifica da `docs/claude-protocol.md`.
- Produces: `encode_project_dir(path: &str) -> String`; `session_dirs_for(folders: &[String]) -> Vec<PathBuf>`; `read_session_meta(file: &Path) -> Result<SessionMeta>` con `SessionMeta { session_id, cwd, first_prompt, last_activity_ms, turn_count, custom_title: Option<String>, mtime_ms }`; `read_session_items(file: &Path) -> Result<Vec<ChatItem>>`; `sync_project_sessions(store, project) -> Vec<Conversation>`; comando `conv_list(projectId) -> Conversation[]`, `conv_history(sessionId) -> ChatItem[]`; evento `conv:upsert` (payload `Conversation`); watcher su `session_dirs_for` che richiama la sincronizzazione sul file toccato.
- Regole: una sessione appartiene al progetto solo se il `cwd` letto dal file è una delle sue cartelle (non fidarsi del solo nome della directory, la codifica perde informazione); righe con `isSidechain: true` non contano come turni; `turn_count` = numero di messaggi utente scritti dalla persona (esclusi risultati di strumenti e righe `isMeta`); titolo provvisorio = `custom_title` se presente, altrimenti `first_prompt` troncato a 60 caratteri; le sessioni non pilotate hanno `owned = false`, `status = idle`; `maybe_open_elsewhere = !owned && now - mtime_ms < 60_000`.

- [ ] **Step 1: Test che falliscono:**
  - `encode_matches_cli_rule`: i casi verificati nello spike, almeno `/Users/a/workspace/code-art` → `-Users-a-workspace-code-art` e `/Users/a/v2.3.6` → `-Users-a-v2-3-6`.
  - `meta_counts_only_human_turns` su fixture con 2 prompt umani, 3 risultati strumento, 1 riga sidechain → `turn_count == 2`.
  - `meta_skips_malformed_and_unknown_lines` (Review Focus 1): fixture con una riga troncata e una `{"type":"future-thing"}` → `Ok`, stessi conteggi.
  - `meta_streams_large_file` (Review Focus 2): file generato di 120 MB → completa e il picco di memoria allocata dal lettore resta sotto 8 MB (leggere con `BufReader::lines`, verificare con un allocatore contatore nel test oppure limitando la capacità del buffer e asserendo che nessuna `String` superi la riga più lunga).
  - `session_with_foreign_cwd_is_excluded`.
  - `recent_foreign_write_sets_maybe_open_elsewhere`.
- [ ] **Step 2:** Run `cargo test … sessions` → FAIL.
- [ ] **Step 3:** Implementare. `read_session_items` produce `ChatItem` secondo le forme documentate nello spike; tipo di riga non riconosciuto → nessun elemento.
- [ ] **Step 4:** Run test → PASS.
- [ ] **Step 5:** Commit `feat: lettura e sincronizzazione delle sessioni Claude esistenti`.

### Task 6: Protocollo e macchina degli stati

**Files:**
- Create: `src-tauri/src/claude/{protocol,state}.rs`
- Test: negli stessi file, con le fixture del Task 4

**Interfaces:**
- Produces: `parse_line(line: &str) -> ClaudeEvent`, mai `Err`:
  ```rust
  enum ClaudeEvent {
    Init { session_id: String, model: String, slash_commands: Vec<String> },
    TextDelta { id: String, text: String },
    Thinking { id: String, text: String },
    ToolStart { id: String, name: String, input: Value, parent_id: Option<String> },
    ToolEnd { id: String, ok: bool, result: String },
    Ask { request_id: String, ask_type: AskType, prompt: String, options: Vec<String> },
    Todos(Vec<Todo>),
    TurnEnd { context_used_pct: Option<f32> },
    Unknown { label: String, json: String },
  }
  ```
- Produces: `fn next_status(cur: ConvState, input: StateInput) -> ConvState` con `ConvState { status: ConvStatus, open_asks: u32, turn_count: u32, seen_turn: u32 }` e `StateInput { UserSent, Event(&ClaudeEvent), AskAnswered, Viewed, ProcessExited }`.
- `ClaudeEvent` è serializzato con `#[serde(tag = "kind", rename_all = "camelCase")]` e specchiato come unione discriminata `ClaudeEvent` in `src/lib/types.ts` (campo `kind`: `init | textDelta | thinking | toolStart | toolEnd | ask | todos | turnEnd | unknown`); è questo il tipo che Task 10 riceve in `conv:event`.
- Regole di stato: `UserSent` → `working`; `Ask` → `waiting` e `open_asks += 1`; `AskAnswered` → `open_asks -= 1`, e se arriva a 0 → `working`; `TurnEnd` → `turn_count += 1`, poi `waiting` se `seen_turn < turn_count`, altrimenti `idle`; `Viewed` → `seen_turn = turn_count`, e se lo stato era `waiting` con `open_asks == 0` → `idle`; `ProcessExited` mentre `working` → `waiting`; `Unknown` non cambia nulla.

- [ ] **Step 1: Test che falliscono:** un test per fixture (`simple-turn` → sequenza `Init, TextDelta…, TurnEnd`; `permission-request` → contiene `Ask` di tipo permesso; `subagent` → `ToolStart` con `parent_id` valorizzato; `todo-update` → `Todos`); `garbage_line_is_unknown_not_panic` con `"{not json"` e con `{"type":"future-thing"}` (Review Focus 3); per lo stato una tabella di transizioni che copre ogni regola sopra, più `unknown_event_never_leaves_working_stuck`: `UserSent, Unknown, TurnEnd` finisce in `waiting`.
- [ ] **Step 2:** Run `cargo test … protocol state` → FAIL.
- [ ] **Step 3:** Implementare secondo `docs/claude-protocol.md`.
- [ ] **Step 4:** Run test → PASS.
- [ ] **Step 5:** Commit `feat: parser eventi Claude e macchina degli stati`.

### Task 7: Motore delle conversazioni

**Files:**
- Create: `src-tauri/src/claude/{locate,engine}.rs`, `src/components/ClaudeMissing.svelte`, `src-tauri/tests/fake_claude.sh`
- Modify: `src-tauri/src/main.rs`

**Interfaces:**
- Consumes: `parse_line`, `next_status` (Task 6); `Store`, `Project` (Task 2); `conv:upsert` (Task 5).
- Produces (comandi): `claude_check() -> { found: boolean; version: string | null; path: string | null }`; `conv_start(projectId, opts: { model?: string; permissionMode?: string }) -> string` (sessionId generato da noi e passato con `--session-id`); `conv_resume(sessionId)`; `conv_send(sessionId, text, imagePaths: string[])`; `conv_answer(sessionId, requestId, choice: string)`; `conv_interrupt(sessionId)`; `conv_set_model(sessionId, model)`; `conv_set_permission_mode(sessionId, mode)`; `conv_mark_viewed(sessionId)`; `conv_archive(sessionId, archived)`; `conv_running(projectId) -> string[]`.
- Produces (eventi): `conv:event` `{ sessionId, event: ClaudeEvent }`; `conv:upsert` a ogni cambio di stato o metadati.
- Regole: i processi vivono in uno stato dell'app (`Mutex<HashMap<String, ConvHandle>>`), non della finestra. Avvio: `cwd` = cartella di origine della conversazione (per le nuove: `folders[0]`), un `--add-dir` per ogni altra cartella del progetto, `--permission-mode` = quello della conversazione (default da impostazione `default_permission_mode`, valore iniziale `bypassPermissions`). L'ambiente del processo è quello della shell di login (`$SHELL -lic env`, letto una volta all'avvio dell'app) così PATH, nvm e MCP funzionano come nel terminale. `conv_send` su una sessione non avviata la riprende prima. Uscita del processo con codice ≠ 0 → `ProcessExited` e ultimo stderr salvato su `Conversation` come elemento `raw` con etichetta "Processo terminato".

- [ ] **Step 1: Test che falliscono** (integrazione, con `fake_claude.sh` che ignora gli argomenti, stampa una fixture riga per riga e legge stdin; il percorso dell'eseguibile è iniettabile in `Engine::new(claude_path, store, emitter)` dove `emitter` è un trait finto nei test):
  - `start_passes_cwd_add_dirs_and_permission_mode`: il finto scrive i propri argomenti e `pwd` su un file; verificare `--add-dir <seconda cartella>`, `--permission-mode bypassPermissions`, `pwd == folders[0]`.
  - `send_emits_events_and_ends_waiting_until_viewed`: dopo la fixture `simple-turn` lo stato salvato è `waiting`; dopo `conv_mark_viewed` è `idle`.
  - `permission_fixture_sets_waiting_then_answer_writes_to_stdin`: il finto registra stdin; contiene la risposta nel formato dello spike.
  - `crash_mid_turn_sets_waiting_and_keeps_resumable`: il finto esce con codice 1 a metà.
  - `two_conversations_run_in_parallel`.
- [ ] **Step 2:** Run `cargo test … engine` → FAIL.
- [ ] **Step 3:** Implementare `locate.rs` (risoluzione nel PATH di login, `claude --version`) ed `engine.rs`. `ClaudeMissing.svelte` copre la colonna conversazioni quando `claude_check().found` è falso, con il comando di installazione/login da copiare.
- [ ] **Step 4:** Run test → PASS.
- [ ] **Step 5:** Commit `feat: motore conversazioni Claude`.

### Task 8: Titoli e riassunti vivi

**Files:**
- Create: `src-tauri/src/claude/titles.rs`
- Modify: `src-tauri/src/claude/engine.rs` (aggancio a `TurnEnd`), `src-tauri/src/claude/sessions.rs` (coda di titolazione degli importati)

**Interfaces:**
- Consumes: `Store`, `Conversation`, `read_session_items`, `conv:upsert`.
- Produces: `should_retitle(turn_count: u32, title_turn: Option<u32>, locked: bool) -> bool`; `build_title_prompt(current_title: Option<&str>, recent: &[ChatItem]) -> String`; `parse_title_reply(reply: &str) -> Option<(String, String)>` (titolo, riassunto); `apply_title(conv: &mut Conversation, title: String, summary: String, now_ms: i64)`; comandi `conv_rename(sessionId, title)` (imposta `titleLocked = true`) e `conv_unlock_title(sessionId)`.
- Regole: `should_retitle` è vero se `!locked` e (`title_turn` è `None` e `turn_count >= 1`, oppure `turn_count - title_turn >= 3`). Generazione: `claude -p --model haiku --output-format json` avviato in una cartella temporanea vuota (così non crea sessioni nel progetto), con ingresso gli ultimi 12 elementi `user`/`assistant` troncati a 600 caratteri ciascuno; il prompt chiede JSON `{"title": "...", "summary": "..."}`, titolo di massimo 6 parole nella lingua della conversazione, riassunto di massimo 160 caratteri che dice a che punto è il lavoro, e **di restituire il titolo attuale identico se l'argomento principale non è cambiato**. `apply_title`: se il titolo è uguale all'attuale aggiorna solo riassunto e `title_turn`; se diverso sposta l'attuale in `prev_title` e imposta `title_changed_at`. Fallimento o risposta non valida: nessuna modifica, nessun errore mostrato. Una sola generazione alla volta per conversazione; al massimo 2 in parallelo nell'app. Importati: alla sincronizzazione mettere in coda le 20 sessioni più recenti senza `title_turn`; le altre solo quando la scheda viene aperta. Sessioni proseguite altrove (`owned = false`): quando la sincronizzazione vede crescere `turn_count` e `should_retitle` è vero, la sessione entra nella stessa coda (test `foreign_session_progress_requeues_title`).

- [ ] **Step 1: Test che falliscono:** `should_retitle` — tabella: `(0,None,false)→false`, `(1,None,false)→true`, `(3,Some(1),false)→false`, `(4,Some(1),false)→true`, `(9,Some(1),true)→false`; `parse_title_reply_rejects_more_than_six_words`; `parse_title_reply_tolerates_code_fence`; `apply_same_title_keeps_prev_title`; `apply_new_title_moves_old_to_prev`; `rename_locks_and_blocks_auto`; `import_queue_takes_20_most_recent` (25 sessioni → 20 in coda, ordinate per `last_activity` decrescente).
- [ ] **Step 2:** Run `cargo test … titles` → FAIL.
- [ ] **Step 3:** Implementare; l'esecutore della CLI è iniettato (trait `TitleRunner`) per i test.
- [ ] **Step 4:** Run test → PASS.
- [ ] **Step 5:** Commit `feat: titoli e riassunti vivi delle conversazioni`.

### Task 9: Colonna conversazioni

**Files:**
- Create: `src/lib/stores/conversations.svelte.ts`, `src/lib/conv/order.ts`, `src/components/{ConvList,ConvCard}.svelte`
- Modify: `src/components/Titlebar.svelte`
- Test: `src/lib/conv/order.test.ts`, `src/components/ConvCard.test.ts`

**Interfaces:**
- Consumes: `conv_list`, `conv:upsert`, `conv_start`, `conv_archive`, `conv_rename`, `conv_mark_viewed`.
- Produces: `sortConversations(list: Conversation[]): Conversation[]`; `waitingCount(list): number`; store `conversations` con `list`, `activeId`, `select(id)` (chiama `conv_mark_viewed`), `create()`.
- Regole: ordine = `waiting`, poi `working`, poi `idle`, dentro ogni gruppo per `lastActivity` decrescente; le archiviate sono escluse (visibili con il filtro "Archiviate"). Scheda: punto di stato colorato (pulsante per `working`), titolo, riassunto su due righe, etichetta di stato ("aspetta te" / "al lavoro" / nessuna), tempo relativo; se `maybeOpenElsewhere` icona con suggerimento "Forse aperta in un altro programma"; doppio click sul titolo per rinominare; lucchetto se `titleLocked`. Titlebar: "N ti aspetta/aspettano" quando `waitingCount > 0`, e "K conversazioni attive" = `working + waiting`.

- [ ] **Step 1: Test che falliscono:** `sortConversations` con sei elementi misti → ordine atteso esplicito; archiviate escluse; `waitingCount`; `ConvCard` rende le tre etichette e la classe `pulse` solo per `working`.
- [ ] **Step 2:** Run `pnpm vitest run src/lib/conv src/components/ConvCard.test.ts` → FAIL.
- [ ] **Step 3:** Implementare; lista virtualizzata oltre 50 schede.
- [ ] **Step 4:** Run test → PASS. Manuale: aprire un progetto reale → compaiono le sessioni esistenti, i titoli provvisori diventano titoli generati.
- [ ] **Step 5:** Commit `feat: colonna conversazioni con stati e ordinamento`.

### Task 10: Conversazione aperta

**Files:**
- Create: `src/lib/conv/reduce.ts`, `src/components/{ConvPane,ChatItem,AskBox,TodoStrip}.svelte`
- Test: `src/lib/conv/reduce.test.ts`

**Interfaces:**
- Consumes: `conv_history`, `conv:event`, `conv_answer`, `editor.openDiff` (Task 3), tipi `ChatItem`, `Todo`.
- Produces: `reduce(state: ChatState, ev: ClaudeEvent): ChatState` con `ChatState { items: ChatItem[]; todos: Todo[]; contextUsedPct: number | null }`; `initialState(history: ChatItem[]): ChatState`.
- Regole di `reduce`: `TextDelta` con lo stesso `id` si accoda all'elemento esistente; `ToolEnd` completa il `tool` con quell'`id`; `ToolStart` con `parentId` resta nella lista piatta e la vista lo annida sotto il padre; `Ask` aggiunge un elemento `ask`; `Todos` sostituisce `todos`; `Unknown` aggiunge un elemento `raw`; un evento che riferisce un `id` inesistente non lancia eccezioni. Vista: strumenti come righe compatte espandibili; strumenti di modifica file (nome `Edit`/`Write`/`MultiEdit`, o come risulta dallo spike) hanno "Apri diff" che chiama `editor.openDiff`; subagent come blocco ripiegabile con conteggio dei passi e risultato; thinking ripiegato; `AskBox` ambra con un pulsante per opzione, e per `askType: 'plan'` il piano in markdown con "Approva" / "Continua a pianificare"; `TodoStrip` fissa sopra il campo di risposta; lista virtualizzata; sotto il titolo "titolo aggiornato X fa · prima: «prevTitle»" quando `prevTitle` esiste.

- [ ] **Step 1: Test che falliscono:** `text_deltas_merge_by_id`; `tool_end_completes_matching_tool`; `ask_then_answer_marks_answered`; `todos_replace`; `unknown_becomes_raw_item` e `tool_end_for_missing_id_is_ignored` (Review Focus 3); `replaying_fixture_simple_turn_yields_one_user_one_assistant` usando la fixture del Task 4 convertita in eventi.
- [ ] **Step 2:** Run `pnpm vitest run src/lib/conv/reduce.test.ts` → FAIL.
- [ ] **Step 3:** Implementare `reduce.ts` e i componenti.
- [ ] **Step 4:** Run test → PASS. Manuale: una conversazione reale con uso di strumenti, un subagent e una modifica a file → diff apribile.
- [ ] **Step 5:** Commit `feat: vista della conversazione`.

### Task 11: Campo di risposta e controlli

**Files:**
- Create: `src/components/Composer.svelte`, `src/lib/conv/slash.ts`
- Modify: `src/components/ConvPane.svelte`
- Test: `src/lib/conv/slash.test.ts`

**Interfaces:**
- Consumes: `conv_send`, `conv_interrupt`, `conv_set_model`, `conv_set_permission_mode`, `Init.slash_commands` (salvati nello store conversazioni come `slashCommands`), `Conversation.contextUsedPct`.
- Produces: `slashMatches(input: string, cursor: number, commands: string[]): string[]` — attivo solo se il testo prima del cursore è `/` + lettere all'inizio del messaggio; corrispondenza per prefisso prima, poi per sottostringa; massimo 8 risultati.
- Regole: ⌘↵ invia, ↵ va a capo; mentre lo stato è `working` il pulsante diventa "Interrompi" (anche Esc); trascinare un file dall'albero inserisce `@percorso/relativo`; incollare o trascinare un'immagine la salva in una cartella temporanea dell'app e la mostra come miniatura rimovibile, passata in `imagePaths`; selettore modello (alias `opus`, `sonnet`, `haiku` più quelli riportati da `Init`); selettore modalità permessi (`bypassPermissions`, `acceptEdits`, `plan`, `default`) con la modalità attiva sempre visibile come etichetta — ambra se `bypassPermissions`; `/compact` e `/clear` sono inviati come normali messaggi slash; anello di contesto con percentuale e suggerimento del consumo; la bozza non inviata è salvata per conversazione in `localStorage`.

- [ ] **Step 1: Test che falliscono:** `slashMatches("/co", 3, ["compact","clear","code-review","commit"])` → `["compact","code-review","commit"]`; non attivo per `"ciao /co"`; massimo 8; prefisso prima di sottostringa.
- [ ] **Step 2:** Run `pnpm vitest run src/lib/conv/slash.test.ts` → FAIL.
- [ ] **Step 3:** Implementare.
- [ ] **Step 4:** Run test → PASS. Manuale: usare una skill con `/`, incollare uno screenshot, cambiare modello a metà, passare a `plan` e approvare un piano, usare uno strumento MCP Playwright.
- [ ] **Step 5:** Commit `feat: campo di risposta, slash, modelli, permessi`.

### Task 12: Terminale

**Files:**
- Create: `src-tauri/src/pty.rs`, `src/components/TerminalPane.svelte`

**Interfaces:**
- Produces (comandi): `pty_spawn(cwd, cols, rows) -> number`, `pty_write(id, data)`, `pty_resize(id, cols, rows)`, `pty_kill(id)`; eventi `pty:data` `{ id, data }` e `pty:exit` `{ id }`.
- Regole: shell = `$SHELL` come shell di login; `cwd` iniziale = `folders[0]`; più schede; xterm.js via `import()` al primo uso, con addon fit e web-links, tema sui colori del vetro su `.surface-opaque`; ⌃` mostra/nasconde il pannello; i PTY della finestra vengono chiusi alla chiusura della finestra.

- [ ] **Step 1: Test che fallisce (Rust):** `spawn_echo_roundtrip`: avviare `/bin/sh`, scrivere `echo ok-$((40+2))\n`, entro 2 s l'output raccolto contiene `ok-42`; `kill_emits_exit`.
- [ ] **Step 2:** Run `cargo test … pty` → FAIL.
- [ ] **Step 3:** Implementare.
- [ ] **Step 4:** Run test → PASS. Manuale: `vim`, ridimensionamento, due schede.
- [ ] **Step 5:** Commit `feat: terminale integrato`.

### Task 13: Ricerca unica, apri file, comandi

**Files:**
- Create: `src/components/Palette.svelte`, `src/lib/palette/search.ts`
- Modify: `src-tauri/src/fs_tree.rs` (comando `fs_all_files`), `src-tauri/src/claude/sessions.rs` (comando `conv_search`)
- Test: `src/lib/palette/search.test.ts`, test Rust per i due comandi

**Interfaces:**
- Produces (comandi): `fs_all_files(projectId) -> string[]` (percorsi relativi, rispettando gitignore, limite 50 000); `conv_search(projectId, query) -> { sessionId: string; snippet: string }[]` (ricerca senza distinzione maiuscole nel testo dei messaggi `user`/`assistant`, massimo 30 risultati, comprese le archiviate).
- Produces (TS): `fuzzyRank(query: string, items: string[], limit: number): string[]` — sottosequenza, punteggio più alto per corrispondenze contigue e sul nome del file rispetto al percorso; `commands: { id: string; title: string; run(): void }[]` registro dei comandi.
- Regole: ⌘P solo file; ⌘⇧P solo comandi; ⌘K tutto, in sezioni Conversazioni (titoli prima, poi testo via `conv_search` con attesa di 200 ms), File, Comandi. Comandi minimi: Nuova conversazione, Apri cartella, Aggiungi cartella al progetto, Rinomina progetto, Mostra/nascondi terminale, Archivia conversazione, Cambia modalità permessi predefinita.

- [ ] **Step 1: Test che falliscono:** `fuzzyRank("invsvc", ["src/invoice.service.ts","src/pdf.ts"])` → primo risultato il file invoice; il nome file batte il percorso (`"pdf"` → `src/pdf.ts` prima di `pdfs/readme.md`); `conv_search_finds_text_in_archived`; `fs_all_files_skips_ignored`.
- [ ] **Step 2:** Run i test → FAIL.
- [ ] **Step 3:** Implementare.
- [ ] **Step 4:** Run test → PASS.
- [ ] **Step 5:** Commit `feat: ricerca unica, apertura rapida, palette comandi`.

### Task 14: Ciclo di vita, notifiche, Dock

**Files:**
- Create: `src-tauri/src/lifecycle.rs`
- Modify: `src-tauri/src/main.rs`, `src-tauri/src/claude/engine.rs`

**Interfaces:**
- Consumes: `conv_running`, `conv_interrupt`, stato dell'`Engine`, plugin Tauri `notification` e `dialog`.
- Produces: `decide_close(running: usize) -> CloseAction { CloseNow, Ask }`; `badge_text(waiting_total: usize) -> Option<String>`; `should_notify(prev: ConvStatus, next: ConvStatus, window_focused: bool) -> bool`.
- Regole: chiusura finestra con conversazioni `working` → dialogo "Lascia finire in sottofondo" / "Interrompi" / "Annulla"; in sottofondo la finestra si chiude, i processi continuano e l'app resta nel Dock anche senza finestre. ⌘Q con conversazioni `working` in qualunque progetto → conferma, poi interruzione di tutte e uscita. Badge del Dock = totale `waiting` su tutti i progetti (nessun badge a 0). Notifica macOS quando una conversazione passa a `waiting` e la sua finestra non è in primo piano o è chiusa; testo = titolo della conversazione, sottotitolo = nome del progetto; il click apre o porta in primo piano la finestra del progetto e seleziona la conversazione. Al riavvio dopo uscita o crash nessun turno riparte da solo: le conversazioni che erano `working` risultano `waiting` con elemento "Turno interrotto — Riprendi".

- [ ] **Step 1: Test che falliscono:** `decide_close(0) == CloseNow`, `decide_close(2) == Ask`; `badge_text(0) == None`, `badge_text(3) == Some("3")`; `should_notify` vero solo per transizione verso `Waiting` da altro stato con `window_focused == false`; `startup_downgrades_working_to_waiting` (store con una conversazione `working` → dopo `Engine::recover` è `waiting`).
- [ ] **Step 2:** Run `cargo test … lifecycle` → FAIL.
- [ ] **Step 3:** Implementare.
- [ ] **Step 4:** Run test → PASS. Manuale: chiudere la finestra a turno in corso scegliendo "sottofondo" → arriva la notifica, il click riapre sulla conversazione.
- [ ] **Step 5:** Commit `feat: ciclo di vita, notifiche e badge del Dock`.

### Task 15: Pacchetto installabile e verifica finale

**Files:**
- Modify: `src-tauri/tauri.conf.json` (bundle `dmg`, icona), `package.json` (script `build:dmg`)
- Create: `src-tauri/icons/*` (icona in vetro coerente col tema), `docs/verifica-tappa-1.md`

- [ ] **Step 1:** `pnpm tauri build` → produce `src-tauri/target/release/bundle/dmg/Code Art_*.dmg` con firma ad-hoc (`signingIdentity: "-"`).
- [ ] **Step 2:** Run `pnpm vitest run` e `cargo test --manifest-path src-tauri/Cargo.toml` → tutto PASS.
- [ ] **Step 3:** Installare dal `.dmg` e misurare, annotando i numeri in `docs/verifica-tappa-1.md`: tempo dall'avvio a freddo alla finestra visibile (obiettivo ≤ 1 s); RAM dell'app più i processi WebKit con un progetto aperto e nessun turno in corso (obiettivo < 250 MB); dimensione di `Code Art.app` (obiettivo < 40 MB); scorrimento di un file da 5 MB e di una conversazione con oltre 500 elementi senza scatti.
- [ ] **Step 4:** Percorso manuale su un progetto reale, annotando l'esito: due progetti → due finestre con i nomi giusti in Mission Control; sessioni esistenti presenti con titoli generati; tre conversazioni in parallelo con stati corretti; titolo che cambia dopo un cambio di argomento e resta stabile altrimenti; skill, subagent, Playwright MCP, immagine incollata, cambio modello, modalità piano; modifica di Claude visibile come diff; conversazione proseguita da VS Code che si aggiorna nell'app.
- [ ] **Step 5:** Per ogni obiettivo mancato, correggere prima di chiudere (candidati: caricamento pigro non rispettato, liste non virtualizzate).
- [ ] **Step 6:** Commit `chore: pacchetto dmg e verifica della tappa 1`.
