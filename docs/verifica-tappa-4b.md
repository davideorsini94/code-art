# Verifica della tappa 4b — Oracle

> **Stato: rimandato (decisione dell'utente, 2026-11-23).** L'app installata (`Code Art.app`, firma ad-hoc con hardened
> runtime) non può caricare Instant Client, firmato da Oracle con un altro Team ID: macOS lo rifiuta ("different Team
> IDs"). Funziona solo da sviluppo (`pnpm tauri dev`, `cargo test`), che non ha l'hardened runtime. Strade possibili, da
> decidere: il permesso `com.apple.security.cs.disable-library-validation` con caricamento solo da una cartella scelta col
> selettore nativo, oppure Oracle in un processo separato senza hardened runtime. L'app installata mostra un messaggio
> che lo dice chiaramente invece del rimedio sbagliato (`xattr`).

Test automatici: `pnpm typecheck`, `pnpm test`, `cargo test`, `cargo clippy --all-targets`. **Nessuna prova contro un
server Oracle né con Instant Client installato**: qui non ci sono. Le prove qui sotto sono la prima volta.

## Preparazione

1. Scarica "Instant Client for macOS (ARM64) — Basic Light" da
   <https://www.oracle.com/database/technologies/instant-client/macos-arm64-downloads.html> (la guida dentro l'app apre la
   stessa pagina).
2. Apri il `.dmg` ed esegui `install_ic.sh` (copia la cartella in `~/Downloads/instantclient_…`) o copia la cartella dove
   preferisci.
3. Un server di prova: ad esempio Oracle Database Free in un container (`gvenzl/oracle-free`), servizio `FREEPDB1`.

## Da verificare a vista

- [ ] **Guida**: con una connessione Oracle e Instant Client non configurato compare la guida; "Scegli la cartella…" o
  una cartella trovata la carica; con la cartella sbagliata il messaggio dice cosa fare; se macOS blocca la libreria la
  guida mostra il comando `xattr -dr com.apple.quarantine "<cartella>"` da copiare.
- [ ] **Connessione**: tipo Oracle, porta 1521, servizio; password sbagliata → richiesta di password ("auth"); servizio
  sbagliato → "Servizio sconosciuto sul server"; tunnel SSH funzionante.
- [ ] **Albero**: proprietari → tabelle e viste, senza gli schemi di sistema; nomi in minuscolo fra apici.
- [ ] **Query**: clic su una tabella → `SELECT * FROM "PROPRIETARIO"."TABELLA"`; pagine da 500; NUMBER con decimali, DATE,
  TIMESTAMP (anche WITH TIME ZONE), CLOB e BLOB resi come testo; Annulla durante `BEGIN DBMS_SESSION.SLEEP(30); END;` (blocco
  PL/SQL terminato da una riga con `/`); più istruzioni separate da `;`.
- [ ] **Modifiche in griglia**: tabella con chiave primaria, modifica di NUMBER, DATE e VARCHAR2, riga nuova e riga
  eliminata; anteprima; Applica tutto o niente; colonne LOB, RAW, LONG e TIMESTAMP WITH LOCAL TIME ZONE in sola lettura.
- [ ] **Esporta** CSV/JSON di un risultato Oracle.
- [ ] **Connessione caduta**: ferma il server → "disconnesso" entro un minuto circa (controllo ogni 30 s), "Riconnetti".

## Limiti noti

- Solo TCP: nessun TCPS né wallet. Per cifrare la connessione usa un tunnel SSH.
- Le istruzioni ad hoc sono confermate subito (come in Postgres fuori da un `BEGIN`): non si possono tenere transazioni
  aperte a mano tra un'esecuzione e l'altra; una `SELECT … FOR UPDATE` tiene i lock fino all'istruzione successiva che
  modifica dati.
- In Oracle la stringa vuota è NULL: la griglia non le distingue.
- Cambiare la cartella di Instant Client dopo averla caricata richiede il riavvio dell'app.
- Una connessione che non risponde entro 10 s viene abbandonata, ma il tentativo resta in sospeso nel driver fino al
  timeout del sistema.
- TIMESTAMP WITH LOCAL TIME ZONE in sola lettura in griglia finché non è verificato con un server vero.

## Punti aperti dalla revisione finale (da sistemare quando si riprende Oracle)

- Una connessione Oracle caduta tiene bloccata per circa 3 s la mappa delle sessioni Oracle (`ora.rs`, `ended()` lascia
  cadere l'attore sotto il lucchetto).
- Errore di battitura in `RESOLVE_SQL` (`SYS_CONTEXT('USERDENV', …)` invece di `'USERENV'`): una `SELECT * FROM t` senza
  proprietario non risulta mai modificabile (le schede aperte dall'albero non ne sono toccate).
- TIMESTAMP WITH LOCAL TIME ZONE: il core lo permette ancora (`ora_edit.rs`), lo blocca solo l'interfaccia; serve un flag
  `editable` per colonna condiviso fra core e interfaccia (anche per INTERVAL, ROWID, BFILE, XMLTYPE, JSON; chiavi RAW con
  `HEXTORAW`).
- Connessione senza password salvata: ORA-01005 non è tradotto in `auth`, quindi la richiesta di password non compare.
- Le cartelle `instantclient*` trovate in `~/Downloads` o `~` si possono caricare con una sola chiamata dalla pagina:
  prima di allentare la protezione di macOS, richiedere il selettore nativo (o controllare proprietario e quarantena).
- Il ping di controllo non ha un tempo massimo: su una connessione TCP mezza aperta la sessione resta appesa.
- Il controllo delle istruzioni distruttive non vede `FORALL … DELETE` senza `WHERE`.
- Modificare un TIMESTAMP WITH TIME ZONE riscrive un fuso con nome come scostamento fisso (stesso istante).
