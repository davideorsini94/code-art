# Tappa 4b — Database: Oracle — piano

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** connessioni Oracle con la stessa esperienza di Postgres (albero, query paginate, griglia, esportazione,
modifiche in griglia con anteprima e transazione), più la guida all'installazione di Oracle Instant Client.

**Architecture:** crate `oracle` (rust-oracle, ODPI-C compilato nel crate) che carica a runtime la libreria
`libclntsh.dylib` di Instant Client dalla cartella scelta dall'utente (`InitParams::oracle_client_lib_dir`, una volta per
processo). Le chiamate del driver sono bloccanti: ogni sessione Oracle vive in un **thread dedicato** che possiede la
`Connection` e serve richieste da un canale (connessione, query, pagine, modifiche, chiusura); così un `ResultSet` aperto
resta nel thread tra una pagina e l'altra. Risultati, griglia, esportazione e interfaccia riusano le forme della 4a
(`QueryResult`, `QueryPage`, `resultId`), con id nell'intervallo `2^61 ≤ id < 2^62` (Mongo resta `≥ 2^62`).

**Tech Stack:** Tauri 2 (Rust), `oracle` crate, Oracle Instant Client (Basic o Basic Light, arm64) installato
dall'utente, Svelte 5 + TS, Vitest. Installazione dei crate autorizzata dall'utente (2026-11-19).

**Spec:** `docs/superpowers/specs/2026-11-11-code-art-ide-design.md` — §9 tappa 4 ("4b Oracle (richiede l'installazione di
Oracle Instant Client, guidata dall'app)"), §11, §11 bis; piano e limiti della 4a
(`docs/superpowers/plans/2026-11-19-code-art-tappa-4a-database.md`, `docs/verifica-tappa-4a.md`).

## Global Constraints

- Tutti i vincoli della 4a valgono anche qui: parametri separati dal SQL (`:1`, `:2`… con conversione esplicita), nomi
  citati con doppi apici (Oracle distingue maiuscole fra apici: si usano i nomi esatti del catalogo), segreti solo nel
  Portachiavi (`Code Art DB`), conferma nativa per istruzioni distruttive e per applicare le modifiche, fiducia richiesta da
  ogni comando `db_*` (classificati in `tests/security.rs` e in `docs/sicurezza.md`), limiti 500 righe / 100 000 / 64 KB,
  risultato aperto chiuso dopo 90 s di inattività, nessuna libreria npm nuova.
- **Instant Client**: mai scaricato dall'app. La cartella è scelta con il selettore nativo (o trovata fra i percorsi
  noti) e accettata solo se contiene `libclntsh.dylib` come file regolare dell'utente (niente link fuori dalla cartella);
  salvata nelle impostazioni dell'app (`oracle_client_dir`, non modificabile dalla WebView se non tramite il selettore).
  Cambiarla dopo il primo caricamento richiede il riavvio dell'app (detto chiaramente).
- **Sessione**: `NLS_DATE_FORMAT='YYYY-MM-DD HH24:MI:SS'`, `NLS_TIMESTAMP_FORMAT='YYYY-MM-DD HH24:MI:SS.FF'`,
  `NLS_TIMESTAMP_TZ_FORMAT='YYYY-MM-DD HH24:MI:SS.FF TZH:TZM'`, `NLS_NUMERIC_CHARACTERS='.,'` subito dopo la connessione,
  così i valori mostrati come testo si riscrivono identici.
- **Transazioni**: le istruzioni ad hoc girano con autocommit attivo, come in Postgres fuori da un `BEGIN` [scelta mia,
  coerente con la 4a; dichiarata]. Le modifiche della griglia girano in una transazione esplicita (autocommit spento per la
  durata, `COMMIT`/`ROLLBACK`).
- **Rete**: solo TCP (nessun TCPS/wallet in questa tappa: il selettore TLS è nascosto per Oracle e il limite è dichiarato);
  tunnel SSH come Postgres.
- Testi in italiano; ogni pulsante con `use:tooltip`; repository pubblico (nessun dato reale); commit con
  `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

## Review Focus

- Instant Client assente, cartella sbagliata, architettura sbagliata (x86_64) o libreria non firmata/bloccata da macOS →
  messaggio chiaro con il passo da fare, nessun crash dell'app.
- Nomi Oracle in maiuscolo, minuscolo fra apici, con spazi o `$`/`#` → albero, `SELECT` di apertura e istruzioni generate
  corretti.
- Tipi Oracle (NUMBER con decimali, DATE, TIMESTAMP WITH TIME ZONE, CLOB, BLOB/RAW, NULL vs stringa vuota — in Oracle `''`
  è NULL) → testo fedele; la griglia non promette di distinguere `''` da NULL.
- Query lunga annullata (`dbms_session.sleep` / `dbms_lock.sleep`) → si ferma, la sessione resta utilizzabile.
- Connessione caduta o server riavviato → stato visibile, "Riconnetti", nessuna modifica applicata a metà.

---

### Task 1: Instant Client (core) e guida

**Files:** crea `src-tauri/src/db/oracle_client.rs` (+ test); modifica `Cargo.toml` (`oracle`), `projects.rs`/impostazioni se
serve, `db/mod.rs`, `lib.rs`, `ipc.ts`, `types.ts`, `docs/sicurezza.md`, `tests/security.rs`.

**Interfaces (Produce):**
- `fn validate_client_dir(dir: &Path) -> Result<PathBuf, String>`: cartella esistente, `libclntsh.dylib` regolare (o link
  che resta dentro la cartella), canonicalizzata.
- `fn detect() -> Vec<PathBuf>`: percorsi noti (`~/Downloads/instantclient_*`, `/opt/oracle/instantclient*`,
  `~/instantclient_*`, `/usr/local/lib`, `/opt/homebrew/lib`), solo quelli validi.
- `fn ensure_initialized(dir) -> Result<(), String>`: `oracle::InitParams` con `oracle_client_lib_dir` una sola volta
  (`OnceLock`); errore di caricamento (architettura, quarantena di macOS) tradotto in italiano con il rimedio
  ("scarica la versione arm64", "esegui `xattr -dr com.apple.quarantine <cartella>`").
- Comandi `db_oracle_client_status() -> { dir: Option<String>, loaded: bool, detected: Vec<String>, error: Option<String> }`,
  `db_oracle_client_pick() -> Option<status>` (selettore nativo), `db_oracle_client_use(dir)` solo per una voce di
  `detected` (riconfermata nel core).

- [ ] Test: `validate_client_dir` (cartella vuota, file finto `libclntsh.dylib`, link che esce dalla cartella), `detect`
  su una home finta, traduzione degli errori di caricamento (stringhe tipiche di ODPI-C/dlopen).
- [ ] Commit `feat: Oracle Instant Client: rilevamento e configurazione`.

### Task 2: sessione Oracle in un thread dedicato

**Files:** crea `src-tauri/src/db/ora.rs` (+ test); modifica `db/mod.rs` (`DbKind::Oracle`, porta predefinita 1521,
`database` = nome del servizio, migrazione se il tipo è vincolato), `db/tunnel.rs` (riuso), `lib.rs` (uscita).

**Interfaces (Produce):**
- `OraSessions` con `connect(id, params, password, tunnel)`, `disconnect(id)`, `states()`; thread per sessione con canale
  di richieste; stringa di connessione `//host:port/servizio` costruita dai campi (mai la password nella stringa:
  `Connector::new(user, password, connect_string)`), timeout 10 s, NLS impostati, autocommit attivo; `break_execution`
  per annullare; caduta → `disconnected`; errori `ORA-01017` → `auth`, `ORA-12541`/`12545`/`12170` → "Server non
  raggiungibile", `ORA-12514` → "Servizio sconosciuto sul server".
- `db_connect`/`db_disconnect`/`db_states`/uscita dell'app smistano anche su Oracle; Instant Client non configurato →
  errore con codice `oracle_client` (l'interfaccia apre la guida).

- [ ] Test: costruzione dei parametri e della stringa di connessione (Debug senza password), mappatura degli errori ORA,
  protocollo del thread con un finto backend (richieste in ordine, chiusura pulita, nessun thread rimasto).
- [ ] Commit `feat: sessione Oracle`.

### Task 3: albero, query e modifiche Oracle (core)

**Files:** crea `src-tauri/src/db/ora_query.rs`, `src-tauri/src/db/ora_edit.rs` (+ test); modifica `db/mod.rs`, `ipc.ts`,
`types.ts`, `db/export.rs` (smistamento), `docs/sicurezza.md`, `tests/security.rs`.

**Interfaces (Produce):**
- `db_ora_tree(id)`: proprietari con tabelle/viste da `ALL_TABLES`/`ALL_VIEWS` esclusi gli schemi di sistema
  (`SYS`, `SYSTEM`, `XDB`, `MDSYS`, `CTXSYS`, `ORDSYS`, `OUTLN`, `DBSNMP`, `APPQOSSYS`, `WMSYS`, `GSMADMIN_INTERNAL`,
  `LBACSYS`, `OJVMSYS`, `DVSYS`, `AUDSYS`, … e `ORACLE_MAINTAINED = 'Y'` dove la colonna esiste), stessa forma di
  `PgTree`; `db_ora_columns(id, owner, table)` con tipo (da `ALL_TAB_COLUMNS`: `DATA_TYPE`, lunghezza/precisione/scala),
  nullable, PK (`ALL_CONSTRAINTS` `P` + `ALL_CONS_COLUMNS`).
- `db_query` per Oracle: un'istruzione per volta (Oracle non accetta `;` multipli: il core divide sui `;` di primo
  livello con il lessico della 4a esteso alle stringhe `q'[…]'` e rifiuta i blocchi PL/SQL multipli con un messaggio);
  le `SELECT`/`WITH` tengono il `ResultSet` nel thread (pagine di 500, valori come testo: NUMBER/DATE/TIMESTAMP via NLS,
  CLOB letto fino a 64 KB, BLOB/RAW in esadecimale troncato); DML/DDL → righe interessate. `editableTable` calcolato come
  nella 4a (sintassi `SELECT * FROM "OWNER"."TABELLA"` più WHERE/ORDER BY/OFFSET…FETCH).
  `db_query_more`/`close`/`cancel`/`db_export` smistano sugli id Oracle; chiusura dopo 90 s di inattività.
- Controllo distruttivo della 4a anche su Oracle (più `ALTER … DROP`, `PURGE`).
- `ora_edit::plan_edits` come `pg_edit` con `:n` e conversione esplicita per tipo (`TO_NUMBER(:n)`,
  `TO_DATE(:n, 'YYYY-MM-DD HH24:MI:SS')`, `TO_TIMESTAMP(…)`, `TO_TIMESTAMP_TZ(…)`, testo diretto; CLOB/BLOB/RAW/LONG non
  modificabili in griglia); `db_ora_preview_edits`, `db_ora_apply_edits` (conferma nativa, transazione esplicita, una riga
  esatta per istruzione, rollback su ogni errore).

- [ ] Test: divisione delle istruzioni (stringhe `q'…'`, commenti, blocchi `BEGIN … END;`), conversione dei valori in testo
  (funzioni pure sui tipi del driver dove possibile), `plan_edits` (nomi con minuscole fra apici, `$`/`#`, PK composta,
  tipi, colonne LOB rifiutate), smistamento degli id.
- [ ] Commit `feat: albero, query e modifiche Oracle`.

### Task 4: interfaccia Oracle

**Files:** `src/components/db/DbForm.svelte`, `DbPanel.svelte`, `DbTree.svelte`, `QueryTab.svelte`, `ResultGrid.svelte` (se
serve), crea `src/components/db/OracleClientGuide.svelte` (+ test); `src/lib/stores/db.svelte.ts`.

- Modulo: tipo "Oracle" (porta 1521, campo "Servizio" al posto di "Database", nessun selettore TLS: nota "solo TCP; per
  la cifratura usa un tunnel SSH").
- Guida Instant Client (aperta dal pannello quando manca o dall'errore `oracle_client`): passi numerati — 1. scarica
  "Instant Client for macOS (ARM64) — Basic Light" dalla pagina di Oracle (link aperto nel browser con `openExternal`:
  `https://www.oracle.com/database/technologies/instant-client/macos-arm64-downloads.html`), 2. apri il `.dmg` ed esegui
  `install_ic.sh` o copia la cartella, 3. "Scegli la cartella…" (selettore nativo) oppure una delle cartelle trovate;
  stato (trovato / caricato / errore con rimedio, comando copiabile per la quarantena); nota del riavvio se si cambia
  cartella dopo il caricamento.
- Albero Oracle (proprietari → tabelle/viste), apertura con `SELECT * FROM "OWNER"."TABELLA"`, griglia modificabile con
  le stesse regole (Oracle: nessuna distinzione `''`/NULL nell'editor, colonne LOB in sola lettura).

- [ ] Test: modulo Oracle (porta, servizio, niente TLS), guida (passi, selettore, cartelle trovate, errore con rimedio),
  albero e apertura tabella con identificatori citati, smistamento dei comandi `db_ora_*` per le schede Oracle.
- [ ] Commit `feat: interfaccia Oracle con guida a Instant Client`.

### Task 5: documentazione e verifica

**Files:** `docs/verifica-tappa-4b.md`, `docs/sicurezza.md`, `README.md`.

- [ ] Tutti i controlli verdi (`pnpm typecheck`, `pnpm test`, `cargo test`, `cargo clippy`, `pnpm build`, `cargo audit`,
  `pnpm audit`); commit `docs: tappa 4b`.
