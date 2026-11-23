# Verifica della tappa 4b — Oracle

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
