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
- La griglia è modificabile solo per le schede aperte da una tabella (`SELECT * FROM "schema"."tabella"` senza JOIN);
  rieseguire la query scarta le modifiche non applicate.
- MongoDB: TLS `prefer` = TLS per gli host remoti, nessun TLS in locale o con tunnel (MongoDB non negozia TLS); i
  certificati sono verificati con le radici pubbliche (`webpki-roots`), non con il Portachiavi: una CA privata richiede
  "accetta certificati non validi"; lo stato resta "connesso" anche se il server cade (il driver riprova da sé) e gli errori
  compaiono sulle operazioni; i documenti sono modificabili solo dai risultati di una ricerca, non di un'aggregazione.
