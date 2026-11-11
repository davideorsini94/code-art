# Verifica della tappa 1

Misure fatte da riga di comando su macOS (Apple Silicon), build release firmata ad-hoc.

## Pacchetto

- Comando: `pnpm tauri build` (oppure `pnpm build:dmg`). Output in `src-tauri/target/release/bundle/`:
  `macos/Code Art.app` e `dmg/Code Art_0.1.0_aarch64.dmg` (non versionati).
- Icona: `python3 scripts/genera-icona.py <png>` poi `pnpm tauri icon <png>`.
- Nota di build: con `strip = true` nel profilo release, su questo macOS i proc-macro (dylib) strippati
  non sono caricabili da rustc ("mis-aligned LINKEDIT string pool"); il build falliva con
  `can't find crate for phf_macros`. Risolto con `[profile.release.build-override] strip = "none"`.

## Numeri

| Obiettivo | Misura | Esito |
|---|---|---|
| `Code Art.app` < 40 MB | 11 MB (`du -sh "…/Code Art.app"`); dmg 7,3 MB (`ls -l`). Tolto poi `panic = "abort"` dal profilo release (serve a `catch_unwind`): eseguibile 9,17 → 11,27 MB, app stimata ~13 MB | OK (Monaco è caricato in modo pigro, nessuna riduzione necessaria) |
| Avvio a freddo → finestra ≤ 1 s | 339–608 ms (3 lanci; il primo, a freddo, 608 ms) | OK |
| RAM app + WebKit < 250 MB | ~95 MB di `footprint` (app 28–34, WebContent 46, GPU 14, Networking 6) sulla finestra di benvenuto; RSS sommato ~200 MB | provvisorio (solo finestra di benvenuto) |

Metodi:
- Avvio: piccolo programma Swift che lancia il binario e interroga `CGWindowListCopyWindowInfo` ogni 3 ms
  finché compare una finestra a livello 0 del processo; margine d'errore ~±5 ms più l'avvio del processo.
  Con `CODEART_TIMING=1` l'app scrive su stderr `[timing] finestra creata in N ms` (~280 ms dall'inizio di `run`)
  e `[timing] pagina caricata in N ms` (~370–395 ms): la pagina è quindi disegnata entro ~0,4 s.
- Memoria: `footprint <pid>` per il processo dell'app e per i processi `com.apple.WebKit.*` comparsi dopo il lancio
  (sono figli XPC con ppid 1, individuati confrontando `pgrep -f com.apple.WebKit` prima e dopo). Misurata sulla
  finestra di benvenuto (nessun progetto aperto: non c'era alcun progetto da aprire senza dialogo di sistema).
  Con un progetto aperto e Monaco/xterm caricati il valore sarà più alto: da rimisurare a vista.

## Avvio

Nessun panic, nessun errore su stdout/stderr all'avvio dell'app compilata; il database viene creato in
`~/Library/Application Support/CodeArt/`. CSP e capability restrittive non hanno causato errori visibili da CLI
(gli errori della WebView non sono leggibili da riga di comando: confermare con gli Strumenti per sviluppatori).

## Titolo doppio

Corretto: `hidden_title(true)` su ogni finestra (il titolo nativo resta impostato per Mission Control, menu
Finestra e Dock). Test `native_title_hidden_but_still_set`. La cattura dello schermo non era autorizzata da CLI:
l'assenza del doppio disegno va confermata a vista.

## Suite

Dopo l'ondata di correzioni della revisione finale: `pnpm typecheck` (0 errori, 0 warning), `pnpm vitest run`
(370 test in 38 file), `cargo test` (207 + 6 test), `cargo check` senza warning: tutto verde.

## Da verificare a vista

- [ ] Due progetti → due finestre con i nomi giusti in Mission Control
- [ ] Titolo mostrato una sola volta nella barra, non sotto i tre pulsanti
- [ ] Aspetto del vetro e dell'icona nel Dock
- [ ] Sessioni esistenti presenti con titoli generati
- [ ] Tre conversazioni in parallelo con stati corretti
- [ ] Titolo che cambia dopo un cambio di argomento e resta stabile altrimenti
- [ ] Skill, subagent, Playwright MCP, immagine incollata, cambio modello, modalità piano
- [ ] Modifica di Claude visibile come diff
- [ ] Conversazione proseguita da VS Code che si aggiorna nell'app
- [ ] Scorrimento di un file da 5 MB e di una conversazione con oltre 500 elementi senza scatti
- [ ] RAM con un progetto aperto e nessun turno in corso (< 250 MB)
- [ ] Installazione dal `.dmg` (trascinamento in Applicazioni, primo avvio con Gatekeeper)
