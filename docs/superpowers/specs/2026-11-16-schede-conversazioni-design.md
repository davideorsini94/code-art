# Conversazioni a schede — design

Data: 2026-11-16. Modifica piccola, fuori dalle tappe.

## Perché

Richiesta dell'utente: le conversazioni di Claude devono essere quelle della cartella di lavoro e
apparire come schede in alto, come i file aperti in VS Code. Oggi l'elenco occupa una colonna intera
(`convlist`) con schede alte, accanto alla colonna della conversazione (`conv`).

## Obiettivi

- In cima alla colonna della conversazione, una barra di schede orizzontali: una per ogni conversazione
  **aperta** (scelta A dell'utente), chiudibile con ×. Clic = la mostra.
- L'elenco completo diventa una vista **"Conversazioni" della barra laterale**, accanto a Esplora, Cerca
  e Git (scelta 1 dell'utente). La colonna `convlist` sparisce.
- Le conversazioni restano filtrate sulla cartella del progetto, con il filtro del core di oggi
  (`cwd_in_project`): nessun cambio nel core.

## Non obiettivi

- Nessun cambio a come si leggono le sessioni da `~/.claude` o al database.
- Niente trascinamento per riordinare le schede (si può aggiungere dopo).
- Le sessioni avviate in una sottocartella del progetto restano escluse, come oggi.

## Approcci considerati

1. **Vista nella barra laterale + schede in alto** — scelto. Riusa `ConvList`/`ConvCard` (stato,
   attesa, menu rinomina/archivia/elimina) e segue lo schema di VS Code.
2. Tendina "Tutte ▾" in fondo alla barra delle schede — scartato: componente nuovo da costruire, e
   le conversazioni in attesa non aperte non si vedono a tendina chiusa.

## Comportamento

- **Aprire**: clic su una conversazione nella vista laterale, clic su una notifica (`conv:focus`),
  "＋ nuova", palette → la conversazione si aggiunge alle schede (in coda, se non c'è già) e diventa
  attiva.
- **Chiudere**: × o clic centrale sulla scheda. Chiudere l'attiva attiva la vicina (a destra, altrimenti
  a sinistra), o nessuna. Chiudere non ferma Claude né archivia: la conversazione resta nell'elenco.
  Il pallino di stato continua a vedersi nella vista laterale.
- **Rimossa o archiviata** (`conv:removed`, archivia): la sua scheda si chiude.
- **Scheda**: titolo troncato (tooltip col titolo intero), pallino di stato con gli stessi colori di
  `ConvCard` (lavora / aspetta te / errore), × visibile su hover e sempre sull'attiva. Barra scorrevole
  in orizzontale se le schede non ci stanno; l'attiva viene portata in vista.
- **Memoria**: schede aperte e attiva ricordate per progetto in `localStorage`
  (`codeart.convtabs.<projectId>`); all'avvio si tengono solo quelle ancora presenti nell'elenco.
- **Nessuna scheda aperta**: la colonna mostra un invito ("Apri una conversazione dall'elenco o premi
  ＋ nuova").
- **Rail**: il pulsante "☰ Conversazioni" diventa la vista laterale (come Esplora/Cerca/Git); il
  pulsante della colonna conversazione resta. La regola "almeno una tra `convlist` e `conv` aperta"
  sparisce; le impostazioni salvate che citano `convlist` vengono ignorate senza errori.

## Lasciato a chi costruisce

Stile esatto della barra (coerente col tema scuro e con le schede dell'editor, se esistono), icona
della vista laterale, scorciatoie (es. ⌘W resta all'editor).
