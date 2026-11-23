# Verifica della tappa 4a — database (Postgres e MongoDB)

Test automatici: `pnpm typecheck`, `pnpm test`, `cargo test`, `cargo clippy --all-targets`. Postgres è provato anche dal
vivo con test `#[ignore]` che avviano un cluster temporaneo (`initdb`/`pg_ctl` in una cartella usa e getta):
`cargo test db:: -- --ignored`. **MongoDB non è mai stato provato contro un server vero** (`mongod` non è installato qui).

## Da verificare a vista

Postgres (un database tuo, meglio di prova):

- [ ] **Connessione**: vista Database (icona cilindro, ⌘⇧D) → "＋ connessione" → Postgres; senza password chiede la
  password ("auth") e la salva nel Portachiavi; TLS `prefer` con un server che lo offre, `require` con uno che non lo
  offre (messaggio chiaro), "accetta certificati non validi" in ambra.
- [ ] **Tunnel SSH**: una connessione con server SSH della 3b connesso → funziona; con il server SSH disconnesso → messaggio
  "Connetti prima il server SSH".
- [ ] **Albero**: schemi, tabelle e viste; nomi con maiuscole o spazi.
- [ ] **Query**: clic su una tabella apre la scheda con `SELECT * FROM "schema"."tabella"` già eseguita; una tabella grande
  mostra subito 500 righe e le successive scorrendo; ▶ (⌘↵), Annulla durante `SELECT pg_sleep(30)`; NULL in grigio corsivo
  distinto dalla stringa vuota; cronologia; il testo resta dopo aver chiuso e riaperto la scheda.
- [ ] **Istruzioni distruttive**: `DELETE FROM t` senza `WHERE` o `DROP TABLE` → dialogo nativo; "Annulla" non esegue nulla.
- [ ] **Esporta** CSV e JSON (selettore nativo); apri il CSV in Numbers/Excel: virgole, a capo e virgolette corretti.
- [ ] **Modifica in griglia** (tabella con chiave primaria): doppio clic su una cella, "∅" per NULL, "＋ Riga", Elimina riga;
  Anteprima mostra le istruzioni; Applica chiede conferma e applica tutto o niente (prova a cambiare la stessa riga da
  `psql` prima di applicare: errore e nessuna modifica). Tabella senza chiave primaria o vista: sola lettura col motivo.
  Invio o clic altrove su una cella NULL senza scrivere: resta NULL; Maiusc+Invio va a capo; un valore troppo lungo per
  un `varchar(n)` dà errore (niente troncamento); chiudere la scheda con modifiche in sospeso chiede Applica / Scarta / Annulla.
- [ ] **Transazioni tue**: `BEGIN; UPDATE …` poi una `SELECT` (avviso "transazione aperta", al massimo 500 righe), Applica
  della griglia rifiutato ("Chiudi prima la transazione aperta"), poi `ROLLBACK`: l'UPDATE non c'è.
- [ ] **Connessione caduta**: ferma il server → stato disconnesso, "Riconnetti", il testo della query resta.

MongoDB (un server di prova, es. `brew install mongodb-community` o un container):

- [ ] Connessione (anche con utente e `authSource`), albero database → collezioni, clic = ricerca vuota.
- [ ] Filtro `{"campo": {"$gt": 1}}`, ordinamento, proiezione; aggregazione con una pipeline; `$out`/`$merge` chiedono
  conferma.
- [ ] Doppio clic su un documento → editor JSON → Salva (anteprima e conferma); Nuovo documento; Elimina documento.

## Limiti noti

- Postgres: le query con più istruzioni, `EXPLAIN`, `SHOW` o una `SELECT` tra parentesi non usano il cursore: il driver le
  legge tutte in memoria prima di mostrarne 500; l'esportazione di quei risultati contiene solo quelle 500 righe.
- Postgres: le celle oltre 64 KB sono troncate anche nell'esportazione.
- Il controllo delle istruzioni distruttive è lessicale (aiuto, non garanzia): ad esempio `EXPLAIN ANALYZE DELETE …` non
  viene segnalato.
- La griglia è modificabile solo se il core riconosce nel testo eseguito una lettura semplice di una tabella: una sola
  `SELECT * FROM tabella` (con o senza schema, citata o no, risolta dal server col `search_path`) seguita al più da `WHERE`,
  `ORDER BY`, `LIMIT`, `OFFSET`; niente alias, virgole, `JOIN`, `UNION`/`INTERSECT`/`EXCEPT`, `GROUP BY`, `FOR UPDATE`.
  Rieseguire la query scarta le modifiche non applicate; le celle troncate a 64 KB sono in sola lettura.
- Postgres: un risultato con altre righe da leggere tiene aperta una transazione (e i lock in lettura della tabella).
  Dopo **90 s senza pagine né esportazioni** il cursore si chiude da solo (`COMMIT`) e la griglia mostra «risultato chiuso
  dopo 90 s di inattività: riesegui la query»: le righe già lette restano visibili, le altre no.
- Postgres: **un solo risultato aperto per connessione, condiviso da tutte le schede e le finestre**: una query in
  un'altra scheda della stessa connessione chiude il risultato precedente («risultato scaduto, riesegui la query» su
  "Carica altre" o Esporta).
- Postgres: le transazioni aperte da te (`BEGIN`/`START TRANSACTION`) non vengono mai concluse dal core: finché sono aperte
  le query girano senza cursore (al massimo 500 righe, avviso) e Applica della griglia è rifiutato. Il core lo chiede al
  server (`now() = statement_timestamp()`) solo dopo che hai eseguito un `BEGIN`/`START`.
- Il controllo lessicale (istruzioni distruttive, lettura semplice di una tabella) presuppone
  `standard_conforming_strings = on` (il valore predefinito): con `off` una stringa come `'\''` viene letta diversamente
  dal server e un'istruzione distruttiva può non essere segnalata.
- MongoDB: TLS `prefer` = TLS per gli host remoti, nessun TLS in locale o con tunnel (MongoDB non negozia TLS); i
  certificati sono verificati con le radici pubbliche (`webpki-roots`), non con il Portachiavi: una CA privata richiede
  "accetta certificati non validi"; lo stato resta "connesso" anche se il server cade (il driver riprova da sé) e gli errori
  compaiono sulle operazioni; i documenti sono modificabili solo dai risultati di una ricerca, non di un'aggregazione.
- MongoDB: l'editor dei documenti usa il JSON esteso canonico (`{"$numberInt": "5"}`, `{"$numberLong": "5"}`,
  `{"$numberDouble": "1.0"}`…), più verboso ma senza cambi di tipo al salvataggio; un documento oltre 1 MB non si apre
  nell'editor (la riga resta leggibile nella griglia).
