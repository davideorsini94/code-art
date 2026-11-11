# Code Art

Un IDE personale per Mac, leggero e veloce, costruito attorno alle conversazioni con [Claude Code](https://claude.com/claude-code).

> **Stato: in costruzione.** È in corso la tappa 1 (ramo [`tappa-1`](../../tree/tappa-1)). Non c'è ancora una versione installabile.

## Perché

Chi sviluppa ogni giorno con Claude Code dentro un editor tradizionale finisce con le conversazioni sparse fra i tab dei file, tutte con titoli poco parlanti. Code Art ribalta la prospettiva:

- **Una finestra per progetto**, e il titolo della finestra è il nome del progetto: in Mission Control si capisce subito cosa è cosa.
- **Le conversazioni al centro**, in una colonna dedicata con schede leggibili invece di una fila di tab.
- **Titoli vivi**: ogni conversazione prende il nome del suo argomento principale e lo aggiorna da sola quando l'argomento cambia.
- **Stato a colpo d'occhio**: *al lavoro*, *aspetta te*, *inattiva*, con notifica e badge nel Dock quando Claude ha bisogno di una risposta.
- **Le sessioni Claude Code esistenti** compaiono subito: il motore è la CLI `claude` già installata, con i suoi plugin, skill, agenti e server MCP.

Il tutto in un tema scuro in stile "liquid glass", con editor, terminale, albero dei file e ricerca nella stessa finestra.

## Tappe

| Tappa | Contenuto | Stato |
|---|---|---|
| 1. Il cuore | Finestre per progetto, albero file, editor, terminale, conversazioni Claude con titoli vivi e stati, tema liquid glass | in corso |
| 2. Strumenti da dev | Git e GitHub (commit, branch, merge, pull request, creazione repo), ricerca nel progetto, completamento del codice | da fare |
| 3. SSH | Server salvati, cartelle remote nell'albero, trascinamento file tra Mac e server, terminale remoto | da fare |
| 4. Database | PostgreSQL, MongoDB, Oracle: query, griglia dei risultati modificabile | da fare |

Il design completo è in [`docs/superpowers/specs`](docs/superpowers/specs/2026-11-11-code-art-ide-design.md), il piano della tappa 1 in [`docs/superpowers/plans`](docs/superpowers/plans/2026-11-11-code-art-tappa-1-il-cuore.md).

## Tecnologia

- [Tauri 2](https://tauri.app) — guscio nativo in Rust con la WebView di sistema, senza Chromium incorporato
- [Svelte 5](https://svelte.dev) + TypeScript + Vite per l'interfaccia
- [Monaco](https://microsoft.github.io/monaco-editor/) per l'editor, [xterm.js](https://xtermjs.org) per il terminale
- SQLite per i dati dell'app; i file di Claude Code in `~/.claude` vengono solo letti, mai modificati

## Sviluppo

Requisiti: macOS, [Rust](https://rustup.rs), Node 22+, [pnpm](https://pnpm.io), e la CLI `claude` installata e autenticata.

Il codice è sul ramo `tappa-1`:

```bash
git clone https://github.com/davideorsini94/code-art.git
cd code-art
git checkout tappa-1
pnpm install
pnpm tauri dev
```

Test e controlli:

```bash
pnpm vitest run
pnpm typecheck
cargo test --manifest-path src-tauri/Cargo.toml
```

## Fuori dagli obiettivi

Compatibilità con le estensioni di VS Code, Windows e Linux, sviluppo remoto con agente sul server, debugger grafico, tema chiaro.
