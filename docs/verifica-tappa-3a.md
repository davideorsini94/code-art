# Verifica della tappa 3a — terminali locali

Test automatici: `pnpm typecheck`, `pnpm test`, `cargo test`, `cargo clippy` (nessun avviso nuovo).

## Da verificare a vista

Su un progetto con più repository (una cartella che ne contiene diversi):

- [ ] **＋ del terminale**: apre un menu con le cartelle del progetto, i repository trovati, "Cartella del file aperto"
  (solo con un file aperto) e "Altra cartella…". Con una sola scelta possibile apre subito; ⌘-clic apre sempre nella
  cartella principale. Un secondo clic sul ＋ chiude il menu.
- [ ] **Altra cartella…**: il selettore nativo parte dalla cartella principale; una cartella fuori dal progetto viene
  rifiutata con un messaggio nel pannello, senza aprire terminali; "Annulla" non fa nulla.
- [ ] **Titolo della scheda**: nome della cartella e, se è dentro un repository, ` · branch`. Cambiando branch (anche da
  riga di comando) il titolo si aggiorna. Il tooltip mostra il percorso relativo e la shell.
- [ ] **Rinomina**: doppio clic sul titolo, Invio conferma, Esc annulla, testo vuoto torna al titolo automatico. Un titolo
  rinominato non cambia più col branch.
- [ ] **Riordino**: trascinando una scheda compare l'indicatore di inserimento; rilasciarla su sé stessa o fuori dalla
  barra non cambia nulla.
- [ ] **Chiusura**: × e clic centrale chiudono; chiudere una scheda subito dopo averla aperta non lascia shell accese
  (Monitoraggio Attività: nessuna `zsh -l` orfana).
- [ ] **Albero**: menu contestuale → "Apri terminale qui" su una cartella apre lì, su un file apre nella sua cartella, anche
  sulle cartelle radice. In un progetto non fidato la voce è spenta con il motivo nel tooltip.
- [ ] **Vista Git**: l'icona del terminale nell'intestazione di ogni repository (o accanto alla barra di sincronizzazione
  con un solo repository) apre un terminale in quel repository e mostra il pannello se era nascosto.
- [ ] **⌃`** continua a mostrare e nascondere il pannello; i link nel terminale si aprono nel browser.
