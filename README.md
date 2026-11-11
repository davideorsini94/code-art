# Code Art

A lightweight, fast personal IDE for Mac, built around your conversations with [Claude Code](https://claude.com/claude-code).

> **Status: under construction.** Stage 1 is in progress on the [`tappa-1`](../../tree/tappa-1) branch. There is no installable build yet.

## Why

If you develop with Claude Code every day inside a traditional editor, your conversations end up scattered among file tabs, all with titles that tell you nothing. Code Art flips the perspective:

- **One window per project**, and the window title is the project name, so Mission Control tells you at a glance which is which.
- **Conversations at the center**, in a dedicated column of readable cards instead of a row of tabs.
- **Living titles**: each conversation is named after its main topic and renames itself when the topic changes.
- **Status at a glance**: *working*, *waiting for you*, *idle*, with a notification and a Dock badge when Claude needs an answer.
- **Your existing Claude Code sessions** show up right away: the engine is the `claude` CLI you already have installed, with its plugins, skills, agents and MCP servers.

All of it in a dark "liquid glass" theme, with editor, terminal, file tree and search in the same window.

## Roadmap

| Stage | Scope | Status |
|---|---|---|
| 1. The core | Per-project windows, file tree, editor, terminal, Claude conversations with living titles and statuses, liquid glass theme | in progress |
| 2. Developer tools | Git and GitHub (commit, branch, merge, pull requests, repo creation), project-wide search, code completion | planned |
| 3. SSH | Saved servers, remote folders in the tree, drag and drop between Mac and server, remote terminal | planned |
| 4. Databases | PostgreSQL, MongoDB, Oracle: queries, editable result grid | planned |

The full design is in [`docs/superpowers/specs`](docs/superpowers/specs/2026-11-11-code-art-ide-design.md) and the stage 1 plan in [`docs/superpowers/plans`](docs/superpowers/plans/2026-11-11-code-art-tappa-1-il-cuore.md) (both in Italian).

## Tech stack

- [Tauri 2](https://tauri.app): a native Rust shell using the system WebView, with no bundled Chromium
- [Svelte 5](https://svelte.dev) + TypeScript + Vite for the interface
- [Monaco](https://microsoft.github.io/monaco-editor/) for the editor, [xterm.js](https://xtermjs.org) for the terminal
- SQLite for app data; Claude Code's files in `~/.claude` are only ever read, never modified

## Development

Requirements: macOS, [Rust](https://rustup.rs), Node 22+, [pnpm](https://pnpm.io), and the `claude` CLI installed and signed in.

The code lives on the `tappa-1` branch:

```bash
git clone https://github.com/davideorsini94/code-art.git
cd code-art
git checkout tappa-1
pnpm install
pnpm tauri dev
```

Tests and checks:

```bash
pnpm vitest run
pnpm typecheck
cargo test --manifest-path src-tauri/Cargo.toml
```

## Non-goals

VS Code extension compatibility, Windows and Linux, remote development with a server-side agent, a graphical debugger, a light theme.
