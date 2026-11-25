# Verifica della tappa 4b — Oracle

> **Stato: ripreso il 2026-11-25.** L'app installata (firma ad-hoc con hardened runtime) rifiutava Instant Client,
> firmato da Oracle con un altro Team ID ("different Team IDs"). Scelta dell'utente: il permesso
> `com.apple.security.cs.disable-library-validation` (`src-tauri/Entitlements.plist`) e Instant Client caricato solo da
> una cartella confermata nel selettore nativo (anche le cartelle trovate aprono il selettore su quella cartella). Da
> verificare a vista nell'app installata: `codesign -d --entitlements - "/Applications/Code Art.app"` deve mostrare il
> permesso.

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
  una cartella trovata apre il selettore e, confermata con «Apri», la carica **nell'app installata** (non solo da sviluppo); con la cartella sbagliata il messaggio dice cosa fare; se macOS blocca la libreria la
  guida mostra il comando `xattr -dr com.apple.quarantine "<cartella>"` da copiare.
- [ ] **Connessione**: tipo Oracle, porta 1521, servizio; password sbagliata → richiesta di password ("auth"); servizio
  sbagliato → "Servizio sconosciuto sul server"; tunnel SSH funzionante.
- [ ] **Albero**: proprietari → tabelle e viste, senza gli schemi di sistema; nomi in minuscolo fra apici.
- [ ] **Query**: clic su una tabella → `SELECT * FROM "PROPRIETARIO"."TABELLA"`; pagine da 500; NUMBER con decimali, DATE,
  TIMESTAMP (anche WITH TIME ZONE), CLOB e BLOB resi come testo; Annulla durante `BEGIN DBMS_SESSION.SLEEP(30); END;` (blocco
  PL/SQL terminato da una riga con `/`); più istruzioni separate da `;`.
- [ ] **Modifiche in griglia**: tabella con chiave primaria, modifica di NUMBER, DATE e VARCHAR2, riga nuova e riga
  eliminata; anteprima; Applica tutto o niente; colonne LOB, RAW, LONG, INTERVAL e TIMESTAMP WITH LOCAL TIME ZONE in sola
  lettura; tabella con chiave RAW in sola lettura; `SELECT * FROM tabella` senza proprietario modificabile.
- [ ] **Connessione senza password salvata** → richiesta di password.
- [ ] **Esporta** CSV/JSON di un risultato Oracle.
- [ ] **Connessione caduta**: ferma il server → "disconnesso" entro un minuto circa (controllo ogni 30 s, al più 10 s
  per controllo anche su una connessione mezza aperta), "Riconnetti".

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

## Punti della revisione finale

Sistemati il 2026-11-25:

- Una connessione caduta non blocca più per 3 s la mappa delle sessioni: l'attore si lascia fuori dal lucchetto e
  `shutdown` dal thread della sessione stesso non attende.
- `RESOLVE_SQL` usa `SYS_CONTEXT('USERENV', …)`: una `SELECT * FROM t` senza proprietario è modificabile.
- Flag `editable` per colonna deciso dal core (`ora_edit::bind_expr`) e usato dall'interfaccia: TIMESTAMP WITH LOCAL TIME
  ZONE, INTERVAL, ROWID, BFILE, XMLTYPE, JSON in sola lettura in entrambi; una chiave primaria di questi tipi (anche RAW)
  rende la tabella in sola lettura.
- ORA-01005 (nessuna password) è tradotto in `auth`: compare la richiesta di password.
- Instant Client si carica solo da una cartella confermata nel selettore nativo (`db_oracle_client_use` tolto).
- Il ping di controllo ha un tempo massimo di 10 s (`set_call_timeout`); DPI-1067 conta come connessione persa.
- Il controllo delle istruzioni distruttive vede `FORALL … DELETE|UPDATE` senza `WHERE`.

Ancora aperto:

- Modificare un TIMESTAMP WITH TIME ZONE riscrive un fuso con nome come scostamento fisso (stesso istante).
- Chiavi RAW modificabili con `HEXTORAW` (oggi la tabella è in sola lettura).
