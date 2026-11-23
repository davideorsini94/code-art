<script lang="ts">
  import { tooltip } from '../../lib/tooltip'
  import { db, type DbQueryRequest } from '../../lib/stores/db.svelte'
  import Icon from '../Icon.svelte'
  import type { DbKind } from '../../lib/types'

  /** Albero di una connessione. `disabled` = connessione chiusa: resta visibile in grigio, senza azioni. Tutti i nomi
   *  sono resi come testo. */
  let { connectionId, kind, disabled = false }: { connectionId: string; kind: DbKind; disabled?: boolean } = $props()

  const data = $derived(db.trees[connectionId])
  let expanded = $state<Record<string, boolean>>({})
  let menu = $state<{ x: number; y: number; req: DbQueryRequest; openable: boolean } | null>(null)

  const MENU_W = 180
  const MENU_H = 110
  const toggle = (k: string) => (expanded[k] = !expanded[k])
  const base = (): DbQueryRequest => ({ connectionId, kind })

  function openMenu(ev: MouseEvent, req: DbQueryRequest, openable: boolean) {
    ev.preventDefault()
    ev.stopPropagation()
    if (disabled) return
    menu = {
      x: Math.max(4, Math.min(ev.clientX, window.innerWidth - MENU_W - 4)),
      y: Math.max(4, Math.min(ev.clientY, window.innerHeight - MENU_H - 4)),
      req,
      openable,
    }
  }

  function open(req: DbQueryRequest) {
    if (!disabled) db.openQuery(req)
  }
</script>

<svelte:window onclick={() => (menu = null)} onkeydown={(e) => e.key === 'Escape' && (menu = null)} />

{#snippet chev(open: boolean)}
  <span class="chev" class:open aria-hidden="true">
    <svg width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M3.500 2 6.500 5 3.500 8" /></svg>
  </span>
{/snippet}

<div class="tree" class:off={disabled} role="tree" aria-label="Struttura del database">
  {#if data?.kind === 'postgres'}
    {#each data.tree.schemas as s, i (`${i}:${s.name}`)}
      {@const k = `s:${s.name}`}
      <div class="row" role="treeitem" aria-selected="false" aria-expanded={!!expanded[k]} tabindex="0" use:tooltip={'Mostra o nasconde le tabelle'}
        onclick={() => toggle(k)} onkeydown={(e) => e.key === 'Enter' && toggle(k)}
        oncontextmenu={(e) => openMenu(e, { ...base(), schema: s.name }, false)}>
        {@render chev(!!expanded[k])}
        <span class="ic"><Icon name="files" size={14} /></span>
        <span class="name">{s.name}</span>
      </div>
      {#if expanded[k]}
        {#each s.tables as t, j (`${j}:${t.name}`)}
          {@const req = { ...base(), schema: s.name, table: t.name }}
          <div class="row leaf" role="treeitem" aria-selected="false" tabindex="0" style:padding-left="30px"
            use:tooltip={t.kind === 'view' ? 'Vista: apri in una scheda query' : 'Tabella: apri in una scheda query'}
            onclick={() => open(req)} onkeydown={(e) => e.key === 'Enter' && open(req)}
            oncontextmenu={(e) => openMenu(e, req, true)}>
            <span class="ic"><Icon name={t.kind === 'view' ? 'view' : 'table'} size={14} /></span>
            <span class="name">{t.name}</span>
          </div>
        {/each}
        {#if s.tables.length === 0}<p class="more" style:padding-left="30px">Nessuna tabella</p>{/if}
      {/if}
    {/each}
    {#if data.tree.schemas.length === 0}<p class="more">Nessuno schema</p>{/if}
  {:else if data?.kind === 'mongo'}
    {#each data.dbs as d, i (`${i}:${d.database}`)}
      {@const k = `d:${d.database}`}
      <div class="row" role="treeitem" aria-selected="false" aria-expanded={!!expanded[k]} tabindex="0" use:tooltip={'Mostra o nasconde le collezioni'}
        onclick={() => toggle(k)} onkeydown={(e) => e.key === 'Enter' && toggle(k)}
        oncontextmenu={(e) => openMenu(e, { ...base(), database: d.database }, false)}>
        {@render chev(!!expanded[k])}
        <span class="ic"><Icon name="database" size={14} /></span>
        <span class="name">{d.database}</span>
      </div>
      {#if expanded[k]}
        {#each d.collections as c, j (`${j}:${c}`)}
          {@const req = { ...base(), database: d.database, collection: c }}
          <div class="row leaf" role="treeitem" aria-selected="false" tabindex="0" style:padding-left="30px"
            use:tooltip={'Collezione: apri una ricerca in una scheda query'}
            onclick={() => open(req)} onkeydown={(e) => e.key === 'Enter' && open(req)}
            oncontextmenu={(e) => openMenu(e, req, true)}>
            <span class="ic"><Icon name="table" size={14} /></span>
            <span class="name">{c}</span>
          </div>
        {/each}
        {#if d.collections.length === 0}<p class="more" style:padding-left="30px">Nessuna collezione</p>{/if}
      {/if}
    {/each}
    {#if data.dbs.length === 0}<p class="more">Nessun database</p>{/if}
  {:else if db.treeErrors[connectionId]}
    <p class="err">{db.treeErrors[connectionId]}</p>
  {:else}
    <p class="more">Lettura della struttura…</p>
  {/if}
  {#if data && db.treeErrors[connectionId]}<p class="err">{db.treeErrors[connectionId]}</p>{/if}
</div>

{#if menu}
  {@const m = menu}
  <div class="menu glass-strong" style:left="{m.x}px" style:top="{m.y}px" role="menu">
    <button role="menuitem" use:tooltip={'Apre una scheda query vuota'} onclick={() => { const { table: _t, collection: _c, ...rest } = m.req; menu = null; open(rest) }}>Nuova query</button>
    {#if m.openable}
      <button role="menuitem" use:tooltip={'Apre la tabella o la collezione in una scheda query'} onclick={() => { menu = null; open(m.req) }}>Apri</button>
    {/if}
    <button role="menuitem" use:tooltip={'Rilegge la struttura dal database'} onclick={() => { menu = null; void db.loadTree(connectionId) }}>Aggiorna</button>
  </div>
{/if}

<style>
  .tree { overflow: auto; font-size: 12.5px; user-select: none; }
  .tree.off { opacity: 0.45; }
  .row { display: flex; align-items: center; gap: 5px; height: 24px; padding: 0 8px; border-radius: 6px; cursor: default; color: var(--txt); white-space: nowrap; }
  .row:hover { background: var(--glass); }
  .chev { width: 10px; height: 10px; color: var(--faint); flex: none; display: flex; }
  .chev svg { transition: transform 170ms ease; }
  .chev.open svg { transform: rotate(90deg); }
  @media (prefers-reduced-motion: reduce) { .chev svg { transition: none; } }
  .ic { width: 16px; flex: none; display: flex; color: var(--dim); }
  .name { overflow: hidden; text-overflow: ellipsis; }
  .err { margin: 4px 8px; color: var(--wait); font-size: 12px; overflow-wrap: anywhere; }
  .more { margin: 2px 8px; color: var(--faint); font-size: 11.5px; }
  .menu { position: fixed; z-index: 90; min-width: 160px; padding: 4px; display: flex; flex-direction: column; }
  .menu button { text-align: left; background: none; border: 0; color: var(--txt); padding: 6px 10px; border-radius: 6px; font: inherit; font-size: 12.5px; cursor: default; }
  .menu button:hover { background: var(--glass2); }
</style>
