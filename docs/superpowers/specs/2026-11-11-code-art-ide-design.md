# Code Art — IDE personale per Mac, centrato sulle conversazioni Claude

Data: 2026-11-11 · Stato: in attesa di approvazione

Legenda: **[utente]** = detto o confermato dall'utente. **[scelta mia]** = decisione
presa da Claude, non discussa: l'utente può ribaltarla.

## 1. Perché

L'utente sviluppa da mesi quasi solo attraverso Claude Code, usando l'estensione
di VS Code. Lì le conversazioni sono tab dell'editor mescolate ai file, tutte
con titoli poco parlanti. Il tempo perso ogni giorno è **capire quale tab è la
conversazione giusta**. In Mission Control, inoltre, le finestre non dicono a
quale progetto appartengono.

Vuole un IDE suo: leggero, veloce, bello, in cui le conversazioni con Claude
sono il centro e il resto (editor, terminale, git, SSH, database) è a portata
di mano nella stessa finestra.

## 2. Obiettivi

1. **Trovare la conversazione giusta in un colpo d'occhio.** [utente]
   - Il titolo di ogni conversazione descrive il suo argomento principale e si
     aggiorna da solo quando l'argomento cambia. [utente]
   - Ogni conversazione mostra uno stato: *al lavoro* / *aspetta te* /
     *inattiva*. [scelta mia, vista e approvata nel mockup]
   - Esiste una vista d'insieme delle conversazioni del progetto (schede con
     titolo, riassunto di due righe, stato, ultima attività), non una fila di
     tab strette. [scelta mia, approvata nel mockup]
2. **Una finestra = un progetto, con il nome del progetto come titolo della
   finestra macOS**, leggibile in Mission Control. [utente]
3. **Funzionalità da IDE sul modello di VS Code**: albero delle cartelle dei
   progetti aperti, editor, terminale, git/GitHub, ricerca. [utente]
4. **SSH**: terminale sul server e file remoti navigabili come una cartella
   locale, con copia/spostamento tra Mac e server. [utente]
5. **Database**: Postgres, MongoDB, Oracle; query per vedere, aggiungere e
   modificare dati. [utente]
6. **Grafica**: tema scuro riposante in stile "liquid glass" Apple, che
   sorprenda. [utente]
7. **Leggero e rapido**, app installabile su Mac. [utente]

## 3. Non-obiettivi [scelte mie]

- Compatibilità con le estensioni di VS Code o un marketplace di plugin.
- Windows e Linux.
- Sviluppo remoto "alla VS Code Remote" (nessun agente installato sul server;
  Claude gira solo in locale).
- Debugger grafico (breakpoint, step). Si usa il terminale. *Chiesto
  all'utente, non ha risposto su questo punto: resta escluso finché non lo
  chiede; sarebbe una tappa 5 a sé.*
- Tema chiaro.
- Collaborazione in tempo reale, sincronizzazione impostazioni su cloud.
- Pubblicazione su App Store / notarizzazione: è un'app personale.

## 4. Anti-obiettivi (sarebbe un fallimento anche se "funziona")

- Le conversazioni tornano a perdersi fra i tab dei file.
- Il titolo della finestra mostra il file aperto invece del progetto.
- L'app è pesante: avvio lento, RAM da Electron, scroll o digitazione a scatti.
- Il vetro compromette la leggibilità del codice o stanca gli occhi.
- Titoli delle conversazioni che cambiano così spesso da non essere più
  riconoscibili.

## 5. Tecnologia [scelte mie, comunicate all'utente: "scegli tu"]

| Parte | Scelta | Motivo |
|---|---|---|
| Guscio app | **Tauri 2** (Rust + WebView di sistema) | pochi MB, avvio rapido, niente Chromium incorporato |
| Interfaccia | **Svelte 5 + TypeScript + Vite** | runtime minimo, reattività fine |
| Editor | **Monaco** | stesso motore di VS Code: scorciatoie, multi-cursore, diff |
| Terminale | **xterm.js** + PTY in Rust (`portable-pty`) | standard, veloce |
| Claude | **CLI `claude` già installata** (v2.1.x in `~/.local/bin`), pilotata in modalità streaming JSON | riusa login, impostazioni, MCP e sessioni esistenti dell'utente; nessuna chiave API |
| Git | `git` di sistema | affidabile, zero dipendenze |
| GitHub | CLI `gh` già installata e autenticata | PR e repo senza gestire token |
| SSH/SFTP | crate Rust `russh` + `russh-sftp` | nessun processo esterno, trasferimenti con avanzamento |
| Database | `tokio-postgres`, `mongodb`, `oracle` (richiede Oracle Instant Client) | driver nativi |
| Segreti | Portachiavi macOS | password SSH/DB mai su file |
| Dati dell'app | SQLite in `~/Library/Application Support/CodeArt/` | titoli, stati, progetti, connessioni |

Ambiente verificato sulla macchina: macOS 27.2, `cargo`, `node` 24, `pnpm`,
`git`, `gh`, `claude` 2.1.319 presenti.

Nome dell'app: **Code Art** (dal nome della cartella) [scelta mia].

Distribuzione: `.dmg` con firma ad-hoc, installazione trascinando in
Applicazioni [scelta mia].

## 6. La finestra

Layout approvato dall'utente sul mockup
(`.superpowers/brainstorm/41040-1794386226/content/main-window.html`, che fa
da riferimento visivo). Da sinistra a destra:

1. **Barra del titolo**: semafori, nome del progetto in evidenza con branch e
   numero di conversazioni attive, ricerca unica ⌘K (file, conversazioni,
   comandi), indicatore "N ti aspettano", server SSH connesso.
2. **Barra icone**: Esplora, Cerca, Git, SSH, Database, Impostazioni.
3. **Pannello laterale**: albero delle cartelle del progetto; sotto, server
   SSH e database nello stesso albero.
4. **Colonna conversazioni**: le schede delle conversazioni del progetto.
5. **Conversazione aperta**.
6. **Editor** (tab dei file, diff delle modifiche di Claude) con **terminale**
   sotto.

Le colonne sono ridimensionabili e richiudibili; la colonna conversazioni e la
conversazione aperta non spariscono mai entrambe [scelta mia].

**Progetto** = un nome + una o più cartelle. Aprire una cartella crea un
progetto col nome della cartella; il nome è modificabile. Il titolo nativo
della finestra macOS è sempre e solo il nome del progetto. Ogni progetto apre
la propria finestra; riaprire un progetto già aperto porta in primo piano la
sua finestra. All'avvio si riaprono le finestre dell'ultima sessione. [scelta
mia nei dettagli, requisito dell'utente nella sostanza]

**Progetti con più cartelle** [scelta mia]: la prima cartella è la *principale*.
Le nuove conversazioni partono lì, con le altre cartelle aggiunte come
directory di lavoro aggiuntive (`--add-dir`). Il terminale parte nella
principale e il branch in barra del titolo è il suo. Le sessioni esistenti si
importano da tutte le cartelle del progetto in un'unica lista; ogni
conversazione ricorda la cartella in cui è nata e viene ripresa da lì. Nel
modello dati la conversazione appartiene al progetto e porta con sé la sua
cartella di origine.

### Stile visivo

- Finestra completamente opaca (scelta dell'utente dopo averla provata: la
  versione traslucida con vibrancy lasciava vedere il desktop e rendeva
  tutto poco leggibile). L'effetto vetro resta dentro la finestra: pannelli
  come lastre di vetro con bordo luminoso sottile in alto, ombra interna in
  basso, sfocatura e saturazione dello sfondo.
- Sfondo "aurora" scuro (viola, petrolio, magenta spento, blu) a piena
  opacità su base `#0a0b12`, che dà ai pannelli qualcosa da rifrangere.
- **L'area del codice e del terminale resta quasi opaca** per la leggibilità:
  il vetro è nella cornice, non sotto il testo.
- Colori di stato: ambra = aspetta te, verde acqua pulsante = al lavoro,
  grigio = inattiva. Accento blu-lavanda.
- Animazioni brevi (150–250 ms), rispettando "Riduci movimento" di macOS.
- Carattere di sistema (SF Pro) per l'interfaccia, SF Mono per il codice.

## 7. Conversazioni Claude (il cuore)

### Motore
Ogni conversazione è un processo `claude` avviato dalla parte Rust nella
cartella del progetto, con ingresso/uscita in streaming JSON; gli eventi
(testo, uso di strumenti, richieste di permesso, fine turno) vengono inoltrati
all'interfaccia. Riprendere una conversazione usa l'id di sessione di Claude
Code (`--resume`).

**Lasciato al costruttore, da verificare con uno spike come primo passo della
tappa 1:** il protocollo esatto con cui la CLI 2.1.x espone in streaming le
richieste di permesso e le domande all'utente. Se dalla sola CLI non è
praticabile, ripiego: un piccolo processo Node affiancato che usa il Claude
Agent SDK (Node è già sulla macchina). Il resto del design non cambia.

### Sessioni esistenti
All'apertura di un progetto l'app legge le sessioni già presenti in
`~/.claude/projects/<cartella-codificata>/*.jsonl` e le mostra come
conversazioni riprendibili. L'app **non modifica mai** quei file: titoli e
stati vivono nel suo SQLite, legati all'id di sessione. **Unica eccezione,
richiesta dal proprietario dopo la prova** ("devo poter eliminare anche le
conversazioni di Claude"): «Elimina conversazione» sposta nel Cestino di macOS
il file di sessione e l'eventuale cartella `<id>/` accanto, mai cancellazione
definitiva, solo dopo una conferma nativa mostrata dal core (vedi §11 bis).

**Convivenza con VS Code e terminale** [scelta mia]: l'app sorveglia quelle
cartelle di sessioni, quindi una conversazione creata o proseguita altrove
compare e si aggiorna da sola (titolo compreso). Per le sessioni che l'app non
sta pilotando lo stato è sempre *inattiva*: *al lavoro* e *aspetta te* valgono
solo per le conversazioni avviate o riprese da Code Art. Riprendere una
sessione è sempre permesso; se il suo file è stato scritto negli ultimi 60
secondi da un altro programma, l'app avvisa che potrebbe essere aperta altrove.

**Vita dei processi** [scelta mia]: i processi `claude` appartengono all'app,
non alla finestra. Chiudendo una finestra con conversazioni al lavoro si può
scegliere di lasciarle finire: l'app resta attiva nel Dock, le notifiche
riaprono la finestra del progetto, e lì si risponde alle richieste di
permesso. Uscire dall'app (⌘Q) chiede conferma e interrompe i turni in corso;
dopo un'uscita o un crash le conversazioni restano riprendibili ma i turni
interrotti non ripartono da soli. Nessun processo supervisore separato.

### Titoli vivi
- Generati con una chiamata breve e separata alla CLI (`claude -p` con il
  modello più economico) a partire dagli ultimi messaggi: massimo ~6 parole,
  nella lingua della conversazione.
- Quando: dopo il primo scambio; poi alla fine di un turno, solo se sono
  passati almeno 3 turni dall'ultimo titolo. Il modello riceve il titolo
  attuale e deve restituirlo invariato se l'argomento principale non è
  cambiato (stabilità prima di tutto).
- Sotto il titolo resta visibile il titolo precedente con l'ora del cambio.
- L'utente può rinominare a mano; un titolo manuale blocca l'aggiornamento
  automatico finché non lo sblocca.
- Per le sessioni importate il titolo si genera alla prima apertura del
  progetto, in sottofondo, partendo dalle più recenti (massimo 20; le altre
  su richiesta), usando intanto il primo messaggio come titolo provvisorio.
- Insieme al titolo si genera il riassunto di due righe della scheda.

### Stati
- **al lavoro**: un turno è in corso.
- **aspetta te**: c'è una richiesta di permesso o una domanda aperta, oppure
  il turno è finito e l'utente non ha ancora aperto la conversazione.
- **inattiva**: turno finito e già visto.
- Le conversazioni si possono **archiviare** (spariscono dalla lista, restano
  ricercabili).

Ordinamento schede: prima *aspetta te*, poi *al lavoro*, poi le altre per
ultima attività. Il contatore in barra del titolo e il badge sull'icona nel
Dock contano le *aspetta te*. Notifica macOS quando una conversazione passa ad
*aspetta te* e la finestra non è in primo piano.

### Conversazione aperta
- Messaggi, con gli usi di strumenti ripiegati in righe compatte espandibili.
- Richieste di permesso e domande di Claude come riquadro ambra con pulsanti
  di risposta.
- Le modifiche ai file proposte/fatte da Claude si aprono come diff
  nell'editor a fianco con un click.
- Campo di risposta con invio ⌘↵, allegare file trascinandoli dall'albero,
  interrompere il turno, scegliere modello e modalità dei permessi.
- **Parità con l'uso quotidiano dell'utente** [utente: "agenti, plugin, cambio
  modelli, quasi sempre bypass permissions, Playwright e altri plugin, tutto
  quello che serve"]. Poiché il motore è la sua CLI, plugin, skill, server MCP
  (Playwright compreso), agenti e impostazioni funzionano così come sono
  configurati oggi; l'interfaccia della tappa 1 deve inoltre offrire:
  - comandi slash e skill con completamento digitando `/`;
  - subagent mostrati come blocchi annidati ripiegabili, con stato e
    risultato; lista todo della conversazione sempre visibile;
  - cambio modello a conversazione in corso;
  - modalità permessi per conversazione, con **default configurabile
    impostato su bypass permissions**; la modalità attiva è sempre visibile
    accanto al campo di risposta;
  - modalità piano, con il piano mostrato e approvabile con un pulsante;
  - incollare o trascinare immagini e screenshot nel messaggio;
  - compact e clear; indicatore di contesto usato e consumo;
  - ragionamento (thinking) ripiegato, espandibile.
  - Fuori dalla tappa 1 [scelta mia]: torna indietro a un punto precedente
    (rewind) con ripristino dei file.
- Più conversazioni lavorano in parallelo nello stesso progetto.
- ⌘K cerca anche nei titoli e nel testo delle conversazioni.

## 8. Editor, albero, terminale (tappa 1)

- Albero: creare, rinominare, spostare, eliminare (nel Cestino), aggiornamento
  automatico ai cambi su disco, icone per tipo, file ignorati da git attenuati.
- Editor Monaco: tab, evidenziazione della sintassi, cerca/sostituisci,
  multi-cursore, minimap, vista divisa, diff. Salvataggio ⌘S; avviso se il
  file è cambiato su disco (succede spesso: lo modifica Claude) con ricarica
  automatica se non ci sono modifiche locali non salvate.
- Apertura rapida file ⌘P, palette comandi ⌘⇧P, scorciatoie come VS Code.
- Terminale: più schede, shell di login dell'utente, nella cartella del
  progetto.

## 9. Tappe

Ogni tappa ha un suo piano di realizzazione e termina con un'app installabile
e usabile. Ordine proposto da Claude, visto dall'utente.

1. **Il cuore** — sezioni 6, 7, 8.
2. **Strumenti da dev**
   - Git: modifiche, diff, stage, commit, fetch, pull, push, merge, cambio e
     creazione branch, risoluzione conflitti nell'editor. [utente]
   - GitHub: creare repository, pull request (creare, elencare, vedere stato
     e controlli, unire). [utente] Clonare un repo dal proprio account
     [scelta mia].
   - Ricerca e sostituzione in tutto il progetto (con `ripgrep` incluso).
   - Completamento intelligente: TypeScript/JavaScript, JSON, HTML, CSS già in
     Monaco; altri linguaggi tramite i language server installati sulla
     macchina. *I linguaggi da coprire per primi vanno chiesti all'utente
     all'inizio di questa tappa.*
3. **SSH**
   - Elenco server (letti anche da `~/.ssh/config`), chiavi e ssh-agent,
     password nel Portachiavi.
   - Il server compare nell'albero: espandi, crea, rinomina, sposta, elimina.
   - Trascinamento tra Mac e server nei due sensi, con avanzamento e
     annullamento. [utente]
   - Terminale remoto, aperto nella cartella selezionata. [utente]
   - Doppio click apre il file remoto nell'editor; salvare lo riscrive sul
     server [scelta mia].
4. **Database**
   - Connessioni salvate (anche attraverso tunnel SSH verso un server della
     tappa 3 [scelta mia]); albero di schemi/tabelle o collezioni.
   - Editor di query con evidenziazione e cronologia; risultati in griglia,
     paginati, esportabili in CSV/JSON.
   - Modifica in griglia: le celle cambiate, le righe aggiunte ed eliminate
     diventano istruzioni mostrate in anteprima e applicate solo dopo
     conferma, in transazione. Tabelle senza chiave primaria: sola lettura in
     griglia. Mongo: documenti modificabili come JSON. [utente nella sostanza,
     scelte mie nei dettagli]
   - Ordine: 4a Postgres e MongoDB; 4b Oracle (richiede l'installazione di
     Oracle Instant Client, guidata dall'app) [scelta mia].

## 10. Prestazioni (obiettivi misurabili) [scelte mie]

- Finestra visibile entro 1 s dall'avvio a freddo.
- Meno di 250 MB di RAM con un progetto aperto e nessuna conversazione al
  lavoro (esclusi i processi `claude`).
- Dimensione dell'app sotto i 40 MB.
- Digitazione e scroll fluidi su file fino a 5 MB; conversazioni lunghe con
  lista virtualizzata.
- Monaco, terminale, pannelli SSH e DB caricati solo al primo uso.

## 11. Errori e casi limite

- `claude` non trovato o non autenticato: schermata che lo spiega e indica il
  comando da eseguire.
- Processo di una conversazione terminato in modo anomalo: stato visibile
  nella scheda, pulsante "riprendi".
- Chiusura finestra con conversazioni al lavoro: chiede se lasciarle
  terminare in sottofondo o interromperle.
- Generazione del titolo fallita: resta il titolo precedente, nessun errore
  mostrato.
- Connessioni SSH/DB cadute: riconnessione con un click, nessuna perdita delle
  query scritte.

## 11 bis. Sicurezza

Requisito dell'utente (aggiunto in corso d'opera): "un IDE sicuro". Le misure
concrete sono [scelte mie]:

**Principio**: l'interfaccia è una pagina web che mostra contenuti non fidati
(output di Claude, file, risultati di query, nomi di file). Va trattata come
potenzialmente compromessa: il core Rust non si fida di ciò che gli chiede.

- **Nessun contenuto remoto nell'interfaccia.** Content-Security-Policy
  restrittiva: script e stili solo dal pacchetto dell'app, nessun `eval`,
  nessuna connessione di rete dalla WebView. I link si aprono nel browser di
  sistema, mai dentro l'app.
- **Contenuti non fidati sempre neutralizzati.** Markdown di Claude, testo dei
  file, risultati di strumenti e query sono resi come testo o come markdown
  sanificato: mai HTML grezzo, mai `{@html}` su dati non sanificati.
- **Comandi del core con confini.** Ogni comando su file accetta solo percorsi
  dentro le cartelle del progetto della finestra che lo chiama (dopo
  risoluzione di `..` e link simbolici). I permessi Tauri sono il minimo
  necessario, per finestra.
- **Eccezione alla sola lettura di `~/.claude` (richiesta del proprietario).**
  `conv_delete` sposta nel Cestino il file di sessione (e la cartella `<id>/`
  accanto) di una conversazione del progetto della finestra, solo se il
  progetto è fidato: il file si trova unendo le cartelle di sessione note del
  progetto con `<id>.jsonl` (mai un percorso dalla pagina), si rifiutano link
  simbolici e percorsi fuori dalla radice delle sessioni, e la conferma è un
  dialogo nativo aperto dal core (titolo fisso, «Annulla» predefinito) che la
  WebView non può aggirare. È l'unico punto in cui l'app scrive in `~/.claude`.
- **Processi senza shell.** `claude`, `git`, `gh` e simili sono avviati con
  lista di argomenti, mai componendo una riga di comando da testo.
- **Segreti solo nel Portachiavi macOS.** Password SSH e DB non finiscono mai
  in SQLite, file, log o messaggi di errore.
- **Bypass permissions resta una scelta visibile.** È il default voluto
  dall'utente, ma la modalità attiva è sempre in vista (ambra) e alla prima
  apertura di una cartella mai vista l'app chiede conferma che sia fidata
  prima di avviare Claude lì dentro.
- **SSH** (tappa 3): verifica della chiave del server contro `known_hosts`,
  con conferma esplicita per server nuovi e blocco se la chiave cambia.
- **Database** (tappa 4): TLS quando il server lo offre; parametri sempre
  separati dal testo SQL nelle istruzioni generate dalla griglia; conferma
  prima di istruzioni distruttive senza `WHERE`.
- **Nessuna telemetria**, nessuna chiamata di rete dell'app che non sia
  chiesta dall'utente.
- **Dipendenze controllate**: `cargo audit` e `pnpm audit` a ogni fine tappa;
  versioni bloccate dai lockfile.
- **Repository pubblico**: nessun segreto, percorso personale o dato reale nel
  codice, nei test o nelle fixture.

## 12. Verifica

- Rust: test unitari per lettura delle sessioni, macchina degli stati delle
  conversazioni, regole di aggiornamento dei titoli, generazione delle
  istruzioni SQL dalla griglia.
- Interfaccia: test dei componenti con Vitest per lista conversazioni
  (ordinamento, stati) e albero.
- Fine di ogni tappa: prova manuale dell'app installata dal `.dmg` su un
  progetto reale dell'utente, con controllo degli obiettivi di prestazione.

## 13. Lasciato al costruttore

- Struttura dei moduli e dei comandi Tauri, schema SQLite.
- Protocollo esatto con la CLI Claude (vedi spike in §7).
- Prompt per titoli e riassunti.
- Resa precisa del vetro, partendo dal mockup.
- Libreria per la griglia dati e per le liste virtualizzate.
