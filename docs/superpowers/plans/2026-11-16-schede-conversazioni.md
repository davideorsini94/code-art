# Conversazioni a schede — piano

> Esecuzione: nativa (in sessione), poi una revisione finale dell'intero ramo.

**Obiettivo:** le schede delle conversazioni aperte in cima alla colonna della conversazione, e l'elenco completo
come vista "Conversazioni" della barra laterale.

**Spec:** `docs/superpowers/specs/2026-11-16-schede-conversazioni-design.md`

## Vincoli globali

- Nessun cambio al core Rust.
- Testi in italiano, tooltip su ogni pulsante (`use:tooltip`), tema scuro con le variabili esistenti.
- `localStorage` sempre dentro try/catch.

## Da tenere d'occhio

- Una scheda salvata la cui conversazione non esiste più (eliminata fuori dall'app) → scartata al caricamento.
- `conv:focus` di una conversazione non aperta → apre la scheda.
- Archiviare l'attiva → la scheda si chiude, si attiva la vicina.
- Layout salvato con `convlist` chiusa o `conv` chiusa → si carica senza errori, `conv` torna richiudibile.
- Molte schede → barra scorrevole, l'attiva visibile.

---

### Task 1: logica delle schede

**File:** crea `src/lib/conv/tabs.ts` + `tabs.test.ts`; modifica `src/lib/stores/conversations.svelte.ts`.

**Produce:**
- `openTab(tabs: string[], id: string): string[]` — in coda se manca.
- `closeTab(tabs: string[], activeId: string | null, id: string): { tabs: string[]; activeId: string | null }` —
  chiudendo l'attiva: la vicina a destra, altrimenti a sinistra, altrimenti `null`.
- `pruneTabs(tabs: string[], known: Set<string>): string[]`.
- `loadTabs(projectId)` / `saveTabs(projectId, state)` su `codeart.convtabs.<projectId>`, `{ tabs, activeId }`.
- Nello store: `tabs = $state<string[]>([])`, `openConvs = $derived(...)` (le `Conversation` delle schede, in
  ordine), `close(id)`. `select` apre la scheda; `remove` e `archive(id, true)` la chiudono; `start` ripristina
  (dopo `convList`, potato sugli id non archiviati) e ogni cambio salva.

Test: apertura idempotente; chiusura attiva al centro → destra; ultima → sinistra; unica → null; chiusura non
attiva lascia l'attiva; prune; load con JSON rotto → vuoto.

### Task 2: barra delle schede

**File:** crea `src/components/ConvTabs.svelte` + test; modifica `ConvPane.svelte`.

Scheda: pallino di stato (stessi colori di `ConvCard`), titolo troncato con tooltip, × (hover e sempre
sull'attiva), clic centrale chiude, `role="tab"`/`aria-selected`. Barra scorrevole in orizzontale, attiva portata
in vista (`scrollIntoView({ inline: 'nearest' })`). `ConvPane`: barra sempre in cima; senza scheda l'invito
"Apri una conversazione dall'elenco o premi ＋ nuova". Il titolo grande nell'intestazione resta (con "prima: …").

Test: rende una scheda per conversazione aperta, clic → `select`, × → `close`, attiva marcata.

### Task 3: vista laterale e via la colonna

**File:** `sidebar.svelte.ts` (vista `convs`), `SidebarView.svelte`, `Rail.svelte`, `layout.ts` (+ test),
`App.svelte` (colonne, scorciatoia), `ConvList.svelte` (adattata alla larghezza della barra laterale).

- `SidebarViewId` aggiunge `'convs'`; Rail la mostra come quarta vista, scorciatoia ⌘⇧C se libera.
- `ResizableCol = 'sidebar' | 'conv'`; `canCollapse` sempre vero; `loadLayout` ignora `convlist`.
- `ClaudeMissing` va nella colonna `conv` (copre la conversazione finché manca `claude`).

Test: `loadLayout` con layout vecchio; Rail con la vista Conversazioni.

### Task 4: verifica

`pnpm test`, `pnpm check`, `pnpm build`; poi build dell'app (`pnpm tauri build`) da mandare all'utente.
