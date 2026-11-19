# Tappa 4a — Database: Postgres e MongoDB — piano

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** connessioni salvate a Postgres e MongoDB (anche attraverso un server SSH della tappa 3b), albero di schemi/tabelle
e database/collezioni, editor di query con cronologia, risultati in griglia paginata ed esportabile, modifiche in griglia
mostrate in anteprima e applicate solo dopo conferma, in transazione.

**Architecture:** driver nativi nel core (`tokio-postgres` con `postgres-native-tls`, `mongodb`), una sessione per
connessione aperta, tenuta dal core; la WebView chiede operazioni per id di connessione e riceve righe come testo/JSON. Le
query ad hoc di Postgres passano dal protocollo semplice (valori già testo, più istruzioni) dentro un cursore per la
paginazione; le modifiche dalla griglia diventano istruzioni con parametri (`$1…`), mai testo SQL composto con i valori.
Il tunnel SSH riusa la connessione condivisa della 3b (`ssh -O forward -L 127.0.0.1:<porta>:<host>:<porta>`).

**Tech Stack:** Tauri 2 (Rust, tokio), `tokio-postgres`, `postgres-native-tls` + `native-tls`, `mongodb` (+ `bson`),
Svelte 5 + TS, Monaco, Vitest. Installazione dei crate autorizzata dall'utente il 2026-11-19.

**Spec:** `docs/superpowers/specs/2026-11-11-code-art-ide-design.md` — §2 obiettivo 5, §5 (Database, Segreti), §6 (barra
icone: Database), §9 tappa 4 (4a), §10 (pannelli DB caricati al primo uso), §11 (connessioni cadute, query scritte non si
perdono), §11 bis (TLS, parametri separati dal SQL, conferma per istruzioni distruttive senza `WHERE`).

## Global Constraints

- **Parametri sempre separati dal testo SQL** nelle istruzioni generate dalla griglia (`$n` con tipo esplicito); i nomi
  (schema, tabella, colonna) sempre citati con `quote_ident` (doppi apici raddoppiati). Mongo: filtri come documenti BSON,
  mai stringhe valutate.
- **Segreti solo nel Portachiavi** (servizio `Code Art DB`, account = id della connessione), riusando `ssh/keychain.rs`
  generalizzato con il nome del servizio. Mai in SQLite, log, eventi, errori; le stringhe di connessione con password non
  escono mai dal core (errori ripuliti).
- **TLS**: modi `prefer` (predefinito: TLS se il server lo offre), `require`, `disable`; certificati verificati con il
  Portachiavi di sistema (`native-tls`), opzione "accetta certificato non valido" solo esplicita per connessione, mostrata in
  ambra.
- **Conferma nativa** del core (titolo fisso, "Annulla" predefinito) prima di: `UPDATE`/`DELETE` senza `WHERE` nelle query ad
  hoc (controllo lessicale dichiarato "di aiuto", non una garanzia), `DROP`/`TRUNCATE`, e prima di applicare le modifiche
  della griglia (con il riepilogo delle istruzioni).
- **Fiducia**: tutti i comandi `db_*` richiedono il progetto fidato (come `ssh_*`) e vanno classificati in
  `src-tauri/tests/security.rs` e nella tabella di `docs/sicurezza.md`.
- **Limiti**: pagina 500 righe; al massimo 100 000 righe lette per risultato; valori di cella oltre 64 KB troncati con
  indicazione; timeout di connessione 10 s; query annullabile (`CancelToken` / `killOp` non serve: chiusura del cursore).
- Lavoro di rete sul runtime async di Tauri, mai sul thread principale; le sessioni si chiudono all'uscita dell'app.
- Testi in italiano; icone da `Icon.svelte` (aggiungere `database`, `table`, `play`, `history`, `download` se servono,
  stesso stile, niente `{@html}`); ogni pulsante con `use:tooltip`; nessuna libreria npm nuova (griglia virtualizzata
  scritta qui).
- Repository pubblico: test con host finti o cluster temporanei; nessun dato reale.
- Commit con `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

## Review Focus

- Nomi di tabelle/colonne con maiuscole, spazi, apici e parole riservate → albero, `SELECT` di apertura e istruzioni
  generate funzionano (citazione corretta).
- Tipi Postgres non banali (numeric, timestamptz, jsonb, bytea, array, NULL vs stringa vuota) → mostrati fedelmente come
  testo; una modifica in griglia li riscrive senza perdere precisione o confondere NULL e "".
- Connessione che cade a metà (rete, server riavviato) → stato visibile, "Riconnetti", il testo della query resta,
  nessuna modifica applicata a metà (transazione).
- Risultato enorme (milioni di righe) → prima pagina subito, memoria limitata, annullabile.
- Password sbagliata / server senza TLS con modo `require` / certificato non valido → messaggio chiaro, nessun segreto
  nel messaggio.

---

### Task 1: connessioni salvate (core)

**Files:** crea `src-tauri/src/db/mod.rs` (+ test); modifica `ssh/keychain.rs` (servizio come parametro, chiamate SSH
invariate), `store.rs` (migrazione `db_connections`, `db_history`), `lib.rs`, `src/lib/ipc.ts`, `src/lib/types.ts`,
`docs/sicurezza.md`, `tests/security.rs`.

**Interfaces (Produce):**
- `DbConnection { id, kind: "postgres"|"mongo", name, host, port, user, database, tls: "prefer"|"require"|"disable",
  acceptInvalidCerts: bool, sshServerId: Option<String>, hasPassword: bool }`; `DbConnectionInput` senza `hasPassword`.
- Validazione: nome 1–100; host come i server SSH (charset sicuro, niente `-` iniziale); porta 1–65535 (predefinite 5432 /
  27017); utente/database senza caratteri di controllo; `sshServerId` esistente se presente.
- Comandi: `db_connections()`, `db_connection_save(input)`, `db_connection_delete(id)` (toglie anche la password),
  `db_password_set(id, password)`, `db_password_clear(id)`.
- Cronologia: tabella `db_history(id INTEGER PK, connection_id, text, at_ms)`, ultime 200 per connessione; comandi
  `db_history(id)`, `db_history_add(id, text)` (≤ 100 KB; duplicato consecutivo non ripetuto).

- [ ] Test: validazione, salvataggio/cancellazione, cronologia (limite 200, duplicati), keychain generalizzato con le
  chiamate SSH che restano sul servizio `Code Art SSH`.
- [ ] Commit `feat: connessioni ai database salvate`.

### Task 2: sessione Postgres, TLS e tunnel SSH (core)

**Files:** crea `src-tauri/src/db/pg.rs`, `src-tauri/src/db/tunnel.rs` (+ test); modifica `Cargo.toml` (`tokio-postgres`,
`postgres-native-tls`, `native-tls`), `ssh/conn.rs` (funzioni `forward`/`cancel_forward` sulla connessione condivisa),
`db/mod.rs`, `lib.rs`.

**Interfaces (Produce):**
- `tunnel::open(server_id, host, port) -> u16` (porta locale libera su 127.0.0.1, `ssh -O forward -L`), `close(...)`;
  server SSH non connesso → errore "Connetti prima il server SSH".
- `db_connect(id)`, `db_disconnect(id)`, `db_states()`; evento `db:state` `{ id, state, message? }` a tutte le finestre;
  una sessione per connessione (un `Client` più il task della connessione); caduta → `disconnected`.
- TLS secondo il modo; `require` senza TLS → errore chiaro; errori di autenticazione → codice `auth` (l'interfaccia chiede
  la password).

- [ ] Test: argomenti del forward; mappatura degli errori (autenticazione, TLS, rete) senza segreti; test d'integrazione
  `#[ignore]` che avvia un cluster temporaneo (`initdb` + `pg_ctl` in una cartella temporanea, porta libera, socket nella
  stessa cartella; saltato se `initdb` manca) e si connette in `disable` e con password.
- [ ] Commit `feat: sessione Postgres con TLS e tunnel SSH`.

### Task 3: albero e query Postgres (core)

**Files:** crea `src-tauri/src/db/pg_query.rs` (+ test); modifica `db/mod.rs`, `ipc.ts`, `types.ts`.

**Interfaces (Produce):**
- `db_pg_tree(id) -> { schemas: [{ name, tables: [{ name, kind: "table"|"view", }] }] }` (da `information_schema` /
  `pg_catalog`, esclusi gli schemi di sistema); `db_pg_columns(id, schema, table) -> [{ name, type, nullable, isPk }]`.
- `db_query(id, text) -> QueryResult` dove per un'istruzione che restituisce righe si apre un cursore (`BEGIN; DECLARE c
  NO SCROLL CURSOR FOR …; FETCH 500 FROM c` col protocollo semplice: valori già testo, NULL distinto) e il core tiene la
  transazione aperta per le pagine successive; altrimenti si esegue col protocollo semplice e si riportano i conteggi.
  `QueryResult { resultId?, columns: [{ name, typeOid? }], rows: (string|null)[][], hasMore, affected?, notices? }`.
- `db_query_more(resultId) -> { rows, hasMore }`, `db_query_close(resultId)`, `db_query_cancel(id)` (CancelToken).
- Controllo distruttivo lessicale (`UPDATE`/`DELETE` senza `WHERE`, `DROP`, `TRUNCATE`, ignorando commenti e stringhe) →
  conferma nativa prima di eseguire.
- Limiti dei vincoli globali (500 righe, 100 000 totali, celle 64 KB).

- [ ] Test (unitari): analizzatore lessicale (commenti, stringhe con `$$`, `WHERE` dentro una stringa non conta), troncamento
  celle; (integrazione `#[ignore]`, cluster temporaneo): albero con nomi strani, paginazione su 1 200 righe, NULL vs "",
  numeric/jsonb/bytea/array come testo, annullamento di `pg_sleep`.
- [ ] Commit `feat: albero e query Postgres con paginazione`.

### Task 4: modifiche in griglia Postgres (core)

**Files:** crea `src-tauri/src/db/pg_edit.rs` (+ test); modifica `db/mod.rs`, `ipc.ts`.

**Interfaces (Produce):**
- `pub fn plan_edits(table: &TableRef, columns: &[Column], edits: &[RowEdit]) -> Result<Vec<Statement>, String>` (pura):
  `RowEdit` = `Update { pk: Vec<(col, Option<String>)>, set: Vec<(col, Option<String>)> } | Insert { values } | Delete { pk }`;
  `Statement { sql, params: Vec<Option<String>>, preview }` con `quote_ident` per i nomi e `$n::<tipo>` per ogni
  parametro (il tipo dalla colonna, mai dalla WebView: la WebView manda solo nomi di colonna, validati contro
  `db_pg_columns`). Tabella senza chiave primaria → errore (sola lettura).
- `db_pg_preview_edits(id, schema, table, edits) -> Vec<String>` (anteprima leggibile), `db_pg_apply_edits(id, schema,
  table, edits) -> { affected }`: conferma nativa con il riepilogo, poi `BEGIN` / istruzioni / `COMMIT`; ogni `UPDATE`/
  `DELETE` deve toccare esattamente una riga, altrimenti `ROLLBACK` e errore con l'istruzione colpevole.

- [ ] Test: `plan_edits` con nomi strani, NULL vs "", tipi; tabella senza PK; integrazione `#[ignore]`: modifica +
  inserimento + cancellazione in transazione, rollback se una riga non corrisponde più.
- [ ] Commit `feat: modifiche in griglia Postgres con anteprima e transazione`.

### Task 5: MongoDB (core)

**Files:** crea `src-tauri/src/db/mongo.rs` (+ test); modifica `Cargo.toml` (`mongodb`), `db/mod.rs`, `ipc.ts`, `types.ts`.

**Interfaces (Produce):**
- Connessione dai campi (mai una URI scritta dalla WebView): host/porta (o tunnel), utente/password dal Portachiavi,
  `authSource` = database se indicato, TLS secondo il modo; stessi `db_connect`/`db_disconnect`/stato della Task 2
  (dispatch sul tipo).
- `db_mongo_tree(id) -> [{ database, collections: [name] }]`.
- `db_mongo_find(id, db, coll, filter: String, sort: String, projection: String) -> resultId + prima pagina` (JSON esteso
  rilassato in ingresso, analizzato nel core; 500 documenti per pagina, cursore tenuto dal core), `db_mongo_aggregate(id, db,
  coll, pipeline: String)`; `db_query_more`/`db_query_close` condivisi.
- Documenti restituiti come JSON esteso rilassato (stringhe), più colonne di primo livello per la griglia.
- `db_mongo_replace(id, db, coll, id_json, doc_json)` (per `_id`, esattamente un documento), `db_mongo_insert`,
  `db_mongo_delete(id, db, coll, id_json)`: anteprima + conferma nativa.

- [ ] Test: analisi di filtro/ordinamento/pipeline (errori leggibili), conversione BSON → JSON (ObjectId, date, decimal,
  binari), opzioni di connessione costruite dai campi (TLS, authSource) senza segreti nei messaggi.
- [ ] Commit `feat: MongoDB: albero, ricerca, aggregazioni e modifica dei documenti`.

### Task 6: esportazione CSV/JSON (core)

**Files:** crea `src-tauri/src/db/export.rs` (+ test); modifica `db/mod.rs`, `ipc.ts`.

- `db_export(resultId, format: "csv"|"json") -> Option<String>`: selettore nativo di salvataggio (nome suggerito
  `risultato.csv`/`.json`), scrive tutte le righe del risultato (pagine già lette + le restanti, fino al limite di 100 000),
  CSV RFC 4180 (virgolette raddoppiate, celle con a capo), NULL = cella vuota in CSV e `null` in JSON; restituisce il
  percorso scritto.

- [ ] Test: CSV con virgole, virgolette, a capo, NULL; JSON con tipi come stringhe.
- [ ] Commit `feat: esportazione dei risultati in CSV e JSON`.

### Task 7: vista Database nella barra laterale

**Files:** crea `src/lib/stores/db.svelte.ts` (+ test), `src/components/db/DbPanel.svelte`, `DbForm.svelte`,
`DbTree.svelte` (+ test); modifica `sidebar.svelte.ts` (vista `db`), `SidebarView.svelte`, `Rail.svelte` (icona
`database`, ⌘⇧D se libera), `Icon.svelte`. Pannello caricato al primo uso (`import()` dinamico come la vista Git).

- Elenco connessioni con stato, Connetti/Disconnetti/Riconnetti, "＋ connessione" (tipo, nome, host, porta, utente,
  database, TLS, accetta certificati non validi in ambra, server SSH per il tunnel scelto fra quelli della 3b, password
  facoltativa → Portachiavi). Errore `auth` → chiede la password.
- Albero: Postgres schemi → tabelle/viste (clic: apri tabella = `SELECT * FROM "schema"."tabella"` in una scheda query);
  Mongo database → collezioni (clic: ricerca vuota in una scheda query). Menu: Nuova query, Apri, Aggiorna.

- [ ] Test: store (stati, auth), form, albero con nomi strani resi come testo, clic su tabella apre una scheda query.
- [ ] Commit `feat: vista Database con connessioni e albero`.

### Task 8: scheda query e griglia dei risultati

**Files:** crea `src/components/db/QueryTab.svelte`, `ResultGrid.svelte` (+ test), `src/lib/db/grid.ts` (+ test);
modifica `editor.svelte.ts` / `EditorPane.svelte` (nuovo tipo di scheda `query` accanto ai file).

- Scheda query: Monaco (SQL per Postgres, JSON per Mongo con campi filtro/ordinamento/proiezione o pipeline), ▶ Esegui
  (⌘↵), Annulla durante l'esecuzione, cronologia (pannello con le ultime 200, clic = incolla), testo salvato per
  connessione (non si perde chiudendo o se la connessione cade).
- Griglia virtualizzata (righe e colonne), intestazioni ridimensionabili, NULL in grigio corsivo distinto da "", celle
  troncate segnalate, "Carica altre 500" / scorrimento in fondo carica la pagina successiva, conteggio righe, messaggi e
  righe interessate per le istruzioni senza risultato, esporta CSV/JSON.

- [ ] Test: logica della griglia (finestra visibile, larghezze), scheda (esegui, pagina successiva, annulla, cronologia),
  NULL vs "".
- [ ] Commit `feat: scheda query con griglia dei risultati`.

### Task 9: modifica in griglia e documenti Mongo (interfaccia)

**Files:** `ResultGrid.svelte`, crea `src/components/db/EditBar.svelte`, `DocEditor.svelte` (+ test).

- Postgres, solo per le schede aperte da una tabella con chiave primaria: doppio clic modifica la cella (Invio/Esc,
  "Imposta NULL"), righe nuove ed eliminate segnate a colori; barra "N modifiche · Anteprima · Applica · Annulla";
  Anteprima mostra le istruzioni (`db_pg_preview_edits`), Applica chiama `db_pg_apply_edits` (il core chiede conferma) e
  ricarica. Tabelle senza PK: griglia in sola lettura con il motivo.
- Mongo: doppio clic su un documento apre il JSON in Monaco; Salva → anteprima → `db_mongo_replace`; Nuovo documento,
  Elimina.

- [ ] Test: modifiche accumulate e annullate, anteprima e applica con ipc finto, sola lettura senza PK, editor di documenti.
- [ ] Commit `feat: modifica in griglia e documenti Mongo`.

### Task 10: documentazione e verifica

**Files:** `docs/sicurezza.md`, `docs/verifica-tappa-4a.md`, `README.md`.

- [ ] Tutti i controlli verdi (`pnpm typecheck`, `pnpm test`, `cargo test`, `cargo clippy`, `pnpm build`, `cargo audit` se
  disponibile, `pnpm audit`); commit `docs: tappa 4a`.
