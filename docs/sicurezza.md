# Sicurezza di Code Art

Verifica della tappa 1 (Task 16) e della tappa 2 (Task 11, sezione "Verifica trasversale della tappa 2"). Requisito del proprietario: "un IDE sicuro".

## Modello di minaccia (in dieci righe)

1. La WebView mostra contenuti non fidati: output di Claude, file, nomi di file, risultati di strumenti.
2. Va quindi trattata come potenzialmente compromessa (XSS da markdown, file o nomi costruiti ad arte).
3. Una WebView compromessa può chiamare qualunque comando Tauri con argomenti arbitrari.
4. Il core Rust è il confine: valida ogni argomento e non si fida di percorsi o id arrivati dalla pagina.
5. Il core avvia `claude` in bypass permissions dentro le cartelle del progetto: lì si esegue codice.
6. Una cartella appena clonata può essere ostile (istruzioni per il modello, script, hook).
7. Quindi Claude, terminale e scritture su disco solo in cartelle fidate, scelte o confermate con UI nativa.
8. L'ambiente della shell di login (con i segreti dell'utente) passa solo ai processi figli, mai alla pagina.
9. Nessuna rete dalla WebView: niente script, stili, immagini o connessioni remote.
10. Il repository è pubblico: nessun segreto, percorso personale o dato reale nei file tracciati.

## Misure attive

- **CSP restrittiva** (`src-tauri/tauri.conf.json`), verificata da `csp_has_no_unsafe_eval_or_remote_origin`.
  Queste misure contro la compromissione della WebView (CSP, nessun contenuto remoto, markdown sanificato,
  testo reso come testo) sono quelle che reggono il peso: in una finestra fidata una pagina compromessa può
  eseguire qualunque cosa come l'utente (vedi "Rischi residui").
- **Markdown sanificato**: `renderMarkdown` (`src/lib/safe-markdown.ts`, marked + DOMPurify con whitelist)
  è l'unica sorgente di `{@html}`; l'HTML grezzo è scartato, nella conversazione le immagini non si caricano mai (nell'anteprima di un file solo
  quelle locali del progetto, come `blob:`, vedi sotto), sopravvivono solo
  link `http(s)` con `rel="noopener noreferrer"`, intercettati (`src/lib/conv/links.ts`) e aperti nel browser
  di sistema. `src/lib/html-sites.test.ts` fallisce se compare un `{@html}` non sanificato o un
  `innerHTML`/`outerHTML`/`insertAdjacentHTML`/`document.write`.
- **Anteprime di file** (markdown, immagini, PDF, audio/video; `src/lib/viewers.ts`): il contenuto arriva solo
  da `fs_read_bytes` (confinato, ≤ 50 MB) e diventa un URL `blob:` con tipo MIME da tabella fissa per
  estensione (mai sniffing, mai il nome del file in HTML). Le immagini (anche SVG) sono mostrate con `<img>`:
  gli script in un SVG non vengono eseguiti. Il PDF è reso da `pdfjs-dist` (caricato al primo uso, worker
  locale) su `<canvas>`, senza scripting, XFA né livello delle annotazioni (quindi nessun link nel PDF) e senza richieste di
  rete (`disableAutoFetch`, `disableStream`, niente `cMapUrl`/font/wasm esterni). Che nulla venga valutato
  come codice lo garantisce la CSP (nessun `unsafe-eval`; il bundle di pdf.js non usa `eval`/`new Function`). Il markdown passa
  solo da `renderMarkdown`; con un risolutore di immagini opzionale (solo percorsi relativi al file, niente
  schema, `//`, percorso assoluto o `..` che esce dal progetto) le immagini locali diventano `<img src="blob:…">`
  (DOMPurify ammette `<img>` con solo `src` `blob:`, `alt`, `title`, `width`, `height`); le immagini remote o
  `data:` restano scartate. Limiti per documento (`src/lib/image-budget.ts`): al massimo 4 caricamenti alla
  volta, 100 immagini distinte, 10 MB l'una (`maxBytes` di `fs_read_bytes`, deciso dai metadati) e 60 MB in
  totale; solo estensioni immagine della tabella MIME; le immagini si caricano quando si avvicinano alla vista;
  le altre mostrano "immagine non caricata". Documenti markdown oltre 400 KB: si aprono in "Modifica" e
  l'anteprima (render sincrono) si fa solo su richiesta; tra 100 e 400 KB l'aggiornamento dal vivo rallenta a
  600 ms. La conversazione chiama `renderMarkdown` senza risolutore: immagini sempre scartate.
- **La WebView non lascia mai la pagina dell'app**: ogni finestra ha una guardia di navigazione
  (`projects::allow_navigation`, `on_navigation`): ammesse solo l'origine dell'app (`tauri://localhost`,
  `http(s)://tauri.localhost`) e, nelle build di sviluppo, l'URL del server di Vite; `window.open` è negato
  (`on_new_window`). Un file o un link trascinati fuori dalle zone di rilascio non navigano: a livello di finestra
  `dragover`/`drop` sono annullati (`src/lib/dropguard.ts`, anche nel benvenuto). Tutti i link esterni (markdown,
  terminale, ⌘-clic in Monaco con `registerLinkOpener`) passano da `openExternal` (`src/lib/external.ts`): solo
  http(s), nel browser di sistema. Test: `navigation_stays_on_the_app_origin`, `dropguard.test.ts`, `external.test.ts`.
- **Confinamento dei file**: i comandi `fs_*` e `pty_spawn` accettano solo percorsi assoluti dentro le
  cartelle del progetto della finestra chiamante (`fs_tree::ensure_inside`: canonicalizzazione, `..`,
  link simbolici e link pendenti che escono → errore `forbidden`). Test: `fs_write_outside_project_is_rejected`
  (passa dallo stesso passo etichetta → radici dei comandi, `fs_tree::mutable_path`).
- **Cartelle nel progetto solo da UI nativa**: una cartella entra in un progetto solo dal selettore nativo
  aperto dal core (`project_pick_and_open`, `project_pick_and_add_folder`, `project_pick_relocate`), dai
  recenti/ripristino (store) o dal menu dell'app. Nessun comando accetta un percorso o un id di progetto
  dalla WebView; i vecchi `project_open_folder`, `project_add_folder`, `project_relocate` sono stati rimossi.
- **Cartelle fidate** (`src-tauri/src/trust.rs`, tabella `trusted_folders`): fidata = scelta col selettore
  nativo oppure confermata con la domanda nativa "Ti fidi di questa cartella? Claude potrà eseguire comandi
  al suo interno senza chiedere" (Mi fido / Apri in sola consultazione; il pulsante predefinito, quello di Invio, è "Apri in sola consultazione" e solo un clic esplicito su "Mi fido" concede la fiducia), mostrata dal core come foglio della
  finestra (`trust_accept`): una pagina compromessa non può dichiarare fidata una cartella da sola. Le cartelle
  già nello store prima di questa funzione non sono fidate (domanda all'apertura). Finché il progetto ha una
  cartella non fidata `conv_start`/`conv_send` (e ogni avvio di `claude` in
  `Engine::ensure_running`), `pty_spawn` e la titolazione automatica sono rifiutati con `untrusted_folder`;
  albero, lettura, ricerca ed editor (in sola lettura) funzionano. La finestra mostra "Cartella non fidata — sola
  lettura; Claude e terminale disattivati · Mi fido". Il dialogo ha un titolo fisso ("Cartella non fidata", non il
  nome del progetto) e mostra i percorsi come testo. Un rifiuto vale per il resto dell'avvio per quel progetto:
  nessuna nuova domanda automatica all'apertura delle finestre, e il "Mi fido" del banner è ignorato per 10 s
  dopo il rifiuto (`trust::may_prompt`, testato); una domanda alla volta per progetto.
- **Sola consultazione = sola lettura reale**: in un progetto con anche una sola cartella non fidata ogni comando
  che modifica il disco (`fs_write`, `fs_create`, `fs_rename`, `fs_trash`) è rifiutato con `untrusted_folder`
  da un unico punto (`fs_tree::mutable_path` → `writable_roots_for_label`). Senza questo una pagina compromessa
  potrebbe piantare file eseguiti altrove: hook in `.git/hooks` o `.claude/settings.json` di una cartella fidata
  annidata, oppure `~/.zshrc` (letto da ogni terminale) se la cartella non fidata è la home. Nell'interfaccia
  editor Monaco `readOnly`, salvataggio disattivato, voci di modifica dell'albero disabilitate.
  Test: `untrusted_project_is_read_only`.
- **Fiducia per percorso**: la fiducia è legata al percorso (normalizzato, senza `/` finale), non all'inode.
  Se una cartella fidata viene cancellata o sostituita (anche da un link simbolico) con altro contenuto allo
  stesso percorso, resta fidata; spostandola o rinominandola invece non lo è più nel nuovo percorso (la si
  riposiziona col selettore, che la rende fidata). La fiducia non si eredita: né sottocartelle né genitori.
- **Permessi Tauri al minimo**, per tipo di finestra (sotto); verificati da `capabilities_grant_no_shell_or_fs_plugin`.
- **Processi senza shell**: sempre lista di argomenti (sotto).
- **Impostazioni**: `settings_get`/`settings_set` accettano solo chiavi in whitelist (`default_permission_mode`, `git_pull_mode`)
  con valore validato.
- **Eliminazione delle conversazioni (unica eccezione alla sola lettura di `~/.claude`)**: richiesta del
  proprietario dopo la prova. `conv_delete` (`src-tauri/src/claude/delete.rs`, `Engine::delete_conversation`)
  sposta nel Cestino di macOS (mai cancellazione definitiva, recuperabile) il file di sessione e l'eventuale
  cartella `<id>/` accanto. Misure: conversazione del progetto della finestra chiamante e progetto fidato
  (`untrusted_folder` altrimenti); `sessionId` validato con `valid_session_id`; il file si individua solo unendo
  le cartelle di sessione note del progetto (`session_dirs_for`) con `<id>.jsonl`, mai da un percorso della
  pagina; rifiutato se è un link simbolico o se, canonicalizzato, esce dalla radice delle sessioni (la cartella
  `<id>/` si sposta solo se è una vera cartella dentro la radice); conferma NATIVA aperta dal core ("Eliminare la
  conversazione?", titolo della conversazione come testo semplice, "Sposta nel Cestino" / "Annulla" con "Annulla"
  predefinito), che la WebView non può aggirare né rispondere; un solo dialogo alla volta per finestra. Poi:
  processo fermato, titolazione tolta dalla coda, riga dello store e cache dei metadati rimosse, evento
  `conv:removed` (Dock ricalcolato). Il watcher vede sparire il file e lo ignora; una sincronizzazione successiva
  non lo reimporta. Dopo la conferma (che può aspettare a lungo) la sessione è "in eliminazione": `send`/`resume`
  sono rifiutati finché dura, e proprietà, fiducia e file sono ricontrollati, così nessun processo può ricreare il
  file. Se la cartella `<id>/` non si sposta l'esito è `deletedWithWarning` (avviso all'utente); se la riga non si
  toglie dallo store (due tentativi) l'errore lo dice e la sincronizzazione successiva scarta le righe con turni il cui
  file non esiste più e senza processo. Il Cestino usa `NSFileManager` (`src-tauri/src/trasher.rs`, lo stesso di `fs_trash`),
  non Finder/AppleScript (niente permesso di Automazione). Test con un finto Cestino su cartelle temporanee (mai il vero Cestino né `~/.claude`).
- **Inventario dei comandi**: `every_command_is_registered_and_listed` verifica che ogni `#[tauri::command]`
  sia registrato in `lib.rs` ed elencato nella tabella qui sotto.

## Verifica trasversale della tappa 2

Test in `src-tauri/tests/security.rs` e `src-tauri/src/untrusted_tests.rs` (nel crate, perché `github` e `lsp`
non sono moduli pubblici):
- **Progetto non fidato**: `every_process_or_write_command_refuses_an_untrusted_project` chiama, su un progetto non
  fidato, ogni comando di git (lettura, scrittura, `fetch`/`pull`/`push`), GitHub su un repository (con un binario
  `gh` inesistente: se partisse qualcosa l'errore sarebbe diverso), i quattro language server, `lsp_send` e
  `search_replace`, e pretende `untrusted_folder`. Eccezioni volute: `gh_accounts`, `gh_repo_list`, `gh_clone`
  (finestra di benvenuto, non eseguono codice di un progetto), `search_start`/`search_cancel` (sola lettura), `git_cancel`
  e `lsp_stop` (fermano solo processi già avviati dalla finestra). `new_process_commands_are_all_classified`
  fa fallire la suite se compare un comando `git_*`/`gh_*`/`lsp_*` non classificato.
- **Nessuna shell**: `stage2_sources_never_spawn_a_shell` scorre i sorgenti non di test della tappa 2 (`search.rs`,
  `tools.rs`, `git/`, `github.rs`, `github/`, `lsp/`) e fallisce su qualunque riferimento a `sh`, `bash`, `zsh` o
  `$SHELL`. Nessuna eccezione necessaria: la sonda del `PATH` di login (`$SHELL -l -i -c env`, comando costante)
  è in `claude/locate.rs`, della tappa 1, e `tools.rs` riusa il suo risultato. Gli altri `-c` sono opzioni di git
  (`git -c core.quotepath=off`, `git switch -c`), non di una shell.
- **Token**: `gh_token_appears_only_in_github_rs` verifica che `GH_TOKEN` compaia nel codice non di test solo in
  `github.rs`.
- Test già presenti, citati qui per completezza: `git_commands_refuse_dash_arguments`,
  `search_never_returns_files_outside_the_project_even_through_symlinks`, `refused_when_untrusted` (git, lsp),
  `replace_refused_when_untrusted_or_outside_project`, `token_never_in_debug_or_errors`.

## Comandi Tauri

Ambito: **finestra** = progetto ricavato dall'etichetta della finestra chiamante (`proj-<id>`), mai da un
argomento; le chiamate dalla finestra di benvenuto falliscono. **globale** = non legato a un progetto.
Effetti: F = file, P = processi, R = rete (nessun comando fa richieste di rete proprie; `claude` sì, per suo conto, `git` per fetch/pull/push avviati dall'utente e `gh` per le operazioni GitHub avviate dall'utente).

| Comando | Dalla WebView | Validazione | Ambito | Effetti |
|---|---|---|---|---|
| `project_current` | — | — | finestra | — |
| `project_rename` | `name` | non vuoto dopo trim | finestra | titolo finestra, menu |
| `project_pick_and_open` | — | il percorso viene dal selettore nativo; diventa fidato | globale | apre una finestra |
| `project_pick_and_add_folder` | — | percorso dal selettore nativo (foglio della finestra); diventa fidato | finestra | allarga il confinamento F |
| `project_pick_relocate` | `old` | deve essere una cartella del progetto; nuovo percorso dal selettore nativo, diventa fidato | finestra | sostituisce una radice F |
| `project_missing_folders` | — | — | finestra | F (stat delle radici, fuori dal thread principale) |
| `project_branch` | `file` (opzionale) | `file` serve solo da prefisso di percorso per scegliere il repository, non viene mai letto; i repository sono quelli di `git_repos` | finestra | F (visita limitata delle cartelle come `git_repos` e lettura di `.git/HEAD` del repository scelto, fuori dal thread principale; non esegue git, perciò senza controllo di fiducia) |
| `settings_get` | `key` | whitelist | globale | — |
| `settings_set` | `key`, `value` | whitelist + validatore (`valid_permission_mode`, `valid_pull_mode`: `ff-only` \| `rebase` \| `merge`) | globale | — |
| `fs_list` | `path` | `ensure_inside` | finestra | F |
| `fs_create` | `path`, `isDir` | `ensure_inside`; progetto fidato; non sovrascrive | finestra | F |
| `fs_rename` | `from`, `to` | `ensure_inside` su entrambi; progetto fidato; destinazione inesistente; `from` mai una cartella del progetto (`movable_path`) | finestra | F |
| `fs_trash` | `path` | `ensure_inside`; progetto fidato; mai una cartella del progetto (`movable_path`); Cestino, non cancellazione | finestra | F |
| `fs_read` | `path` | `ensure_inside`; ≤ 5 MB, solo UTF-8 | finestra | F |
| `fs_read_bytes` | `path`, `maxBytes?` | `ensure_inside` (stesso confinamento di `fs_read`, link simbolici compresi); solo file regolari; ≤ 50 MB e ≤ `maxBytes` se dato (`too_large`, deciso dai metadati prima di leggere); lettura, quindi ammessa anche in progetti non fidati; risposta binaria dell'IPC | finestra | F (lettura) |
| `fs_stat` | `path` | `ensure_inside` (come `fs_read`); solo metadati, nessuna lettura del contenuto | finestra | F (stat) |
| `fs_write` | `path`, `text`, `expectedMtimeMs` | `ensure_inside`; progetto fidato; controllo di conflitto | finestra | F |
| `fs_all_files` | — | non segue i link, salta `.git`, ≤ 50 000 voci | finestra | F (lettura) |
| `ssh_servers` | — | progetto fidato; elenco dei server salvati più gli alias concreti di `$HOME/.ssh/config` (sola lettura, ≤ 256 KB, file assente o illeggibile = nessuno); voci con host, utente o alias non validi scartate; `hasPassword` = esiste l'elemento nel Portachiavi (controllo senza leggere il segreto) | finestra | F (lettura), Portachiavi (esistenza) |
| `ssh_server_save` | `input` | progetto fidato; `validate_server` (host e utente non vuoti, senza spazi né caratteri di controllo né `-` iniziale; host solo lettere e cifre ASCII, `.`, `-`, `_`, `:`, nomi internazionali in punycode; porta 1–65535; nome ≤ 100); mai i server `config:` | finestra | — |
| `ssh_server_delete` | `id` | progetto fidato; mai i server `config:`; elimina prima la password dal Portachiavi (assente = già fatto), poi il server | finestra | Portachiavi |
| `ssh_password_set` | `id`, `password` | progetto fidato; `id` di un server salvato o di un alias attuale di `$HOME/.ssh/config`; password non vuota, ≤ 1024 byte, senza a capo né caratteri nulli; la password non compare mai in log, eventi, errori né torna alla WebView | finestra | Portachiavi (servizio `Code Art SSH`, account = id del server) |
| `ssh_password_clear` | `id` | progetto fidato; `id` come `ssh_password_set`; assente = già fatto | finestra | Portachiavi |
| `ssh_connect` | `id` | progetto fidato; `id` di un server salvato o di un alias attuale di `$HOME/.ssh/config`; destinazione e utente ri-validati (`check_server`, host con `validate_host`); già connesso o in connessione = nulla (decisione e iscrizione sotto un solo lucchetto: niente secondo dialogo né secondo master); socket già usato da un altro server connesso = rifiutato; server sconosciuto (`ssh-keygen -F` su `~/.ssh/known_hosts`, `<dati app>/known_hosts`, `/etc/ssh/ssh_known_hosts*`, con nome e porta reali da `ssh -G`) → `ssh-keyscan`, impronte SHA256 in un dialogo nativo del core (titolo fisso «Nuovo server SSH», «Annulla» predefinito), solo con conferma le righe (tipi noti, base64) si aggiungono a `<dati app>/known_hosts` (0600; mai `~/.ssh`); chiave cambiata = errore senza conferma possibile; errore `auth` = serve una password | globale (connessione condivisa da tutte le finestre; evento `ssh:state` a tutte) | processi `/usr/bin/ssh`, `ssh-keygen`, `ssh-keyscan`; socket 0700 nei dati dell'app; Portachiavi (via askpass) |
| `ssh_disconnect` | `id` | progetto fidato; `ssh -O exit` sul socket; connessione assente = già fatto | globale | processo `ssh` |
| `ssh_states` | — | progetto fidato; solo id, stato e messaggio d'errore di ssh (nessun segreto) | globale | — |
| `ssh_home` | `id` | progetto fidato; `printf %s "$HOME"` sul server; solo risposta assoluta, senza NUL | finestra | processo `ssh` (connessione condivisa) |
| `ssh_list` | `id`, `dir` | progetto fidato; `~` risolto dal core con la home del server; percorso assoluto, senza NUL, ≤ 4096 byte; script costante (glob della shell, niente `ls`), percorso solo in `$1`; ≤ 10 000 voci (`truncated`); nomi con a capo o `-` iniziale intatti (record NUL) | finestra | processo `ssh` |
| `ssh_read` | `id`, `path` | come `ssh_list`; file ≤ 5 MB (deciso sul server prima di leggere) e solo UTF-8 (`too_large`, `not_text`); risponde con testo e versione `size:mtime` | finestra | processo `ssh` |
| `ssh_write` | `id`, `path`, `text`, `version` | come `ssh_list`; `version` vuota o `size:mtime`; lo script ricontrolla la versione sul server (diversa = `conflict`, vuota = il file non deve esistere); scrive in `<path>.codeart-tmp` poi `mv -f` (permessi copiati con `cp -p`; un link si scrive attraverso); il testo passa da stdin, mai nello script | finestra | processo `ssh` |
| `ssh_mkdir` | `id`, `path` | come `ssh_list`; `mkdir --` (esistente = errore) | finestra | processo `ssh` |
| `ssh_rename` | `id`, `from`, `to` | come `ssh_list` su entrambi; destinazione esistente (anche un link rotto) = errore «esiste già»; `mv --` | finestra | processo `ssh` |
| `ssh_delete` | `id`, `path` | come `ssh_list`; mai `/`, la home, né percorsi con `.` o `..`; il core legge il tipo, mostra un dialogo nativo (titolo fisso «Elimina sul server», nome del server e del file con i caratteri di controllo sostituiti, per le cartelle «e tutto il suo contenuto», «Annulla» predefinito) e solo con conferma esegue `rm -rf --`; nessuna conferma dalla WebView; un link si toglie senza seguirlo | finestra | processo `ssh` |
| `ssh_upload` | `id`, `local_paths`, `remote_dir` | progetto fidato; ogni sorgente locale passa da `ensure_inside` sulle cartelle del progetto della finestra (symlink risolti); `remote_dir` come `ssh_list`; cartelle percorse senza seguire i link (i link sono saltati e riportati), `O_NOFOLLOW` sull'apertura; ogni file va in `<nome>.codeart-part` (mai sopra un file esistente: `set -C` e controllo) e si rinomina solo se la destinazione non esiste (`esiste già` = errore per quella voce, le altre continuano); byte da stdin di `cat`, mai nello script; nessun massimo di dimensione (≤ 100 000 voci); un thread per trasferimento; evento `ssh:transfer` (≤ 10 al secondo) solo alla finestra del progetto | finestra | processo `ssh` (connessione condivisa), F (lettura) |
| `ssh_download` | `id`, `remote_paths`, `local_dir` | progetto fidato e cartelle scrivibili; `local_dir` esistente e dentro il progetto (`ensure_inside`); percorsi remoti come `ssh_list`; albero elencato da uno script costante (link e speciali saltati e riportati); nomi relativi controllati (niente `.`, `..`, componenti vuoti) prima di scrivere in locale; file in `<nome>.codeart-part` (`create_new`) poi rinomina solo se la destinazione non esiste; destinazione esistente = errore per quella voce; evento `ssh:transfer` solo alla finestra del progetto | finestra | processo `ssh`, F (scrittura) |
| `ssh_transfer_cancel` | `transfer_id` | nessuna fiducia richiesta (come `git_cancel`: ferma solo processi già avviati); solo un trasferimento avviato dallo stesso progetto, altrimenti nulla; uccide il gruppo del processo `ssh` in corso, cancella il `.codeart-part` (in locale o con `rm -f --` sul server), i file già completati restano; stato `cancelled` | finestra | — |
| `db_connections` | — | progetto fidato; elenco delle connessioni salvate; `hasPassword` = esiste l'elemento nel Portachiavi (controllo senza leggere il segreto) | finestra | Portachiavi (esistenza) |
| `db_connection_save` | `input` | progetto fidato; nome 1–100 senza caratteri di controllo; host con `validate_host` (charset sicuro, niente `-` iniziale); porta 1–65535 (predefinita 5432 Postgres, 27017 Mongo); utente e database senza caratteri di controllo; `sshServerId`, se presente, deve essere un server SSH salvato o un alias attuale di `~/.ssh/config`; salvando di nuovo non si perde la password | finestra | — |
| `db_connection_delete` | `id` | progetto fidato; chiude prima una sessione aperta della connessione (Postgres o MongoDB, anche aperta da un'altra finestra) e il suo tunnel SSH; elimina poi la password dal Portachiavi (assente = già fatto), poi connessione e cronologia in una transazione; un server SSH eliminato non cancella a cascata le connessioni che lo usano (l'id non più valido è un errore alla connessione) | finestra | Portachiavi |
| `db_password_set` | `id`, `password` | progetto fidato; `id` di una connessione salvata; password non vuota, ≤ 1024 byte, senza a capo né caratteri nulli; non compare mai in log, eventi, errori né torna alla WebView | finestra | Portachiavi (servizio `Code Art DB`, account = id della connessione) |
| `db_password_clear` | `id` | progetto fidato; `id` come `db_password_set`; assente = già fatto | finestra | Portachiavi |
| `db_history` | `id` | progetto fidato; `id` di una connessione salvata; ultime 200 voci, dalla più recente | finestra | — |
| `db_history_add` | `id`, `text` | progetto fidato; `id` di una connessione salvata; testo non vuoto, ≤ 100 KB; non si ripete se uguale alla voce più recente della connessione; si tengono le ultime 200 (le più vecchie tolte nella stessa transazione) | finestra | — |
| `db_connect` | `id` | progetto fidato; `id` di una connessione salvata (Postgres o MongoDB); già connessa o in connessione = nulla; password letta dal Portachiavi (servizio `Code Art DB`) e passata solo a `Config::password` (mai in stringhe di connessione, log, eventi, errori; i messaggi sono ripuliti da `password=` e dalla password stessa); TLS `prefer`/`require`/`disable` con `native-tls`, `accept_invalid_certs` solo se impostato sulla connessione; timeout 10 s; con un server SSH: deve essere già `connected` («Connetti prima il server SSH»), inoltro `ssh -O forward -L 127.0.0.1:<porta libera>:<host>:<porta>` sulla connessione condivisa (solo loopback; il nome per il certificato resta l'host vero), tolto alla chiusura o alla caduta; errore `auth` (SQLSTATE 28P01/28000 o password mancante) = serve una password. MongoDB: stesse regole, `ClientOptions` costruite dai campi (host, `Credential` con `authSource` = database della connessione o `admin`, mai una URI), la password sta solo in `Credential` (il `Debug` del driver la nasconde; i messaggi sono ripuliti), TLS con `rustls` e radici `webpki-roots` (non il Portachiavi di sistema): `require` = TLS, `disable` = niente, `prefer` = TLS solo per host remoti (MongoDB non ha un TLS opportunistico), `accept_invalid_certs` solo se impostato; `ping` all'apertura entro 10 s (errori di autenticazione = `auth`); con un tunnel `127.0.0.1:<porta>` e connessione diretta | globale (sessione condivisa da tutte le finestre; evento `db:state` a tutte, senza segreti) | rete (host della connessione), processo `/usr/bin/ssh` (inoltro), Portachiavi (lettura) |
| `db_disconnect` | `id` | progetto fidato; chiude il client, ferma il task della connessione e toglie l'inoltro SSH; già chiusa = nulla | globale | processo `/usr/bin/ssh` (cancel) |
| `db_states` | — | progetto fidato; stato delle sessioni note in questo avvio | finestra | — |
| `db_pg_tree` | `id` | progetto fidato; connessione Postgres `connected`; schemi, tabelle (`r`, `p`) e viste (`v`, `m`) escludendo `pg_catalog`, `information_schema`, `pg_toast*`, `pg_temp*`; solo query con parametri (`$1`) | finestra | rete (server del database) |
| `db_pg_columns` | `id`, `schema`, `table` | progetto fidato; come `db_pg_tree`; schema e tabella passati come parametri, mai nel testo SQL | finestra | rete |
| `db_pg_preview_edits` | `id`, `schema`, `table`, `edits` | progetto fidato; connessione `connected`; la WebView manda solo nomi di colonna e valori testuali (`null` = NULL): colonne e tipi vengono dal catalogo del server, nome sconosciuto o tabella senza chiave primaria = errore; max 10 000 modifiche; restituisce testo SQL leggibile con i valori inseriti (≤ 200 caratteri ciascuno) solo per la lettura, mai eseguito | finestra | rete (legge il catalogo) |
| `db_pg_apply_edits` | `id`, `schema`, `table`, `edits` | progetto fidato; stesso piano dell'anteprima; conferma nativa del core (titolo «Applica modifiche», conteggi e prime 10 istruzioni, «Annulla» predefinito), rifiutata = `Err("annullato")`; nomi sempre citati (`"` raddoppiati), ogni valore è un parametro testuale `$n::<tipo della colonna>` (mai nel testo SQL); `BEGIN`/istruzioni/`COMMIT` in esclusiva sulla connessione (chiude un risultato aperto); con una transazione aperta dall'utente rifiuta prima della conferma («Chiudi prima la transazione aperta (COMMIT o ROLLBACK)»), così non la conclude mai; i parametri sono convertiti nel tipo senza modificatori (`format_type(atttypid, -1)`: `character varying`, `bpchar`, `numeric`) e l'assegnazione alla colonna fa rispettare lunghezza e precisione (troppo lungo = errore, mai troncato); `UPDATE`/`DELETE`/`INSERT` devono toccare esattamente una riga, altrimenti `ROLLBACK` con errore che nomina l'istruzione (valori ≤ 200 caratteri); errori del server riportati col suo messaggio | finestra | rete (scrive nel database) |
| `db_mongo_tree` | `id` | progetto fidato; connessione MongoDB `connected`; `listDatabases` + `listCollections` (senza permesso di elencare: solo il database della connessione), in ordine alfabetico | finestra | rete (server del database) |
| `db_mongo_find` | `id`, `db`, `coll`, `filter`, `sort`, `projection` | progetto fidato; connessione `connected`; nomi non vuoti e senza caratteri di controllo; filtro, ordinamento e proiezione sono testo JSON esteso (rilassato o canonico) analizzato nel core in documenti BSON con l'ordine delle chiavi conservato (mai valutato come codice; vuoto = `{}`; errore = «Filtro non valido: …»); pagine da 500 documenti con cursore tenuto dal core, al massimo 100 000; celle oltre 64 KB troncate con indicazione (JSON esteso rilassato), documento intero a parte in JSON esteso canonico (non inviato oltre 1 MB) e `_id` di ogni riga in JSON esteso canonico, testo opaco ridato dalla WebView così com'è; un solo risultato aperto per connessione; risultato scaduto dopo una riconnessione | finestra | rete (legge) |
| `db_mongo_aggregate` | `id`, `db`, `coll`, `pipeline` | progetto fidato; come `db_mongo_find`; pipeline = array JSON di stadi; se un stadio di primo livello è `$out` o `$merge`: conferma nativa del core (titolo «La pipeline scrive dati», «Annulla» predefinito), rifiutata = `Err("annullato")` senza eseguire (controllo sulle chiavi di primo livello, di aiuto) | finestra | rete (legge; con `$out`/`$merge` scrive) |
| `db_mongo_replace` | `id`, `db`, `coll`, `idJson`, `docJson` | progetto fidato; `idJson` JSON esteso (ObjectId, stringa…); `docJson` deve essere un oggetto, il suo `_id` (se c'è) deve coincidere; conferma nativa (titolo «Modifica documento», `db.coll` e `_id`, «Annulla» predefinito); `replaceOne` con filtro `{"_id": {"$eq": …}}` (un `_id` che sembra un operatore resta un valore): deve trovare esattamente un documento | finestra | rete (scrive) |
| `db_mongo_insert` | `id`, `db`, `coll`, `docJson` | progetto fidato; `docJson` oggetto; conferma nativa «Modifica documento»; restituisce l'`_id` (JSON esteso canonico) | finestra | rete (scrive) |
| `db_mongo_delete` | `id`, `db`, `coll`, `idJson` | progetto fidato; conferma nativa «Elimina documento» (`db.coll` e `_id`, «Annulla» predefinito); `deleteOne` con filtro `{"_id": {"$eq": …}}`: deve eliminare esattamente un documento | finestra | rete (scrive) |
| `db_query` | `id`, `text` | progetto fidato; connessione `connected`; testo non vuoto; controllo lessicale "di aiuto, non una garanzia" (ignora commenti, anche annidati, stringhe, `E''`, `$tag$…$tag$`, identificatori tra virgolette) di `UPDATE`/`DELETE` senza `WHERE` di primo livello, `DROP`, `TRUNCATE`: se scatta, conferma nativa del core (titolo «Istruzione distruttiva», elenco delle istruzioni, «Annulla» predefinito), rifiutata = `Err("annullato")` senza eseguire nulla; una sola SELECT/WITH/VALUES/TABLE gira in `BEGIN` + `DECLARE … NO SCROLL CURSOR` + `FETCH 500` (protocollo semplice; la transazione non è `READ ONLY`, si chiude con `COMMIT`, `ROLLBACK` solo su errore o annullamento; dopo 90 s senza pagine né esportazioni il cursore si chiude da solo con `COMMIT`); dopo un `BEGIN`/`START TRANSACTION` dell'utente il core chiede al server se la transazione è aperta (`now() = statement_timestamp()`, 25P02 = aperta e fallita) e finché lo è non usa il cursore né conclude la transazione; `editableTable` (tabella modificabile in griglia) solo per una lettura semplice `SELECT * FROM t [WHERE|ORDER BY|LIMIT|OFFSET …]` riconosciuta dal lessico del core e risolta con `to_regclass($1)` (parametro); il resto col protocollo semplice, ultimo insieme di righe limitato a 500; celle oltre 64 KB troncate con indicazione; al massimo 100 000 righe per cursore; un solo risultato aperto per connessione (una nuova query chiude il precedente) | finestra (il risultato appartiene alla generazione della sessione: dopo una riconnessione è scaduto) | rete (esegue SQL scritto dall'utente) |
| `db_query_more` | `resultId` | progetto fidato; risultato aperto della sessione attuale (id ≥ 2^62 = MongoDB: pagina di documenti con colonne di quella pagina), altrimenti «risultato scaduto, riesegui la query» | finestra | rete |
| `db_query_close` | `resultId` | progetto fidato; chiude il cursore (`COMMIT`); già chiuso o scaduto = nulla | finestra | rete |
| `db_export` | `resultId`, `format` (`csv`\|`json`) | progetto fidato; il percorso viene solo dal selettore nativo di salvataggio aperto dal core (nome suggerito `risultato.csv`/`.json`), mai dalla WebView; annullato = `Ok(None)` senza toccare il cursore; scrive tutte le righe del risultato (le consegnate, tenute in copia dal core, più le restanti, al massimo 100 000; Postgres: celle già troncate a 64 KB) in un file temporaneo nella stessa cartella e poi rinomina; CSV RFC 4180 (CRLF, NULL = vuoto, `""` = stringa vuota); consuma il cursore (`hasMore = false`; il risultato resta tenuto e si può riesportare); ogni risultato con righe (anche finito, ≤ 500 righe, anche MongoDB) ha un `resultId` e una copia delle righe nel core, rilasciata da `db_query_close`, da una nuova query della connessione o da una riconnessione; risultato scaduto = «risultato scaduto, riesegui la query» | finestra | rete (legge il resto del cursore), file (scrive nel percorso scelto) |
| `db_query_cancel` | `id` | progetto fidato (tenuto sotto fiducia per semplicità); `CancelToken` con lo stesso modo TLS e lo stesso tunnel della sessione; timeout 10 s; la query annullata termina con «Query annullata», fa `ROLLBACK` e scarta il risultato | finestra | rete (connessione di annullamento al server) |
| `pty_pick_cwd` | — | selettore nativo aperto dal core (partenza: cartella principale); la cartella scelta passa da `ensure_inside` (link risolti) e deve esistere, altrimenti "La cartella deve stare dentro il progetto"; non concede fiducia (la chiede `pty_spawn`) | finestra | — |
| `pty_spawn` | `cwd`, `cols`, `rows` | `ensure_inside` + cartella esistente; progetto fidato; 1–1000; ≤ 16 per finestra | finestra | P (`$SHELL -l`) |
| `pty_spawn_ssh` | `server_id`, `dir`, `cols`, `rows` | progetto fidato; server noto e `connected`; `dir` assoluta dopo `~` (validata); 1–1000; ≤ 16 per finestra (stesso registro); la cartella è solo l'argomento quotato `$1` di uno script costante | finestra | P (`ssh -t` sulla connessione condivisa) |
| `pty_write` | `id`, `data` | PTY della stessa finestra; ≤ 1 MB | finestra | P |
| `pty_resize` | `id`, `cols`, `rows` | PTY della stessa finestra; 1–1000 | finestra | P |
| `pty_kill` | `id` | PTY della stessa finestra | finestra | P |
| `conv_list` | `projectId` | deve coincidere con la finestra | finestra | F (legge `~/.claude`, sola lettura); titolazione solo se fidato |
| `conv_history` | `sessionId` | UUID; conversazione del progetto | finestra | F (sola lettura) |
| `conv_search` | `query` | 2–200 caratteri; tempo massimo 3 s | finestra | F (sola lettura) |
| `claude_check` | — | — | globale | P (`claude --version`) |
| `conv_start` | `projectId`, `opts` | coincide con la finestra; `model` (`valid_model`: niente spazi né `-` iniziale), `permissionMode` in whitelist; progetto fidato | finestra | P (`claude`), R |
| `conv_send` | `sessionId`, `text`, `imagePaths` | UUID; del progetto; fidato; ≤ 10 immagini, ≤ 20 MB l'una e 30 MB in tutto, solo png/jpg/gif/webp canonicalizzate dentro la cartella immagini dell'app o del progetto | finestra | P, F (lettura immagini) |
| `conv_answer` | `sessionId`, `requestId`, `choice` | del progetto; richiesta ancora aperta; scelta ammessa per il tipo (domande: testo per una domanda, oppure oggetto JSON con una risposta per ogni domanda e nessuna domanda sconosciuta) | finestra | P (stdin di `claude`) |
| `conv_interrupt` | `sessionId` | del progetto | finestra | P |
| `conv_set_model` | `sessionId`, `model` | del progetto; `valid_model` | finestra | P |
| `conv_set_permission_mode` | `sessionId`, `mode` | del progetto; whitelist | finestra | P |
| `conv_mark_viewed` | `sessionId` | del progetto | finestra | — |
| `conv_archive` | `sessionId`, `archived` | del progetto | finestra | — |
| `conv_save_image` | corpo binario, header `ext` | finestra di progetto; estensione in whitelist, byte iniziali coerenti, ≤ 20 MB; nome casuale, cartella 0700, file 0600 | finestra | F (cartella temporanea dell'app) |
| `conv_delete` | `sessionId` | UUID; del progetto; fidato; file trovato solo dalle cartelle di sessione del progetto, no link simbolici né fuori radice; conferma nativa del core | finestra | F (sposta nel Cestino il file di sessione in `~/.claude`: unica scrittura lì), P (ferma il processo) |
| `conv_pending` | `sessionId` | del progetto | finestra | — |
| `conv_rename` | `sessionId`, `title` | del progetto; lunghezza e caratteri di controllo | finestra | — |
| `conv_unlock_title` | `sessionId` | del progetto | finestra | — |
| `editor_set_dirty` | `count` | finestra di progetto; ≤ 10 000; per finestra, dimenticato alla sua distruzione | finestra | — (la chiusura e l'uscita chiedono conferma) |
| `search_start` | `query` (testo ≤ 1000 caratteri, regex, maiuscole, parola intera, ≤ 50 filtri `include`/`exclude` ≤ 200 caratteri) | progetto e cartelle dalla finestra chiamante; solo lettura, funziona anche in cartelle non fidate; nessun link simbolico seguito; salta `.gitignore`, rumore, binari e file > 2 MB; ≤ 10 000 risultati; una ricerca attiva per finestra (la nuova annulla la precedente) | finestra | — (legge file del progetto, nessun processo; eventi `search:hits`/`search:done` solo alla finestra del progetto) |
| `search_cancel` | `id` | progetto dalla finestra chiamante; annulla solo la ricerca attiva di quella finestra | finestra | — |
| `search_replace` | `query` (come `search_start`), `replacement` (≤ 5000 caratteri), `paths` (≤ 10 000) | progetto dalla finestra chiamante; **solo progetto fidato**; ogni percorso passa da `mutable_path` (confinato, link simbolici risolti); le corrispondenze si ricalcolano sul disco (i risultati della pagina non contano); conferma NATIVA del core («Annulla» predefinito, un dialogo per finestra); salta file non UTF-8, binari, > 2 MB o cambiati dopo la lettura (controllo dell'mtime nella scrittura) | finestra | — (scrive file del progetto, nessun processo) |
| `git_repos` | — | progetto dalla finestra chiamante; **solo progetto fidato** (git esegue hook e `core.fsmonitor`); cartelle del progetto con `.git`, più i repository annidati fino a 3 livelli sotto le cartelle che non lo sono (mai dentro un repository, né in cartelle nascoste, `node_modules`, `target`, `vendor`, `venv`, `dist`, `build`, né attraverso link simbolici; al massimo 50, senza doppioni; al massimo 2000 voci lette per cartella; elenco in cache 2 s) | finestra | F (lettura di cartelle; `.git` esiste) |
| `git_status` | `repo` | **solo progetto fidato**; `repo` deve coincidere (percorso canonico) con una radice di `git_repos`; ≤ 5000 voci | finestra | P (`git status`, `git rev-parse`; argomenti come lista, `GIT_OPTIONAL_LOCKS=0`, `GIT_TERMINAL_PROMPT=0`, timeout 15 s) |
| `git_diff` | `repo`, `path`, `origPath` (facoltativo, rinomine in stage), `staged` | come `git_status`; `path` e `origPath` relativi, senza `..`/`.`/radice/NUL, usati solo come `rev:percorso` (mai come opzione); il lato su disco passa da `ensure_inside` (link simbolici risolti); file > 2 MB o binari non restituiti | finestra | P (`git show`, timeout 15 s) |
| `git_branches` | `repo` | come `git_status` | finestra | P (`git for-each-ref`) |
| `git_log` | `repo`, `limit` | come `git_status`; `limit` limitato a 1–200 | finestra | P (`git log`) |
| `git_stage` | `repo`, `paths` | **solo progetto fidato**; `repo` coincide (percorso canonico) con una radice di `git_repos`; una sola scrittura alla volta per repository ("Un'altra operazione git è in corso"); `paths` (≤ 10 000) relativi, senza `..`/`.`/radice/NUL, mai con `-` iniziale, cartella contenitore dentro il repository (link simbolici risolti); passati da stdin come pathspec letterali (`--pathspec-from-file`, `GIT_LITERAL_PATHSPECS=1`) | finestra | P (`git add -A`; senza `GIT_OPTIONAL_LOCKS`, timeout 120 s) |
| `git_unstage` | `repo`, `paths` | come `git_stage` | finestra | P (`git restore --staged`, oppure `git rm --cached` se non c'è ancora un commit) |
| `git_discard` | `repo`, `paths` | come `git_stage`; **conferma nativa** («Annulla» predefinito) "Scartare le modifiche a N file? Non si possono recuperare."; i file non tracciati vanno nel **Cestino** (mai cancellati); una sola conferma aperta per finestra | finestra | P + F (`git ls-files`, `git restore --worktree`; Cestino di macOS) |
| `git_commit` | `repo`, `message`, `amend` | **solo progetto fidato**; `repo` coincide (percorso canonico) con una radice di `git_repos`; una sola scrittura alla volta per repository ("Un'altra operazione git è in corso"); messaggio non vuoto, ≤ 64 KB, senza NUL, passato via stdin (`commit -F -`); gli hook del repository girano (progetto fidato) con l'ambiente dell'utente (niente `GIT_LITERAL_PATHSPECS` né `LC_ALL=C`) | finestra | P (`git commit`, timeout 120 s, poi SIGTERM e SIGKILL dopo 2 s) |
| `git_checkout` | `repo`, `branch` | **solo progetto fidato**; `repo` coincide (percorso canonico) con una radice di `git_repos`; una sola scrittura alla volta per repository ("Un'altra operazione git è in corso"); nomi di branch mai con `-` iniziale né `@{`, validati con `git check-ref-format --branch` (output identico); branch locale (`switch --no-guess`) o remoto (`switch --track`) esistente | finestra | P (`git switch`) |
| `git_create_branch` | `repo`, `name`, `from` (facoltativo), `checkout` | come `git_checkout` per `name` e `from` | finestra | P (`git branch` / `git switch -c`) |
| `git_delete_branch` | `repo`, `name` | come `git_checkout`; mai il branch corrente; **conferma nativa** («Annulla» predefinito, una per finestra); `-d`, mai `-D` | finestra | P (`git branch -d`) |
| `git_merge` | `repo`, `branch` | come `git_checkout`; `--no-edit`, nessun editor (`GIT_EDITOR=true`) | finestra | P (`git merge`) |
| `git_merge_abort` | `repo` | **solo progetto fidato**; `repo` coincide (percorso canonico) con una radice di `git_repos`; una sola scrittura alla volta per repository ("Un'altra operazione git è in corso") | finestra | P (`git merge --abort`) |
| `git_init` | `folder` | **solo progetto fidato**; `folder` coincide (percorso canonico) con una cartella del progetto che non è già un repository | finestra | P + F (`git init`, crea `.git`) |
| `git_remote_op` | `repo`, `op` (`fetch` \| `pull` \| `push`) | **solo progetto fidato**; `repo` tra le radici di `git_repos`; occupa il repository fino alla fine (una sola scrittura alla volta); `pull` secondo `git_pull_mode` (`--ff-only` predefinito, `--rebase`, `--no-rebase`); `push` imposta l'upstream se manca (`-u origin <branch corrente>`); mai `--force`; eventi `git:progress`/`git:op_done` solo alla finestra del progetto | finestra | P + R (`git fetch/pull/push --progress` in una sessione propria; `GIT_TERMINAL_PROMPT=0`, `GIT_SSH_COMMAND="ssh -o BatchMode=yes"` se l'utente non ha già un comando ssh (`GIT_SSH_COMMAND`, `GIT_SSH`, `core.sshCommand`); helper di credenziali dell'utente attivi; timeout 10 min, poi SIGTERM e SIGKILL dopo 2 s) |
| `git_cancel` | `id` | annulla solo un'operazione del progetto della finestra chiamante (SIGTERM al gruppo di processi, così git toglie i suoi lock; SIGKILL ai superstiti dopo 2 s); la chiusura della finestra annulla le sue operazioni | finestra | — |
| `gh_accounts` | — | anche dalla finestra di benvenuto (non esegue codice di un progetto); JSON su campi espliciti (`--json hosts`), testo come ripiego | globale | P (`gh auth status`, cartella `/`, timeout 60 s) |
| `gh_repo_list` | `account`, `limit` | `account` vuoto (attivo) o login valido (alfanumerici e `-`, ≤ 39, mai `-` iniziale) collegato a `gh`; `limit` 1–200; anche dal benvenuto | globale | P + R (`gh repo list --json nameWithOwner,description,isPrivate,updatedAt`) |
| `gh_clone` | `nameWithOwner`, `account` (facoltativo) | `proprietario/nome` validato; destinazione = cartella scelta col selettore nativo del core (foglio della finestra) + nome del repository, che non deve esistere; la cartella clonata diventa fidata (scelta dall'utente; il repository, anche pubblico e qualunque, lo indica la pagina e lo nomina il titolo del selettore) e si apre in una nuova finestra; `false` = selettore annullato | globale | P + R + F (`gh repo clone`, timeout 10 min) |
| `gh_remote` | `repo` | **solo progetto fidato**; `repo` tra le radici di `git_repos`; restituisce se `origin` esiste e il suo `proprietario/nome` se è su github.com | finestra | P (`git remote get-url origin`) |
| `gh_repo_create` | `repo`, `name`, `private`, `description` | come `gh_remote`; `name` (`nome` o `proprietario/nome`: lettere, cifre, `.`, `_`, `-`, mai `-` iniziale); descrizione su una riga ≤ 350 caratteri passata come `--description=`; una sola scrittura alla volta per repository | finestra | P + R (`gh repo create <nome> --source <repo> --remote origin --push --private\|--public`, timeout 10 min) |
| `gh_pr_list` | `repo`, `state` | come `gh_remote`; `state` `open` \| `closed` \| `all`; ≤ 100 voci | finestra | P + R (`gh pr list --json …`) |
| `gh_pr_checks` | `repo`, `number` | come `gh_remote`; numero > 0 | finestra | P + R (`gh pr checks --json name,state,link,bucket`) |
| `gh_pr_create` | `repo`, `title`, `body`, `base`, `draft` | come `gh_remote`; titolo su una riga ≤ 256 caratteri (`--title=`), corpo ≤ 64 KB da stdin (`--body-file -`), `base` vuota (predefinito) o nome senza spazi né `-` iniziale (`--base=`); `--head=` dal branch corrente (HEAD staccato rifiutato); push con upstream prima, se manca, sullo stesso percorso di `git_remote_op` (mai `--force`); una sola scrittura alla volta per repository | finestra | P + R (`git push`, `gh pr create`, `gh pr view`) |
| `gh_pr_merge` | `repo`, `number`, `method`, `deleteBranch` | come `gh_remote`; `method` `merge` \| `squash` \| `rebase` (validato prima di chiedere); **conferma nativa** («Annulla» predefinito, una per finestra); una sola scrittura alla volta per repository | finestra | P + R (`gh pr merge --merge\|--squash\|--rebase [--delete-branch]`) |
| `lsp_start` | `lang` (`python` \| `java` \| `go` \| `rust`) | progetto dalla finestra chiamante; **solo progetto fidato** (i language server eseguono codice del progetto); binario dalla tabella `lsp/servers.rs`, cercato nell'ambiente di login (mai un percorso dalla pagina); un server per progetto e linguaggio, riusato; cartella di lavoro = cartella principale del progetto; riavvio automatico al massimo 3 volte in 5 minuti, poi fermo fino alla riapertura; tutti fermati alla chiusura della finestra e all'uscita dell'app (che aspetta anche quelli in arresto per la chiusura delle finestre) | finestra | P (`pyright-langserver --stdio` o `pylsp`, `jdtls -data <Application Support>/CodeArt/jdtls/<progetto>` (crea la cartella), `gopls`, `rust-analyzer`; sessione propria, ambiente di login; stderr scartato salvo le ultime 4 KB per l'errore di avvio; eventi `lsp:message`/`lsp:exit` solo alla finestra del progetto) |
| `lsp_send` | `id`, `message` | **solo progetto fidato**; server del progetto della finestra chiamante; oggetto JSON-RPC 2.0 (richiesta, notifica o risposta) ≤ 64 MB; ogni URI `file:` (valori e chiavi, a qualunque profondità; riconosciuto come fa il parser URL WHATWG: spazi e controlli iniziali ignorati, tab e a capo tolti, schema senza maiuscole; poi decodificato) e ogni `rootPath` deve stare dentro le cartelle del progetto (`ensure_inside`: niente `..`, link simbolici risolti, host diverso da `localhost` rifiutato); non si controlla solo il contenuto dei documenti (`textDocument.text`, `contentChanges[].text`); difesa in più, non un confine (vedi "Rischi residui"); metodi qualsiasi, `workspace/executeCommand` compreso | finestra | P (stdin del server, cornice `Content-Length`; in lettura header ≤ 8 KB e contenuto ≤ 64 MB, altrimenti il server viene chiuso) |
| `lsp_stop` | `id` | solo un server del progetto della finestra chiamante; SIGTERM al gruppo di processi, SIGKILL ai superstiti dopo 2 s | finestra | P |
| `project_setting_get` | `key` | progetto dalla finestra chiamante; whitelist (`github_account`) | finestra | — |
| `project_setting_set` | `key`, `value` | progetto dalla finestra chiamante; whitelist + validatore (`github_account`: vuoto o login valido); ammesso anche in un progetto non fidato (salva solo la scelta, nessun processo: vale quando il progetto diventa fidato e sceglie solo tra account già collegati a `gh`) | finestra | — |
| `tools_check` | — | — | globale | P (cerca `git`, `gh` e i language server nell'ambiente di login; non li esegue) |
| `trust_check` | — | — | finestra | — |
| `trust_accept` | `fromClick` | conferma nativa mostrata dal core; dopo un rifiuto niente domanda automatica, banner ignorato per 10 s | finestra | — |

## CSP

```
default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:;
media-src 'self' blob:; connect-src ipc: http://ipc.localhost; worker-src 'self' blob:; object-src 'none';
base-uri 'none'; form-action 'none'
```

Rilassamenti e perché:
- `style-src 'unsafe-inline'`: Monaco e xterm creano stili in linea ed elementi `<style>`; anche le
  transizioni di Svelte. Gli script restano solo `'self'` (Tauri aggiunge da sé gli hash dei propri script).
- `img-src data: blob:`: le icone di Monaco sono `data:` nel suo CSS; le anteprime delle immagini incollate
  nel compositore sono URL `blob:`. Nessuna origine remota: la conversazione non carica immagini, l'anteprima di un file solo quelle locali del progetto.
- `media-src 'self' blob:`: l'anteprima di audio e video usa `<audio>`/`<video>` con un URL `blob:` creato dai
  byte letti con `fs_read_bytes` (confinati nel progetto, tipo MIME da una tabella fissa per estensione).
  Nessuna origine remota: `blob:` può contenere solo ciò che la pagina stessa ha costruito.
- `worker-src 'self' blob:`: i worker di Monaco (`?worker` di Vite, file in `assets/`); `blob:` per il
  meccanismo di riserva di Monaco.
- `connect-src ipc: http://ipc.localhost`: solo il canale IPC di Tauri.
- `devCsp` (solo `pnpm tauri dev`): in più `ws://localhost:1420` per l'HMR di Vite.
- Nessun `unsafe-eval` (verificato: il bundle non usa `eval`/`new Function`), nessuna origine remota,
  `withGlobalTauri` spento, `dangerousDisableAssetCspModification: false`, asset protocol non abilitato.

## Permessi Tauri (capabilities)

`src-tauri/capabilities/project.json` (finestre `proj-*`):
- `core:event:allow-listen`, `core:event:allow-unlisten` — eventi del core (`conv:*` compreso `conv:removed`, `fs:changed`, `git:changed`, `git:progress`, `git:op_done`, `search:hits`, `search:done`, `lsp:message`, `lsp:exit`, `pty:*`).
  Niente `allow-emit`: la pagina non può inviare eventi alle altre finestre.

**Come arrivano gli eventi di un progetto.** In Tauri 2 `emit_to(etichetta)` filtra solo gli ascoltatori
registrati *per quella finestra*: un ascoltatore globale (`listen` senza finestra) di qualunque finestra li riceve
tutti. Per questo:
- ogni evento di progetto porta l'id del progetto nel **nome**: `conv:event:<id>`, `conv:upsert:<id>`,
  `conv:removed:<id>`, `conv:focus:<id>`, `fs:changed:<id>`, `git:changed:<id>`, `git:progress:<id>`, `git:op_done:<id>`, `search:hits:<id>`, `search:done:<id>`, `lsp:message:<id>`, `lsp:exit:<id>`, `pty:data:<id>`, `pty:exit:<id>`. Un solo helper per lato
  costruisce il nome (`projects::project_event` / `emit_to_project` nel core, `src/lib/events.ts` nell'interfaccia,
  che lo ricava dal proprio progetto); il core lo emette con `emit_to` verso la sola finestra del progetto;
- l'interfaccia ascolta solo con `getCurrentWebviewWindow().listen` (finestra corrente), mai con il `listen`
  globale (`src/lib/events.test.ts` fallisce se un sorgente lo importa).
- Test: `project_events_carry_the_project_id_in_the_name` (e nessun `emit_to` con nome fisso fuori dall'helper),
  `listenProject` in `src/lib/events.test.ts`.
- Rischio residuo: una pagina compromessa che conosce l'id (UUID) di un altro progetto può registrare un ascoltatore
  globale su quel nome e leggerne gli eventi (output del terminale, testo delle conversazioni). Le altre finestre non
  ricevono mai l'id di un progetto diverso dal proprio; la finestra di benvenuto non ha il permesso di ascolto.
- `core:window:allow-start-dragging`, `core:window:allow-internal-toggle-maximize` — barra del titolo
  (`data-tauri-drag-region`, doppio clic).
- `opener:allow-open-url` limitato a `http://*` e `https://*` — link esterni nel browser di sistema.

`src-tauri/capabilities/welcome.json` (finestra `welcome`): solo i due permessi della barra del titolo.

Tolti rispetto a prima: `core:default` (comprendeva menu, emit, webview, app…), `dialog:default` (ora i
dialoghi li apre solo il core). Nessun permesso `shell:` o `fs:`. I comandi dell'app non sono filtrati
per finestra dall'ACL di Tauri: il confine è il controllo dell'etichetta nel core (tabella sopra).

## Processi

Tutti avviati con lista di argomenti, mai tramite shell con testo composto:
- `claude` (`engine.rs`): argomenti costanti + `--permission-mode` (whitelist), `--model` (`valid_model`),
  `--resume`/`--session-id` (UUID validato), `--add-dir` (cartelle del progetto dallo store, fidate);
  `current_dir` = cwd della conversazione, che la sincronizzazione accetta solo dentro le cartelle del progetto.
  Ambiente: `env_clear()` + ambiente della shell di login.
- `claude` per i titoli (`titles.rs`): argomenti costanti, cartella temporanea vuota, prompt su stdin;
  mai per progetti non fidati.
- `claude --version` (`locate.rs`): costante. L'eseguibile si cerca nel `PATH` della shell di login, poi in
  `/opt/homebrew/bin`, `/usr/local/bin`, `~/.local/bin`, `~/.claude/local` (solo file eseguibili).
- Ambiente della shell di login (`locate.rs`): `$SHELL -l -i -c env`, comando costante; `$SHELL` solo se assoluto.
- Terminale (`pty.rs`): `$SHELL -l` (solo se percorso assoluto esistente, altrimenti `/bin/zsh`), cwd confinato.
- `gh` (`github.rs`): cercato come gli altri strumenti (`PATH` di login, poi Homebrew); se manca, "GitHub CLI
  (`gh`) non trovata: `brew install gh`". Sempre con `run_tool` (sessione propria, timeout, SIGTERM e poi SIGKILL,
  output limitato), `GH_PROMPT_DISABLED=1`, `GIT_TERMINAL_PROMPT=0`; i valori della pagina passano in forma
  `--opzione=valore` o dopo validazione (nomi di repository, login, numeri); i JSON si leggono con `serde` su campi
  espliciti (`--json` con elenco di campi). I comandi su un repository girano solo in progetti fidati (`gh` esegue
  `git` nel repository). Mai `gh auth switch`.
- `git` (`git/cmd.rs`, `git/remote.rs`): cercato come `gh`, solo in progetti fidati, `current_dir` = repository
  del progetto (canonicalizzato). Sempre `GIT_TERMINAL_PROMPT=0`, `GIT_EDITOR=true`, `GIT_MERGE_AUTOEDIT=no`,
  `-c core.quotepath=off -c color.ui=never`. Letture: in più `GIT_OPTIONAL_LOCKS=0` (non toccano l'indice) e
  `LC_ALL=C`. Scritture: `LC_ALL=C` dove l'output si analizza, `GIT_LITERAL_PATHSPECS=1` solo per i percorsi della
  pagina passati su stdin (`--pathspec-from-file=- --pathspec-file-nul`), ambiente dell'utente per i comandi con
  hook (`commit`, `switch`). Fetch/pull/push: `--progress`, ssh in `BatchMode` se l'utente non ha un proprio comando
  ssh, mai `--force`. Tutti in una sessione propria con timeout (15 s letture, 120 s scritture, 10 min remoti);
  annullamento e timeout fermano il gruppo con SIGTERM e poi SIGKILL.
- Language server (`lsp/`): solo i binari della tabella `lsp/servers.rs` (pyright/pylsp, jdtls, gopls,
  rust-analyzer) con argomenti costanti (`-data` per jdtls in `~/Library/Application Support/CodeArt/jdtls/<id>`, fuori dal progetto), cercati
  come gli altri strumenti, `current_dir` = cartella del progetto, ambiente di login, sessione propria; solo in
  progetti fidati. Fermati alla chiusura della finestra e all'uscita dall'app.
- Uscita dall'app: `RunEvent::Exit` ferma i processi `claude`, i language server, le operazioni remote git
  (`RemoteOps::stop_all`, aspetta i thread) e i processi avviati da `run_tool` (scritture git, `gh`, push di
  `gh_pr_create`: registro in `tools.rs`), sempre SIGTERM, attesa di 2 s, poi SIGKILL: git toglie i suoi lock invece
  di morire di SIGPIPE da orfano. La chiusura di una finestra annulla le sue operazioni remote.

## Segreti e log

- L'ambiente della shell di login (contiene i segreti dell'utente) vive solo in `Launch.env` in memoria:
  il tipo non è serializzabile, non viene loggato né scritto su disco, e passa solo ai figli via `envs()`.
  `claude_check` restituisce solo `found`, `version`, `path`.
- Log: tre `eprintln!`: progetto non riapribile (nome e errore) e due misure di avvio attive solo con
  `CODEART_TIMING=1` (millisecondi ed etichetta della finestra). Nessun contenuto di messaggi, file o variabili
  d'ambiente nei log.
- SQLite (`codeart.db`): progetti (nomi, percorsi), impostazioni, metadati delle conversazioni (titolo,
  riassunto, stato), cartelle fidate. Nessuna credenziale. Schema versionato con `PRAGMA user_version`
  (migrazioni in `store.rs`).
- Account GitHub: per un account diverso da quello attivo di `gh` (impostazione di progetto `github_account`)
  il token di `gh auth token --user <login>` va solo nell'ambiente del processo figlio (`GH_TOKEN`) e quindi dei
  suoi discendenti (il `git` di `gh repo create --push`/`gh repo clone` e gli hook del repository fidato, che
  potrebbero comunque chiedere il token a `gh`), mai nella pagina, nello store, nei log o negli eventi. Limite: il push
  dell'upstream in `gh_pr_create` usa le credenziali git dell'utente (helper, portachiavi), non quell'account; il tipo `Token` ha un `Debug` che lo nasconde. Ogni messaggio
  d'errore di `gh` è ripulito (`Gh::clean`): token in uso, qualunque parola con prefisso di token GitHub
  (`ghp_`, `gho_`, `github_pat_`…), `GH_TOKEN`/`GITHUB_TOKEN` dell'ambiente di login, errori di serde e percorsi della home (`/Users/<nome>` → `~`). Test:
  `token_never_in_debug_or_errors`, `clean_hides_tokens_and_home_paths`.
- Le immagini incollate restano in `$TMPDIR/CodeArt/images` (0700/0600), cancellate dopo 24 ore.
- Database: la cronologia delle query (SQLite, tabella `db_history`, ultime 200 per connessione) e il testo delle schede
  query (`localStorage` della pagina, chiavi `codeart.dbquery.<connessione>.*`) sono salvati **in chiaro**: un testo come
  `ALTER ROLE … PASSWORD '…'` resta lì finché non esce dalla cronologia o non si elimina la connessione. Eliminare una
  connessione cancella la sua cronologia e le sue chiavi in `localStorage`.
- Password SSH (`ssh/keychain.rs`): solo nel Portachiavi di macOS, elementi "password generica" del servizio
  `Code Art SSH` con account = id del server (FFI a mano verso `SecItem*`, tutto l'`unsafe` nel modulo `sys`). La
  password arriva al core una volta (`ssh_password_set`) e non torna mai alla pagina: l'elenco dice solo se c'è
  (`hasPassword`, controllo d'esistenza che non legge il segreto). Gli errori riportano operazione e codice
  `OSStatus`, mai il segreto. La legge solo l'app stessa rilanciata da `ssh` come `SSH_ASKPASS` (`ssh/askpass.rs`):
  con `CODEART_ASKPASS_ACCOUNT` nell'ambiente `main()` non avvia Tauri, risponde solo a prompt di password che
  nominano l'host atteso (`utente@host's password:` o, keyboard-interactive, `(utente@host) …password:`, con `host`
  uguale a `CODEART_ASKPASS_HOST`, impostato da `ssh_connect` con `HostKeyAlias` o il nome reale da `ssh -G`; mai un
  `Password:` senza host, mai il prompt di un salto ProxyJump, mai passphrase né domande sì/no) stampando la password
  su stdout ed esce 0, altrimenti esce 1 senza stampare nulla; prima di leggere il Portachiavi verifica anche che il
  processo padre sia `/usr/bin/ssh` (`getppid` + `proc_pidpath`). Test: `answers_only_password_prompts_for_the_expected_host`,
  `only_system_ssh_may_ask`, `password_rules_never_echo_the_secret`, `keychain_round_trip` (ignorato: richiede il
  Portachiavi di login sbloccato).
- Connessioni SSH (`ssh/conn.rs`): sempre `/usr/bin/ssh`, `/usr/bin/ssh-keygen`, `/usr/bin/ssh-keyscan` (mai dal
  `PATH`) con lista di argomenti; un master per server (`ControlMaster`, `-N`, `ControlPersist=600`: si chiude da solo
  dopo 10 minuti senza client anche se l'app termina male) con socket in `<dati app>/s/<16 hex>` (cartella 0700; se il
  percorso è troppo lungo per un socket Unix, `$TMPDIR/codeart-<uid>/`, poi `/tmp/codeart-<uid>/`, cartella 0700 di
  proprietà dell'utente, mai un link simbolico). `StrictHostKeyChecking=yes` con
  `UserKnownHostsFile="~/.ssh/known_hosts" "<dati app>/known_hosts"`. Le operazioni sono client del master
  (`ControlMaster=no`, `-T`, `RequestTTY=no`, `ClearAllForwardings=yes`, `RemoteCommand=none` contro un
  `RemoteCommand` della config) e girano solo se lo stato è `connected`; hanno anche
  `BatchMode=yes` e `PreferredAuthentications=none`: se il master è sparito OpenSSH ripiegherebbe su una connessione
  diretta, che così non può autenticarsi (un client del master non si autentica mai). Un client del master non
  controlla la destinazione: salvare o eliminare un server connesso lo disconnette prima (`ssh -O exit`; i terminali
  remoti terminano da soli), e ogni operazione (`run`, trasferimenti, terminale) confronta destinazione, host, utente,
  porta e chiave del server con quelli della connessione: se sono cambiati (anche in `~/.ssh/config`) la connessione si
  chiude («Server modificato: riconnetti») e l'operazione è rifiutata. Test: `a_server_changed_after_connect_is_disconnected_and_refused`,
  `saving_or_deleting_an_existing_server_disconnects_it_first`. Un'uscita 255 conta come
  connessione persa solo con un messaggio di ssh riconosciuto e il master che non risponde a `-O check`; il controllo
  periodico dichiara `disconnected` dopo due `-O check` falliti di fila. All'interfaccia arrivano solo messaggi
  italiani fissi, mai lo stderr grezzo di ssh (banner del server compresi). Socket con 16 cifre esadecimali (FNV-1a a
  64 bit dell'id); due server connessi con lo stesso socket: il secondo è rifiutato. Host e alias: solo lettere e cifre
  ASCII, `.`, `-`, `_`, `:` (`validate_host`, anche per gli alias della config e per `hostname`/`hostkeyalias` di
  `ssh -G`); il nome scritto in known_hosts è `host` o `[host]:porta` con quei caratteri (niente jolly, liste,
  marcatori); al massimo 8 chiavi e 16 KB da `ssh-keyscan`, oltre la connessione è rifiutata; sul server l'unica
  shell è `sh -c '<script costante>' _ <argomenti>` con ogni argomento passato da `quote_sh`; gli script sono su una
  riga e senza `!`, così passano anche da una shell di login csh/tcsh (test `every_remote_script_is_one_line_without_bang`,
  `csh_login_shells_run_every_script`). Il file temporaneo del salvataggio remoto si crea solo se non esiste (né come
  link) e in modo esclusivo (`set -C`). I download accettano dal server solo un albero coerente (primo record = la
  sorgente, ogni voce dentro una cartella già annunciata) e non scrivono mai attraverso un collegamento simbolico
  locale (`symlink_metadata` della destinazione e delle cartelle create): test
  `hostile_walker_output_never_writes_outside_the_destination`, `write_never_follows_a_planted_temp_file`. All'uscita dall'app `ssh -O exit` per ogni connessione, poi il registro dei processi.
  Test: `quote_sh_survives_hostile_names` (eseguito con `/bin/sh`), `option_values_with_spaces_quotes_and_percent_reach_openssh_intact`,
  `classifies_real_openssh_errors`, `keyscan_lines_are_filtered_and_renamed`, `known_hosts_lookup_append_and_hashed_entries`.

## Dipendenze (2026-11-11)

- `pnpm audit --prod`: 5 avvisi (1 critico, 1 alto, 3 bassi) su `dompurify` 3.4.15 portato da
  `monaco-editor`. Risolti con l'override `monaco-editor>dompurify: ^3.4.17` in `package.json`.
  Esito dopo: nessuna vulnerabilità nota. Rimosso `@tauri-apps/plugin-dialog` (non più usato dalla pagina).
- `cargo audit` (cargo-audit 0.23, 1453 avvisi nel database): nessuna vulnerabilità. 5 avvisi informativi
  (non mantenuti: `proc-macro-error`, `zbus_names`, `zvariant`, `zvariant_derive`; unsound: `glib` 0.18):
  tutte dipendenze transitive di Tauri/wry per Linux (GTK, D-Bus), non compilate su macOS. Nessuna azione.

## Dipendenze (2026-11-16, tappa 2)

- `cargo audit` (cargo-audit 0.23, 1509 avvisi, 520 crate): nessuna vulnerabilità; gli stessi 5 avvisi
  informativi della tappa 1 (`proc-macro-error`, `zbus_names`, `zvariant`, `zvariant_derive` non mantenuti;
  `glib` 0.18 unsound), tutti transitivi di Tauri/wry per Linux e non compilati su macOS.
- `pnpm audit --prod`: nessuna vulnerabilità nota. `pnpm audit` (con le dipendenze di sviluppo): 1 avviso di
  gravità bassa, `deepmerge` 4.3.1 (prototype poisoning in `mergeObject()`, GHSA-gcqx-w48m-w2h4), portato da
  `@sveltejs/vite-plugin-svelte`. Non correggibile: non esiste una versione corretta e `deepmerge` non finisce nel
  pacchetto dell'app (solo strumenti di build, su dati nostri). Rischio residuo accettato; da ricontrollare agli
  aggiornamenti di `vite-plugin-svelte`.

## Repository pubblico

Scansione dei file tracciati: nessun percorso personale (solo `/Users/dev` e `/Users/a` nei test), nessuna
email oltre all'attribuzione dei commit, nessun token (`sk-`, `ghp_`, chiavi). `.gitignore` copre anche
`.env*`, database locali (`*.db`, `*.sqlite`), chiavi (`*.pem`, `*.key`) e `.claude/settings.local.json`.

## Rischi residui

- **Avvisi di audit non risolvibili** (vedi "Dipendenze (2026-11-16, tappa 2)"): `deepmerge` (bassa, solo
  strumenti di build) e 5 avvisi informativi su dipendenze Linux di Tauri.
- **`git` e `gh` in una cartella fidata eseguono gli hook e la configurazione del repository** (`core.fsmonitor`,
  hook di `.git/hooks`, alias): è per questo che tutti i comandi git e GitHub rifiutano i progetti non fidati.
- **Fidarsi di una cartella significa fidarsi della pagina in quella finestra.** In una finestra di progetto
  fidato una WebView compromessa può avviare un terminale (`pty_spawn` + `pty_write`) ed eseguire qualunque
  comando come l'utente, ovunque, leggendone l'output da `pty:data`; lo stesso tramite `claude` in bypass
  permissions (`conv_start`/`conv_send`). Una finestra fidata compromessa equivale alla compromissione
  dell'account. Il confine reale è: **le cartelle non fidate e la finestra di benvenuto non possono eseguire
  codice né modificare il disco**. Per le finestre fidate il peso lo reggono le misure che impediscono la
  compromissione della WebView: nessun contenuto remoto, CSP restrittiva, markdown sanificato, tutto il resto
  reso come testo.
- **`lsp_send` in una finestra fidata equivale a eseguire comandi.** La configurazione dei language server, che la
  pagina manda con `initialize.initializationOptions`, `workspace/didChangeConfiguration` o le risposte a
  `workspace/configuration`, avvia comandi senza passare da alcun URI `file:` (per esempio `check.overrideCommand` di
  rust-analyzer, `env` di gopls, `python.pythonPath` di pyright). Il controllo degli URI `file:` di `lsp_send` è
  una difesa in più contro client sbagliati, non un confine; la configurazione non viene filtrata.
- La CLI `claude` in bypass permissions, in una cartella fidata, può fare tutto ciò che può fare l'utente:
  è la scelta voluta, resa visibile (modalità in ambra, domanda di fiducia).
- Fiducia per percorso, non per inode (vedi "Misure attive").
- Eventi di progetto: una pagina compromessa che conosce l'UUID di un altro progetto può ascoltarne gli eventi
  (vedi "Come arrivano gli eventi di un progetto").
- La cartella temporanea delle immagini è condivisa tra progetti (nomi casuali non indovinabili).
- **L'eseguibile dell'app consegna le password SSH a chi lo avvia in modalità askpass da `/usr/bin/ssh`.** Il
  controllo del padre (`/usr/bin/ssh`) e dell'host nel prompt non prova con quale server parli davvero quel `ssh`: un
  processo dell'utente può avviare `/usr/bin/ssh` verso un proprio server con `SSH_ASKPASS=<app>`,
  `CODEART_ASKPASS_ACCOUNT=<id>`, `CODEART_ASKPASS_HOST=<host>` e `HostKeyAlias=<host>` (o un `HostName` falso in un
  proprio `-F`), e ricevere la password senza la richiesta del Portachiavi che avrebbe leggendola direttamente
  (l'elemento è dell'app). Contro un processo dello stesso utente non c'è un confine vero (può anche leggere la
  memoria dell'app o sostituirne le scelte); il modello di minaccia resta la pagina, non i processi locali. Il
  controllo dell'host chiude invece il caso reale di un salto ProxyJump che chiede la propria password.
