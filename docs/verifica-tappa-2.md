# Verifica della tappa 2

Misure fatte da riga di comando su macOS (Apple Silicon), build release firmata ad-hoc, il 2026-11-16.

## Pacchetto

- Comando: `pnpm tauri build`. Output in `src-tauri/target/release/bundle/` (non versionato):
  `macos/Code Art.app` e `dmg/Code Art_0.1.0_aarch64.dmg`.

## Numeri

| Obiettivo | Misura | Esito |
|---|---|---|
| `Code Art.app` < 40 MB | 15 MB (`du -sh`; 15,49 MiB secondo Tauri); eseguibile 15,8 MB; dmg 9,2 MiB (9 655 246 byte) | OK (tappa 1: app 11 MB, dmg 7,3 MB) |
| Avvio a freddo → finestra ≤ 1 s | `CODEART_TIMING=1`: finestra creata in 429 ms al primo lancio (a freddo), 207 e 219 ms nei successivi; pagina caricata (benvenuto) in 613, 264 e 274 ms | OK |
| RAM app + WebKit < 250 MB con un progetto aperto | finestra di benvenuto: app 31 MB di `phys_footprint` (RSS 107 MB); i processi WebKit non si isolano in modo affidabile da riga di comando con altre app aperte. Con un progetto aperto: da misurare a vista | da misurare a vista |
| Ricerca nel progetto < 2 s per una parola comune | vedi sotto: 5–7 ms (release, tre giri) | OK |

Metodi:
- Avvio: lancio del binario dentro l'app con `CODEART_TIMING=1` (stampa `[timing] finestra creata in N ms` e
  `[timing] pagina caricata in N ms`), tre lanci in fila, app chiusa subito dopo.
- Memoria: `footprint <pid>` e `ps -o rss` dell'app dopo 4 s; nessun progetto aperto (serve il selettore nativo).
- Ricerca: test ignorato `search_speed_on_this_repo` in `src-tauri/tests/security.rs`, che chiama il nucleo
  (`search::search`) in build release:
  `cargo test --release --test security search_speed -- --ignored --nocapture` (con `SEARCH_WORD=<parola>` per
  cambiare la parola).

### Ricerca su un repository grande

La ricerca rispetta `.gitignore` e salta `node_modules`, `target` e simili (voluto: sono rumore), quindi sulla
radice di questo repository esamina 262 file anche se `node_modules` pesa 380 MB e ha 9 499 file.

| Radice | Parola | File | Risultati | Tempo (3 giri) |
|---|---|---|---|---|
| questo repository | `the` | 262 | 920 | 14, 5,5, 6,7 ms |
| questo repository | `zzqqxx` (nessuna corrispondenza: lettura di tutti i file) | 262 | 1 (il test stesso) | 7,3, 6,2, 6,3 ms |
| `node_modules` come radice | `the` | 233–406 (si ferma al tetto di 10 000 risultati) | 10 000, troncato | 27, 6,8, 8,3 ms |
| `node_modules` come radice | `zzqqxx` | 980 (i link simbolici di pnpm non si seguono) | 0 | 18,7, 12,2, 13,5 ms |

Tutto molto sotto i 2 s. Il caso più pesante realistico (albero con decine di migliaia di file regolari) non era
disponibile in questo repository: da riprovare a vista su un monorepo vero.

## Audit delle dipendenze

- `cargo audit`: nessuna vulnerabilità (520 crate, 1509 avvisi); 5 avvisi informativi di dipendenze Linux di Tauri.
- `pnpm audit --prod`: nessuna vulnerabilità. `pnpm audit`: 1 avviso basso, `deepmerge` (solo strumenti di build,
  nessuna versione corretta). Dettagli in `docs/sicurezza.md` ("Dipendenze (2026-11-16, tappa 2)").

## Sicurezza trasversale

Nuovi test (vedi `docs/sicurezza.md`, "Verifica trasversale della tappa 2"): ogni comando git, GitHub (su un
repository), `lsp_*` e `search_replace` rifiuta un progetto non fidato; nessun sorgente della tappa 2 avvia una
shell; `GH_TOKEN` compare solo in `github.rs`.

## Avvio

L'app compilata si avvia e si chiude senza panic né errori su stdout/stderr (solo le righe `[timing]`). Gli errori
della WebView non sono leggibili da riga di comando.

## Suite

`cargo test`: 349 test del crate + 11 di sicurezza (più 1 di misura ignorato); `pnpm vitest run`: 613 test in 53 file, verdi;
`pnpm typecheck`: 0 errori, 0 warning.

## Da verificare a vista

- [ ] Scorciatoie ⌘⇧E (file), ⌘⇧F (ricerca) e ⌘⇧G (Git) aprono la vista giusta nel pannello laterale
- [ ] Ricerca nel progetto: risultati raggruppati per file, filtri, maiuscole/regex/parola intera, clic che apre il file alla riga
- [ ] Sostituzione: dialogo nativo di conferma («Annulla» predefinito) con il numero di occorrenze e di file; dopo l'esito i file aperti si aggiornano
- [ ] Vista Git: stato con file modificati, in stage, non tracciati
- [ ] Stage e unstage di un file e di più file
- [ ] Scarta modifiche: conferma nativa; un file non tracciato finisce nel Cestino, non viene cancellato
- [ ] Commit con ⌘↵ (anche con messaggio multiriga e "modifica ultimo commit")
- [ ] Selettore dei branch: cambio, creazione, eliminazione (con conferma), unione
- [ ] Fetch, pull e push con avanzamento in tempo reale e pulsante «Annulla» che ferma l'operazione
- [ ] Conflitti di unione: banner "unione in corso" con «Annulla unione» e pulsanti di risoluzione nell'editor
- [ ] Diff modificabile: modificare il lato di destra e salvare; il diff e lo stato si aggiornano
- [ ] Pannello GitHub: account, elenco delle pull request e dei controlli
- [ ] Creazione di una pull request (con push dell'upstream se manca) e unione con conferma nativa
- [ ] Clone di un repository dalla finestra di benvenuto: selettore di cartella nativo, il progetto si apre in una nuova finestra
- [ ] Pubblicazione di un progetto su GitHub (creazione del repository, privato e pubblico)
- [ ] Language server reali: pyright (Python), jdtls (Java), gopls (Go), rust-analyzer (Rust) installati sul Mac
  - [ ] Completamento
  - [ ] Hover
  - [ ] Vai alla definizione
  - [ ] Diagnostica (sottolineature e pannello dei problemi)
  - [ ] Indicatore di stato del server (avvio, attivo, errore, non installato con il suggerimento di installazione)
- [ ] Uscire dall'app ferma i language server (`pgrep -fl 'pyright|jdtls|gopls|rust-analyzer'` non trova più nulla)
- [ ] Tema uniforme: vista Git, ricerca, GitHub, diff, widget dei conflitti e popup di completamento usano gli stessi colori del resto
- [ ] RAM con un progetto aperto e i language server attivi (< 250 MB l'app con WebKit)
- [ ] Ricerca su un monorepo vero (decine di migliaia di file) sotto 2 s
- [ ] Installazione dal `.dmg` (trascinamento in Applicazioni, primo avvio con Gatekeeper)
