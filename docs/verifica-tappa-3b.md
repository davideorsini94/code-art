# Verifica della tappa 3b — SSH

Test automatici: `pnpm typecheck`, `pnpm test`, `cargo test --offline`, `cargo clippy --all-targets`. Gli script remoti sono
provati eseguendoli con `/bin/sh` e `dash` in locale; **nessun test è stato fatto contro un server vero**: le prove qui
sotto sono la prima volta.

## Da verificare a vista (con un server reale)

- [ ] **Elenco**: la vista SSH (icona server nella barra, ⌘⇧S) mostra gli alias di `~/.ssh/config` ("da ssh config") e i
  server salvati. "＋ server" salva; errori di validazione sotto i campi.
- [ ] **Server nuovo**: alla prima connessione compare il dialogo nativo «Nuovo server SSH» con le impronte SHA256;
  "Annulla" non connette; "Connetti" aggiunge la chiave al `known_hosts` dell'app (non tocca `~/.ssh`). Confronta
  l'impronta con quella del server (`ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub` sul server).
- [ ] **Chiave/agent**: un server con chiave in ssh-agent o `IdentityFile` si connette senza chiedere nulla.
- [ ] **Password**: un server con sola password chiede la password (errore "auth"), la salva nel Portachiavi
  ("Password salvata · Rimuovi") e si connette; alla connessione successiva non la chiede più.
- [ ] **Albero remoto**: espandi, crea file e cartella, rinomina, trascina dentro l'albero per spostare, elimina (dialogo
  nativo «Elimina sul server»). Prova un nome con spazi, apici e `$(…)`.
- [ ] **Editor**: doppio clic apre il file remoto; salva; modifica lo stesso file dal server e salva di nuovo →
  barra «Il file è cambiato sul server» (Ricarica / Sovrascrivi / Annulla).
- [ ] **Trasferimenti**: trascina una cartella dall'albero del progetto su una cartella remota e viceversa; barra
  d'avanzamento nel pannello trasferimenti; annulla a metà → nessun `.codeart-part` rimasto; un nome già presente dà
  errore per quella voce.
- [ ] **Terminale remoto**: "Apri terminale qui" dall'albero remoto apre una scheda `server:cartella` nella cartella
  giusta; si chiude da sola se la connessione cade.
- [ ] **Connessione caduta**: spegni la rete o riavvia sshd → stato "disconnesso", albero in grigio con "Riconnetti", le
  schede dell'editor restano e il salvataggio fallisce con messaggio senza perdere le modifiche; "Riconnetti" ripristina.
- [ ] **Uscita**: chiudendo l'app non restano processi `ssh` (Monitoraggio Attività).

## Limiti noti

- La shell di login sul server deve accettare la sintassi POSIX per il comando (`sh -c '…'`): con `fish` o `csh` come
  shell di login i comandi possono fallire.
- Un server raggiungibile solo con `ProxyJump` deve essere già nel `~/.ssh/known_hosts`: `ssh-keyscan` non attraversa i
  salti, quindi il dialogo di conferma non può mostrare la chiave.
- Chi esegue codice come il tuo utente sul Mac può ottenere la password di un server rilanciando `ssh` con l'app come
  askpass (vedi "Rischi residui" in `docs/sicurezza.md`).
- I file Markdown remoti si aprono come testo, senza anteprima; le anteprime di immagini e PDF remoti non ci sono.
