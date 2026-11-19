# Tappa 3b — SSH — piano

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** server SSH nella barra laterale: elenco (anche da `~/.ssh/config`), connessione con chiavi/agent/password nel
Portachiavi, cartelle remote navigabili e modificabili, trascinamento Mac ↔ server con avanzamento e annullamento,
terminale remoto nelle stesse schede dei terminali locali, file remoti aperti e salvati nell'editor.

**Architecture:** nessuna libreria SSH: il core avvia `/usr/bin/ssh` con una connessione condivisa per server
(ControlMaster, socket nella cartella dati dell'app). Ogni operazione sui file è un piccolo script POSIX costante eseguito
con `sh -c` sul server, con i percorsi passati come argomenti quotati (`quote_sh`), mai incollati nello script. I
trasferimenti passano byte per byte da `cat` (stdin/stdout del processo `ssh`), così l'app conta l'avanzamento e può
annullare. La password, se serve, la fornisce l'app stessa, rilanciata da `ssh` come `SSH_ASKPASS`, leggendola dal
Portachiavi.

**Tech Stack:** Tauri 2 (Rust), OpenSSH di sistema, Security.framework via `core-foundation` 0.10 (già nel `Cargo.lock`
come dipendenza indiretta: si aggiunge a `Cargo.toml` alla stessa versione, nessun download), Svelte 5 + TS, Vitest.

**Spec:** `docs/superpowers/specs/2026-11-11-code-art-ide-design.md` — §2 obiettivo 4, §5 (righe SSH e Segreti, aggiornate
il 2026-11-19), §6 (barra icone: SSH), §9 tappa 3 (punti dopo i terminali locali), §11 (connessioni cadute), §11 bis (SSH,
segreti, processi senza shell).

## Global Constraints

- **Mai una riga di comando composta da testo** sul Mac: `ssh` si avvia con lista di argomenti. Sul server l'unica shell è
  `sh -c '<script costante>' _ <arg1> <arg2>…`, con ogni argomento passato da `quote_sh`. Nessun dato della WebView entra
  nel testo di uno script.
- **Destinazioni validate**: host, utente, alias non vuoti, senza spazi, caratteri di controllo, né `-` iniziale; porta
  1–65535 (la destinazione non si può proteggere con `--`, quindi la validazione è l'unica difesa contro l'iniezione di
  opzioni).
- **Chiavi dei server**: `StrictHostKeyChecking=yes`, `UserKnownHostsFile="~/.ssh/known_hosts" <dati app>/known_hosts`
  (mai scrivere in `~/.ssh`). Server nuovo: impronta mostrata in un dialogo nativo del core (titolo fisso, "Annulla"
  predefinito); solo con conferma si aggiunge a `<dati app>/known_hosts`. Chiave cambiata: connessione bloccata con
  messaggio, nessuna conferma possibile dall'app.
- **Segreti solo nel Portachiavi** (servizio `Code Art SSH`, account = id del server). Mai in SQLite, file, log, eventi o
  messaggi d'errore; la password arriva al core una volta (salvataggio) e non torna mai alla WebView.
- **Fiducia**: tutti i comandi `ssh_*` richiedono che il progetto della finestra sia fidato (`ensure_project_trusted`),
  come terminale e Claude.
- Lavoro bloccante fuori dal thread principale; processi `ssh` nel registro di `tools.rs` (gruppo fermato all'uscita);
  le connessioni condivise si chiudono (`ssh -O exit`) all'uscita dell'app.
- Testi in italiano; icone da `Icon.svelte` (aggiungere `server` e `upload`/`download` se servono, stesso stile, niente
  `{@html}`); ogni pulsante con `use:tooltip`.
- Repository pubblico: nei test solo host finti (`example.test`), percorsi `/home/dev/...`, nessun segreto.
- Commit con `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

## Review Focus

- Nomi di file remoti ostili (spazi, `'`, `"`, `$()`, `;`, a capo, `-` iniziale, UTF-8) → elencati, aperti, rinominati,
  trasferiti ed eliminati senza eseguire nulla e senza errori di parsing.
- Connessione che cade (rete, server riavviato) → stato "disconnesso" visibile, "Riconnetti" con un clic, l'albero e le
  schede dell'editor restano; il file modificato non si perde.
- Trasferimento annullato a metà → nessun file parziale rimasto a destinazione; processo `ssh` terminato.
- Server con chiave cambiata o mai visto → nessuna connessione senza la conferma prevista (o mai, se cambiata).
- Server senza GNU coreutils (BusyBox, BSD) → elenco e lettura funzionano (solo POSIX `sh`, `ls` non usato per il parsing).

---

### Task 1: elenco dei server (core)

**Files:** crea `src-tauri/src/ssh/mod.rs`, `src-tauri/src/ssh/config.rs` (+ test); modifica `src-tauri/src/store.rs`
(migrazione `ssh_servers`), `src-tauri/src/lib.rs`, `src/lib/ipc.ts`, `src/lib/types.ts`.

**Interfaces (Produce):**
- `pub struct Server { id: String, name: String, host: String, user: Option<String>, port: Option<u16>, identity_file:
  Option<String>, start_dir: Option<String>, source: ServerSource /* Saved | Config */, has_password: bool }` (serde
  camelCase).
- `config::parse_ssh_config(text: &str) -> Vec<ConfigHost>`: blocchi `Host` con alias concreti (scarta `*`, `?`, `!`),
  `HostName`, `User`, `Port`, `IdentityFile`; `Include` ignorato. I server da config hanno id `config:<alias>` e si
  connettono per alias (OpenSSH applica la config da sé).
- `pub fn validate_server(s: &ServerInput) -> Result<(), String>` (regole dei vincoli globali).
- Comandi: `ssh_servers() -> Vec<Server>` (salvati + config letta da `~/.ssh/config`, solo lettura, max 256 KB),
  `ssh_server_save(input: ServerInput) -> Server` (crea/aggiorna i salvati; mai quelli da config), `ssh_server_delete(id)`
  (rimuove anche la password dal Portachiavi). Tabella `ssh_servers(id TEXT PRIMARY KEY, name, host, user, port,
  identity_file, start_dir)`.
- `src/lib/ipc.ts`: `sshServers`, `sshServerSave`, `sshServerDelete`; tipo `Server` in `types.ts`.

- [ ] Test: parsing (alias multipli, wildcard scartati, maiuscole/minuscole delle chiavi, `=` come separatore, commenti),
  validazione (spazi, `-` iniziale, controllo, porta 0), salvataggio e cancellazione su DB temporaneo.
- [ ] `cargo test`, `cargo clippy --all-targets` senza avvisi nuovi; commit `feat: elenco dei server SSH`.

### Task 2: Portachiavi e askpass

**Files:** crea `src-tauri/src/ssh/keychain.rs`, `src-tauri/src/ssh/askpass.rs`; modifica `src-tauri/Cargo.toml`
(`core-foundation = "0.10"`), `src-tauri/src/main.rs`, comando di salvataggio password.

**Interfaces (Produce):**
- `keychain::set(account: &str, secret: &str) -> Result<(), String>`, `get(account) -> Result<Option<String>, String>`,
  `delete(account) -> Result<(), String>` — `SecItemAdd`/`SecItemUpdate`/`SecItemCopyMatching`/`SecItemDelete`, classe
  generic password, servizio `Code Art SSH`. FFI dichiarata a mano con `#[link(name = "Security", kind = "framework")]`.
- `askpass::maybe_run() -> bool` chiamata come prima riga di `main()`: se l'ambiente ha `CODEART_ASKPASS_ACCOUNT`, e il
  prompt (`argv[1]`) termina con `password:` (senza distinzione di maiuscole), stampa la password del Portachiavi su
  stdout ed esce 0; altrimenti (passphrase, domande sì/no, nessuna password) esce 1 senza stampare nulla. Nessuna finestra.
- Comando `ssh_password_set(id, password)` / `ssh_password_clear(id)`; `has_password` in `Server` letto dal Portachiavi.

- [ ] Test: `askpass` decide sul prompt (funzione pura `answer_for(prompt, secret) -> Option<String>`); keychain con un
  account di test casuale, `#[ignore]` se l'ambiente non ha un Portachiavi sbloccato (documentato nel test).
- [ ] Commit `feat: password SSH nel Portachiavi`.

### Task 3: connessione condivisa e chiavi dei server

**Files:** crea `src-tauri/src/ssh/conn.rs` (+ test); modifica `ssh/mod.rs`, `lib.rs` (stato, uscita), `ipc.ts`.

**Interfaces (Produce):**
- `pub fn base_args(server: &Server, paths: &SshPaths) -> Vec<String>`: `-o ControlPath=<sock> -o ControlMaster=no
  -o StrictHostKeyChecking=yes -o UserKnownHostsFile=… -o BatchMode=no -o ServerAliveInterval=15
  -o ServerAliveCountMax=3`, porta/utente/identità per i salvati, poi la destinazione. Socket: `<dati app>/s/<8 hex>`;
  se il percorso supera 100 byte, `$TMPDIR/codeart-<uid>/<8 hex>` con cartella 0700.
- `connect(server)`: `ssh-keygen -F <host> -f <ogni known_hosts>` → noto: avvia il master (`-M -N -o ControlPersist=600`,
  env `SSH_ASKPASS=<eseguibile dell'app>`, `SSH_ASKPASS_REQUIRE=force`, `CODEART_ASKPASS_ACCOUNT=<id>`, `DISPLAY` vuoto
  non serve con `force`); ignoto: `ssh-keyscan -T 5 -p <porta> <host>` → `ssh-keygen -lf -` per le impronte → dialogo
  nativo di conferma → append a `<dati app>/known_hosts`; chiave cambiata (errore OpenSSH "REMOTE HOST IDENTIFICATION HAS
  CHANGED") → `Err` con spiegazione. Autenticazione fallita → `Err("auth")` così l'interfaccia chiede la password.
- `run(server, script: &str, args: &[&str], stdin: Option<…>) -> Output` sopra la connessione condivisa, con timeout e
  limiti come `tools::run_tool`.
- `pub fn quote_sh(s: &str) -> String` (`'…'` con `'` → `'\''`) e `remote_cmd(script, args) -> String`
  (`sh -c <quote(script)> _ <quote(arg)>…`).
- Stato per server: `ssh:state` evento `{ id, state: "connecting"|"connected"|"disconnected"|"error", message? }`; il
  master che termina → `disconnected`. Comandi `ssh_connect(id)`, `ssh_disconnect(id)`, `ssh_states()`. All'uscita
  dell'app `ssh -O exit` per ogni socket.

- [ ] Test: `quote_sh` con nomi ostili (verificato eseguendo `sh -c` in locale su `remote_cmd` e confrontando argv),
  `base_args` (salvato vs config, socket corto/lungo), classificazione degli errori OpenSSH da stderr reali (fixture).
- [ ] Commit `feat: connessione SSH condivisa con verifica della chiave del server`.

### Task 4: file remoti (core)

**Files:** crea `src-tauri/src/ssh/fs.rs` (+ test); modifica `lib.rs`, `ipc.ts`, `types.ts`.

**Interfaces (Produce):**
- Script costanti POSIX (in `fs.rs`), percorsi sempre assoluti o `~` risolto una volta con `printf %s "$HOME"`:
  - `list(dir)` → righe NUL-separate `tipo\tnome` (`d`, `f`, `l`; i nomi con a capo non rompono il parsing); niente `ls`.
  - `stat(path)` → `size:mtime` (`stat -c '%s:%Y'`, ripiego `stat -f '%z:%m'`, ripiego `wc -c` e mtime 0).
  - `read(path)` → contenuto, rifiutato sopra 5 MB o se non UTF-8 (come `fs_read`).
  - `write(path, text, expected_version)` → rilegge `stat`; versione diversa → errore `conflict`; scrittura in
    `path.codeart-tmp` poi `mv -f`.
  - `mkdir(path)`, `rename(from, to)` (destinazione esistente → errore), `delete(path)` (conferma nativa nel core con il
    nome; cartelle con `rm -rf --` dopo conferma che dice "e tutto il contenuto").
- Comandi `ssh_list(id, dir)`, `ssh_read(id, path)`, `ssh_write(id, path, text, version)`, `ssh_mkdir`, `ssh_rename`,
  `ssh_delete`, `ssh_home(id)`. Percorsi validati: assoluti, senza NUL; `..` ammesso (è il server dell'utente) ma
  normalizzato per la visualizzazione.

- [ ] Test: parser della lista su output con nomi ostili; script eseguiti in locale con `sh` su una cartella temporanea
  (stessa logica che gira sul server) per list/stat/read/write/conflict/mkdir/rename/delete.
- [ ] Commit `feat: file remoti via SSH`.

### Task 5: trasferimenti con avanzamento e annullamento (core)

**Files:** crea `src-tauri/src/ssh/transfer.rs` (+ test); modifica `lib.rs`, `ipc.ts`.

**Interfaces (Produce):**
- `ssh_upload(id, local_paths: Vec<String>, remote_dir) -> transfer_id`, `ssh_download(id, remote_paths: Vec<String>,
  local_dir) -> transfer_id`, `ssh_transfer_cancel(transfer_id)`. Percorsi locali con `ensure_inside` del progetto.
- Cartelle ricorsive: l'app elenca (locale o remoto), crea le cartelle, copia i file uno alla volta con `cat > tmp` /
  `cat --` sopra la connessione condivisa; ogni file scritto prima come `nome.codeart-part` e rinominato alla fine.
  Destinazione esistente: errore per quel file (nessuna sovrascrittura silenziosa) — l'interfaccia mostra l'elenco.
- Evento `ssh:transfer` `{ transferId, done, total, file, state: "running"|"done"|"cancelled"|"error", message? }`, al
  massimo 10 al secondo.
- Annullamento: termina il processo in corso, cancella il `.codeart-part`, stato `cancelled`.

- [ ] Test: upload/download su `sh` locale con file finti (stesso meccanismo `cat`), annullamento a metà lascia la
  destinazione pulita, conflitto di nome.
- [ ] Commit `feat: trasferimenti SSH con avanzamento e annullamento`.

### Task 6: vista SSH nella barra laterale

**Files:** crea `src/lib/stores/ssh.svelte.ts` (+ test), `src/components/ssh/SshPanel.svelte`, `ServerForm.svelte`,
`RemoteTree.svelte` (+ test); modifica `sidebar.svelte.ts` (vista `ssh`), `SidebarView.svelte`, `Rail.svelte`
(icona `server`, ⌘⇧S se libera), `Icon.svelte` (icone nuove).

- Elenco server (salvati e da `~/.ssh/config`, questi con etichetta "da ssh config", non modificabili), stato con pallino,
  Connetti / Disconnetti / Riconnetti. "＋ server" apre il modulo: nome, host, utente, porta, chiave (percorso), cartella
  iniziale, password (facoltativa, campo password, va al Portachiavi; "Password salvata · Rimuovi" se presente).
  Errore `auth` → il modulo chiede la password.
- Connesso: albero remoto sotto il server, dalla cartella iniziale (o home). Espandi; menu contestuale: Nuovo file, Nuova
  cartella, Rinomina, Elimina (conferma nativa del core), Apri terminale qui, Scarica nel progetto… Trascinare dentro
  l'albero sposta (rename). Doppio clic su un file lo apre nell'editor.
- Barra del titolo (§6): il nome del server connesso, se ce n'è uno.

- [ ] Test: store (stati dagli eventi, elenco, errori auth), albero (espansione, menu, nomi strani resi come testo).
- [ ] Commit `feat: vista SSH con server e cartelle remote`.

### Task 7: file remoti nell'editor e trascinamento Mac ↔ server

**Files:** `src/lib/stores/editor.svelte.ts` (+ test), `EditorPane.svelte` se serve, `FileTree.svelte`, `RemoteTree.svelte`,
crea `src/components/ssh/Transfers.svelte`.

- Le schede remote hanno chiave `ssh:<serverId>:<percorso>`, titolo con il nome del file e tooltip `server:percorso`;
  apertura con `sshRead`, salvataggio con `sshWrite` e versione (conflitto → stessa barra "Cambiato altrove" dei file
  locali). Connessione caduta → il salvataggio fallisce con messaggio e la scheda resta modificata.
- Trascinamento: file/cartelle dall'albero del progetto su una cartella remota = upload; dall'albero remoto su una cartella
  del progetto = download. Pannello trasferimenti (in fondo alla vista SSH) con barra d'avanzamento, file corrente,
  Annulla, errori per file.

- [ ] Test: editor (apertura/salvataggio/conflitto remoti con ipc finto), trascinamento che chiama upload/download con i
  percorsi giusti, pannello trasferimenti dagli eventi.
- [ ] Commit `feat: file remoti nell'editor e trascinamento tra Mac e server`.

### Task 8: terminale remoto

**Files:** `src-tauri/src/pty.rs` (comando `pty_spawn_ssh`), `ipc.ts`, `terminals.svelte.ts` (`openRemote(serverId,
dir)`), `TerminalPane.svelte`.

- `pty_spawn_ssh(server_id, dir, cols, rows)`: `ssh -t <base_args> <remote_cmd('cd -- "$1" && exec "${SHELL:-sh}" -l',
  [dir])>` nello stesso registro dei PTY (stessi eventi `pty:data`/`pty:exit`, stesso limite per finestra).
- Le schede remote nella stessa barra: titolo `server:cartella`, icona `server`, stesso comportamento (rinomina,
  riordino, chiusura). "Apri terminale qui" dall'albero remoto.

- [ ] Test: argomenti del comando (nessuna interpolazione di `dir` nello script), richiesta remota dallo store apre una
  scheda remota.
- [ ] Commit `feat: terminale remoto`.

### Task 9: documentazione e verifica

**Files:** `docs/sicurezza.md` (comandi `ssh_*`, askpass, known_hosts dell'app, Portachiavi), `docs/verifica-tappa-3b.md`
(cosa provare a vista, con un server reale), `README.md`.

- [ ] Tutti i controlli verdi (`pnpm typecheck`, `pnpm test`, `cargo test`, `cargo clippy`, `pnpm build`); commit
  `docs: tappa 3b`.
