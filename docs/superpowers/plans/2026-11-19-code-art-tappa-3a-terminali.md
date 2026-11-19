# Tappa 3a — gestione dei terminali locali — piano

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** aprire un terminale nella cartella giusta di un progetto con più repository, con schede che dicono dove sono e si
possono rinominare, chiudere e riordinare.

**Architecture:** il core cambia poco (`pty_spawn` accetta già ogni cartella dentro il progetto): si aggiunge solo un
selettore nativo di cartella validato. Uno store `terminals` diventa il punto d'ingresso unico ("apri un terminale in X")
usato da albero, vista Git e pannello; il pannello (`TerminalPane`) resta il proprietario delle istanze xterm e dei PTY.

**Tech Stack:** Tauri 2 (Rust), Svelte 5 + TS, xterm.js, Vitest.

**Spec:** `docs/superpowers/specs/2026-11-11-code-art-ide-design.md` §9 tappa 3, primo punto ("Gestione dei terminali
locali"); sicurezza §11 bis.

## Global Constraints

- Testi in italiano; ogni pulsante con `use:tooltip` e `aria-label` se è solo un'icona; icone da `src/components/Icon.svelte`
  (aggiungerne una, se serve, nello stesso stile: tracciati 24×24, niente `{@html}`).
- Ogni comando del core prende il progetto dalla finestra chiamante (`window_project_id`) e accetta solo percorsi dentro le
  sue cartelle (`ensure_inside`, dopo risoluzione dei link); terminale solo nei progetti fidati (`ensure_project_trusted`).
- Lavoro bloccante del core fuori dal thread principale (`off_main` / `spawn_blocking`).
- Repository pubblico: nessun percorso personale nei test (usare `/Users/dev/...` o cartelle temporanee).
- `localStorage` sempre dentro try/catch.
- Commit con `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

## Review Focus

- Cartella sparita (cancellata o rinominata) tra la scelta e l'apertura → messaggio leggibile nella scheda, nessuna
  scheda zombie.
- Progetto non fidato → "Apri terminale qui" spento con il motivo nel tooltip; il core rifiuta comunque.
- Selettore nativo su una cartella fuori dal progetto (o un link che porta fuori) → rifiutata dal core con messaggio, nessun
  terminale.
- Il branch cambia mentre il terminale è aperto → il titolo automatico si aggiorna; un titolo rinominato a mano non cambia.
- Trascinare una scheda su sé stessa o fuori dalla barra → ordine invariato.

---

### Task 1: selettore di cartella per il terminale (core)

**Files:** `src-tauri/src/pty.rs` (comando + test), `src-tauri/src/lib.rs` (registrazione), eventuali permessi in
`src-tauri/capabilities/` solo se il pattern esistente lo richiede; `src/lib/ipc.ts`.

**Interfaces:**
- Produce: `#[tauri::command] pub async fn pty_pick_cwd(app, window) -> Result<Option<String>, String>` — apre il selettore
  nativo di cartelle (stesso meccanismo di `pick_folder_for` in `projects.rs`, titolo "Apri un terminale in…", partendo
  dalla cartella principale del progetto). `None` = annullato. Una cartella fuori dalle cartelle del progetto (anche via
  link simbolico) → `Err("La cartella deve stare dentro il progetto")`. Restituisce il percorso validato.
- La validazione sta in una funzione pura testabile: `fn validate_cwd(roots: &[PathBuf], picked: &Path) -> Result<PathBuf, String>`
  (riusa `ensure_inside`; non deve essere una cartella → errore "cartella inesistente").
- Frontend: `export const ptyPickCwd = () => invoke<string | null>('pty_pick_cwd')` in `src/lib/ipc.ts`.
- Verifica anche che `project_branch(file)` accetti una cartella (non solo un file) restituendo il repository più
  profondo che la contiene; se non lo fa, correggilo e aggiungi il test.

- [ ] Test Rust: `validate_cwd` accetta una sottocartella, rifiuta una cartella esterna, rifiuta un link simbolico interno
  che punta fuori, rifiuta un file. Test `branch` con una cartella annidata in un repo.
- [ ] Implementa, `cargo test` verde, `cargo clippy` senza nuovi avvisi.
- [ ] Commit `feat: selettore di cartella per il terminale`.

### Task 2: store dei terminali e logica delle schede

**Files:** crea `src/lib/terminal/tabs.ts` + `tabs.test.ts`, `src/lib/stores/terminals.svelte.ts` + test; modifica
`src/App.svelte` (la visibilità del pannello passa allo store: `terminals.visible`, ⌃\` e `hooks.toggleTerminal`
continuano a funzionare).

**Interfaces (Produce):**
- `export function folderLabel(cwd: string, folders: string[]): string` — nome dell'ultima cartella di `cwd`; se `cwd` è una
  cartella del progetto, il suo nome; `/` finale ignorato.
- `export function tabTitle(t: { cwd: string; branch: string | null; custom: string | null }, folders: string[]): string` —
  `custom` se non vuoto, altrimenti `folderLabel` seguito da `" · <branch>"` se c'è un branch.
- `export function moveTab<T>(list: T[], from: number, to: number): T[]` — nuovo array; indici fuori range o `from === to`
  → copia invariata.
- `class TerminalStore` / `export const terminals`: `visible = $state(false)`, `toggle()`, `show()`,
  `open(cwd?: string)` — rende visibile il pannello e accoda una richiesta `{ cwd: string | undefined, seq: number }`
  (`requests = $state<...[]>([])`), `take()` restituisce e svuota la coda (la consuma `TerminalPane`). Senza `cwd` = cartella
  principale.

- [ ] Test: `folderLabel` (sottocartella, cartella del progetto, slash finale), `tabTitle` (con/senza branch, custom
  vuoto = automatico), `moveTab` (avanti, indietro, fuori range, stesso indice), store (`open` rende visibile e accoda in
  ordine, `take` svuota).
- [ ] Implementa e collega `App.svelte`; `pnpm typecheck` e `pnpm test` verdi.
- [ ] Commit `feat: store dei terminali`.

### Task 3: schede del terminale (titolo, rinomina, riordino, scelta della cartella)

**Files:** `src/components/TerminalPane.svelte` (+ test nuovo `TerminalPane.test.ts` se fattibile con xterm finto, altrimenti
test sulla logica estratta), eventuale `src/components/TerminalNewMenu.svelte`.

**Comportamento:**
- La scheda conserva `cwd`, `repo` (da `projectBranch(cwd)` all'apertura), `branch`, `custom`. Titolo = `tabTitle(...)`;
  tooltip = percorso relativo al progetto (`relativeToFolders`) + shell. Il branch segue il repository: si rilegge da
  `git.status[repo]?.branch` quando disponibile, altrimenti con `projectBranch` all'evento `git:changed`.
- Doppio clic sul titolo → campo di testo; Invio conferma, Esc annulla, testo vuoto torna al titolo automatico.
- Clic centrale chiude; × chiude (come oggi).
- Trascinamento delle schede per riordinarle (HTML5 drag and drop dentro la barra, indicatore di inserimento).
- Il pulsante ＋ apre un menu: le cartelle del progetto, i repository trovati (`git.repos`, senza doppioni), "Cartella del
  file aperto" (se c'è un file attivo nell'editor), "Altra cartella…" (`ptyPickCwd`; annullato = niente; errore = messaggio
  nel pannello). ⌘ o clic semplice sul ＋ con un solo elemento possibile apre direttamente nella principale.
- Consuma `terminals.take()` in un effetto: ogni richiesta apre una scheda nella sua cartella.
- Cartella inesistente o rifiutata: la scheda mostra il messaggio del core (come oggi per "non fidata").

- [ ] Test (logica o componente): titolo da cwd/branch, rinomina con Invio/Esc/vuoto, riordino, richiesta dallo store apre
  una scheda con quel cwd (con `ptySpawn` finto).
- [ ] Implementa; typecheck e test verdi.
- [ ] Commit `feat: schede del terminale con cartella, branch, rinomina e riordino`.

### Task 4: "Apri terminale qui" da albero e vista Git

**Files:** `src/components/FileTree.svelte` (+ test), `src/components/git/GitPanel.svelte` (+ test).

- Menu contestuale dell'albero: voce "Apri terminale qui" per cartelle (quella cartella) e file (la cartella che lo
  contiene), anche sulle cartelle radice. Progetto non fidato → voce spenta, tooltip "Cartella non fidata: terminale
  disattivato".
- Vista Git: pulsante con icona `terminal` nell'intestazione di ogni repository e, nella vista a repository unico
  (`plain`), accanto alla barra di sincronizzazione. Tooltip "Apri un terminale in <nome>".
- Entrambi chiamano `terminals.open(cwd)`.

- [ ] Test: la voce apre `terminals.open` con la cartella giusta (cartella e file); spenta se non fidato; il pulsante Git
  chiama `terminals.open(repo)`.
- [ ] Implementa; typecheck e test verdi.
- [ ] Commit `feat: apri terminale qui da albero e vista Git`.

### Task 5: documentazione e verifica

**Files:** `docs/verifica-tappa-3a.md` (nuovo: cosa provare a vista), `README.md` (stato delle tappe), `docs/sicurezza.md`
(il nuovo comando `pty_pick_cwd` e la sua validazione).

- [ ] `pnpm typecheck`, `pnpm test`, `cargo test`, `cargo clippy`, `pnpm build` verdi.
- [ ] Commit `docs: tappa 3a`.
