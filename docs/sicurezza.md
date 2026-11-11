# Sicurezza di Code Art

Verifica della tappa 1 (Task 16). Requisito del proprietario: "un IDE sicuro".

## Modello di minaccia (in dieci righe)

1. La WebView mostra contenuti non fidati: output di Claude, file, nomi di file, risultati di strumenti.
2. Va quindi trattata come potenzialmente compromessa (XSS da markdown, file o nomi costruiti ad arte).
3. Una WebView compromessa può chiamare qualunque comando Tauri con argomenti arbitrari.
4. Il core Rust è il confine: valida ogni argomento e non si fida di percorsi o id arrivati dalla pagina.
5. Il core avvia `claude` in bypass permissions dentro le cartelle del progetto: lì si esegue codice.
6. Una cartella appena clonata può essere ostile (istruzioni per il modello, script, hook).
7. Quindi Claude e terminale partono solo in cartelle fidate, scelte o confermate dall'utente con UI nativa.
8. L'ambiente della shell di login (con i segreti dell'utente) passa solo ai processi figli, mai alla pagina.
9. Nessuna rete dalla WebView: niente script, stili, immagini o connessioni remote.
10. Il repository è pubblico: nessun segreto, percorso personale o dato reale nei file tracciati.

## Misure attive

- **CSP restrittiva** (`src-tauri/tauri.conf.json`), verificata da `csp_has_no_unsafe_eval_or_remote_origin`.
- **Markdown sanificato**: `renderMarkdown` (`src/lib/safe-markdown.ts`, marked + DOMPurify con whitelist)
  è l'unica sorgente di `{@html}`; l'HTML grezzo è scartato, le immagini non si caricano, sopravvivono solo
  link `http(s)` con `rel="noopener noreferrer"`, intercettati (`src/lib/conv/links.ts`) e aperti nel browser
  di sistema. `src/lib/html-sites.test.ts` fallisce se compare un `{@html}` non sanificato o un
  `innerHTML`/`outerHTML`/`insertAdjacentHTML`/`document.write`.
- **Confinamento dei file**: i comandi `fs_*` e `pty_spawn` accettano solo percorsi assoluti dentro le
  cartelle del progetto della finestra chiamante (`fs_tree::ensure_inside`: canonicalizzazione, `..`,
  link simbolici e link pendenti che escono → errore `forbidden`). Test: `fs_write_outside_project_is_rejected`.
- **Cartelle nel progetto solo da UI nativa**: una cartella entra in un progetto solo dal selettore nativo
  aperto dal core (`project_pick_and_open`, `project_pick_and_add_folder`, `project_pick_relocate`), dai
  recenti/ripristino (store) o dal menu dell'app. Nessun comando accetta un percorso o un id di progetto
  dalla WebView; i vecchi `project_open_folder`, `project_add_folder`, `project_relocate` sono stati rimossi.
- **Cartelle fidate** (`src-tauri/src/trust.rs`, tabella `trusted_folders`): fidata = scelta col selettore
  nativo oppure confermata con la domanda nativa "Ti fidi di questa cartella? Claude potrà eseguire comandi
  al suo interno senza chiedere" (Mi fido / Apri in sola consultazione), mostrata dal core come foglio della
  finestra (`trust_accept`): una pagina compromessa non può dichiarare fidata una cartella da sola. Le cartelle
  già nello store prima di questa funzione non sono fidate (domanda all'apertura). Finché il progetto ha una
  cartella non fidata `conv_start`/`conv_resume`/`conv_send` (e ogni avvio di `claude` in
  `Engine::ensure_running`), `pty_spawn` e la titolazione automatica sono rifiutati con `untrusted_folder`;
  albero ed editor funzionano. La finestra mostra "Cartella non fidata — Claude e terminale disattivati · Mi fido".
- **Permessi Tauri al minimo**, per tipo di finestra (sotto); verificati da `capabilities_grant_no_shell_or_fs_plugin`.
- **Processi senza shell**: sempre lista di argomenti (sotto).
- **Impostazioni**: `settings_get`/`settings_set` accettano solo chiavi in whitelist (`default_permission_mode`)
  con valore validato.
- **Inventario dei comandi**: `every_command_is_registered_and_listed` verifica che ogni `#[tauri::command]`
  sia registrato in `lib.rs` ed elencato nella tabella qui sotto.

## Comandi Tauri

Ambito: **finestra** = progetto ricavato dall'etichetta della finestra chiamante (`proj-<id>`), mai da un
argomento; le chiamate dalla finestra di benvenuto falliscono. **globale** = non legato a un progetto.
Effetti: F = file, P = processi, R = rete (nessun comando fa richieste di rete proprie; `claude` sì, per suo conto).

| Comando | Dalla WebView | Validazione | Ambito | Effetti |
|---|---|---|---|---|
| `project_current` | — | — | finestra | — |
| `project_rename` | `name` | non vuoto dopo trim | finestra | titolo finestra, menu |
| `project_pick_and_open` | — | il percorso viene dal selettore nativo; diventa fidato | globale | apre una finestra |
| `project_pick_and_add_folder` | — | percorso dal selettore nativo (foglio della finestra); diventa fidato | finestra | allarga il confinamento F |
| `project_pick_relocate` | `old` | deve essere una cartella del progetto; nuovo percorso dal selettore nativo, diventa fidato | finestra | sostituisce una radice F |
| `project_missing_folders` | — | — | finestra | F (stat delle radici) |
| `project_branch` | — | — | finestra | F (legge `.git/HEAD`) |
| `settings_get` | `key` | whitelist | globale | — |
| `settings_set` | `key`, `value` | whitelist + validatore (`valid_permission_mode`) | globale | — |
| `fs_list` | `path` | `ensure_inside` | finestra | F |
| `fs_create` | `path`, `isDir` | `ensure_inside`; non sovrascrive | finestra | F |
| `fs_rename` | `from`, `to` | `ensure_inside` su entrambi; destinazione inesistente | finestra | F |
| `fs_trash` | `path` | `ensure_inside`; Cestino, non cancellazione | finestra | F |
| `fs_read` | `path` | `ensure_inside`; ≤ 5 MB, solo UTF-8 | finestra | F |
| `fs_write` | `path`, `text`, `expectedMtimeMs` | `ensure_inside`; controllo di conflitto | finestra | F |
| `fs_all_files` | — | non segue i link, salta `.git`, ≤ 50 000 voci | finestra | F (lettura) |
| `pty_spawn` | `cwd`, `cols`, `rows` | `ensure_inside` + cartella esistente; progetto fidato; 1–1000; ≤ 16 per finestra | finestra | P (`$SHELL -l`) |
| `pty_write` | `id`, `data` | PTY della stessa finestra; ≤ 1 MB | finestra | P |
| `pty_resize` | `id`, `cols`, `rows` | PTY della stessa finestra; 1–1000 | finestra | P |
| `pty_kill` | `id` | PTY della stessa finestra | finestra | P |
| `conv_list` | `projectId` | deve coincidere con la finestra | finestra | F (legge `~/.claude`, sola lettura); titolazione solo se fidato |
| `conv_history` | `sessionId` | UUID; conversazione del progetto | finestra | F (sola lettura) |
| `conv_search` | `query` | 2–200 caratteri; tempo massimo 3 s | finestra | F (sola lettura) |
| `claude_check` | — | — | globale | P (`claude --version`) |
| `conv_start` | `projectId`, `opts` | coincide con la finestra; `model` (`valid_model`: niente spazi né `-` iniziale), `permissionMode` in whitelist; progetto fidato | finestra | P (`claude`), R |
| `conv_resume` | `sessionId` | UUID; del progetto; progetto fidato | finestra | P, R |
| `conv_send` | `sessionId`, `text`, `imagePaths` | UUID; del progetto; fidato; ≤ 10 immagini, ≤ 20 MB l'una e 30 MB in tutto, solo png/jpg/gif/webp canonicalizzate dentro la cartella immagini dell'app o del progetto | finestra | P, F (lettura immagini) |
| `conv_answer` | `sessionId`, `requestId`, `choice` | del progetto; richiesta ancora aperta; scelta ammessa per il tipo | finestra | P (stdin di `claude`) |
| `conv_interrupt` | `sessionId` | del progetto | finestra | P |
| `conv_set_model` | `sessionId`, `model` | del progetto; `valid_model` | finestra | P |
| `conv_set_permission_mode` | `sessionId`, `mode` | del progetto; whitelist | finestra | P |
| `conv_mark_viewed` | `sessionId` | del progetto | finestra | — |
| `conv_archive` | `sessionId`, `archived` | del progetto | finestra | — |
| `conv_running` | `projectId` | coincide con la finestra | finestra | — |
| `conv_stop` | `sessionId` | del progetto | finestra | P (termina il gruppo di processi) |
| `conv_save_image` | corpo binario, header `ext` | finestra di progetto; estensione in whitelist, byte iniziali coerenti, ≤ 20 MB; nome casuale, cartella 0700, file 0600 | finestra | F (cartella temporanea dell'app) |
| `conv_pending` | `sessionId` | del progetto | finestra | — |
| `conv_rename` | `sessionId`, `title` | del progetto; lunghezza e caratteri di controllo | finestra | — |
| `conv_unlock_title` | `sessionId` | del progetto | finestra | — |
| `trust_check` | — | — | finestra | — |
| `trust_accept` | — | conferma nativa mostrata dal core | finestra | — |

## CSP

```
default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:;
connect-src ipc: http://ipc.localhost; worker-src 'self' blob:; object-src 'none'; base-uri 'none'
```

Rilassamenti e perché:
- `style-src 'unsafe-inline'`: Monaco e xterm creano stili in linea ed elementi `<style>`; anche le
  transizioni di Svelte. Gli script restano solo `'self'` (Tauri aggiunge da sé gli hash dei propri script).
- `img-src data: blob:`: le icone di Monaco sono `data:` nel suo CSS; le anteprime delle immagini incollate
  nel compositore sono URL `blob:`. Nessuna origine remota: il markdown non carica immagini.
- `worker-src 'self' blob:`: i worker di Monaco (`?worker` di Vite, file in `assets/`); `blob:` per il
  meccanismo di riserva di Monaco.
- `connect-src ipc: http://ipc.localhost`: solo il canale IPC di Tauri.
- `devCsp` (solo `pnpm tauri dev`): in più `ws://localhost:1420` per l'HMR di Vite.
- Nessun `unsafe-eval` (verificato: il bundle non usa `eval`/`new Function`), nessuna origine remota,
  `withGlobalTauri` spento, `dangerousDisableAssetCspModification: false`, asset protocol non abilitato.

## Permessi Tauri (capabilities)

`src-tauri/capabilities/project.json` (finestre `proj-*`):
- `core:event:allow-listen`, `core:event:allow-unlisten` — eventi del core (`conv:*`, `fs:changed`, `pty:*`).
  Niente `allow-emit`: la pagina non può inviare eventi alle altre finestre.
- `core:window:allow-start-dragging`, `core:window:allow-internal-toggle-maximize` — barra del titolo
  (`data-tauri-drag-region`, doppio clic).
- `opener:allow-open-url` limitato a `http://*` e `https://*` — link esterni nel browser di sistema.

`src-tauri/capabilities/welcome.json` (finestra `welcome`): solo i due permessi della barra del titolo.

Tolti rispetto a prima: `core:default` (comprendeva menu, emit, webview, app…), `dialog:default` (ora i
dialoghi li apre solo il core). Nessun permesso `shell:` o `fs:`. I comandi dell'app non sono filtrati
per finestra dall'ACL di Tauri: il confine è il controllo dell'etichetta nel core (tabella sopra).

## Processi

Tutti avviati con lista di argomenti, mai tramite shell con testo composto:
- `claude` (`engine.rs`): argomenti costanti + `--permission-mode` (whitelist), `--model` (`valid_model`),
  `--resume`/`--session-id` (UUID validato), `--add-dir` (cartelle del progetto dallo store, fidate);
  `current_dir` = cwd della conversazione, che la sincronizzazione accetta solo dentro le cartelle del progetto.
  Ambiente: `env_clear()` + ambiente della shell di login.
- `claude` per i titoli (`titles.rs`): argomenti costanti, cartella temporanea vuota, prompt su stdin;
  mai per progetti non fidati.
- `claude --version` (`locate.rs`): costante.
- Ambiente della shell di login (`locate.rs`): `$SHELL -l -i -c env`, comando costante; `$SHELL` solo se assoluto.
- Terminale (`pty.rs`): `$SHELL -l` (solo se percorso assoluto esistente, altrimenti `/bin/zsh`), cwd confinato.

## Segreti e log

- L'ambiente della shell di login (contiene i segreti dell'utente) vive solo in `Launch.env` in memoria:
  il tipo non è serializzabile, non viene loggato né scritto su disco, e passa solo ai figli via `envs()`.
  `claude_check` restituisce solo `found`, `version`, `path`.
- Log: un solo `eprintln!` (progetto non riapribile: nome e errore). Nessun contenuto di messaggi, file o
  variabili d'ambiente nei log.
- SQLite (`codeart.db`): progetti (nomi, percorsi), impostazioni, metadati delle conversazioni (titolo,
  riassunto, stato), cartelle fidate. Nessuna credenziale.
- Le immagini incollate restano in `$TMPDIR/CodeArt/images` (0700/0600), cancellate dopo 24 ore.

## Dipendenze (2026-11-11)

- `pnpm audit --prod`: 5 avvisi (1 critico, 1 alto, 3 bassi) su `dompurify` 3.4.15 portato da
  `monaco-editor`. Risolti con l'override `monaco-editor>dompurify: ^3.4.17` in `package.json`.
  Esito dopo: nessuna vulnerabilità nota. Rimosso `@tauri-apps/plugin-dialog` (non più usato dalla pagina).
- `cargo audit` (cargo-audit 0.23, 1453 avvisi nel database): nessuna vulnerabilità. 5 avvisi informativi
  (non mantenuti: `proc-macro-error`, `zbus_names`, `zvariant`, `zvariant_derive`; unsound: `glib` 0.18):
  tutte dipendenze transitive di Tauri/wry per Linux (GTK, D-Bus), non compilate su macOS. Nessuna azione.

## Repository pubblico

Scansione dei file tracciati: nessun percorso personale (solo `/Users/dev` e `/Users/a` nei test), nessuna
email oltre all'attribuzione dei commit, nessun token (`sk-`, `ghp_`, chiavi). `.gitignore` copre anche
`.env*`, database locali (`*.db`, `*.sqlite`), chiavi (`*.pem`, `*.key`) e `.claude/settings.local.json`.

## Rischi residui

- La CLI `claude` in bypass permissions, in una cartella fidata, può fare tutto ciò che può fare l'utente:
  è la scelta voluta, resa visibile (modalità in ambra, domanda di fiducia).
- Una WebView compromessa in un progetto fidato può avviare conversazioni e scrivere nel terminale di quel
  progetto: il confine protegge le altre cartelle e gli altri progetti, non il progetto stesso.
- La cartella temporanea delle immagini è condivisa tra progetti (nomi casuali non indovinabili).
