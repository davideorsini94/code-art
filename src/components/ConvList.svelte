<script lang="ts">
  import { onMount } from 'svelte'
  import { conversations } from '../lib/stores/conversations.svelte'
  import { windowRange, WINDOW_THRESHOLD } from '../lib/conv/order'
  import ConvCard from './ConvCard.svelte'

  /** Altezza fissa di una riga (scheda + spazio) usata dalla finestra di rendering oltre 50 schede. */
  const ROW = 112

  let now = $state(Date.now())
  let renamingId = $state<string | null>(null)
  let menu = $state<{ x: number; y: number; id: string; archived: boolean; locked: boolean } | null>(null)
  let scrollTop = $state(0)
  let viewport = $state(600)

  const list = $derived(conversations.list)
  const virtual = $derived(list.length > WINDOW_THRESHOLD)
  const range = $derived(windowRange(list.length, scrollTop, viewport, ROW))
  const visible = $derived(list.slice(range.start, range.end))

  onMount(() => {
    const tick = setInterval(() => (now = Date.now()), 60_000)
    const close = () => (menu = null)
    window.addEventListener('click', close)
    return () => {
      clearInterval(tick)
      window.removeEventListener('click', close)
    }
  })

  function measure(node: HTMLElement) {
    const ro = new ResizeObserver(() => (viewport = node.clientHeight))
    ro.observe(node)
    viewport = node.clientHeight
    return { destroy: () => ro.disconnect() }
  }
</script>

<div class="wrap">
  <div class="h">
    <span>Conversazioni</span>
    <span class="actions">
      <button class="link" class:on={conversations.showArchived} onclick={() => (conversations.showArchived = !conversations.showArchived)}>Archiviate</button>
      <button class="link" onclick={() => void conversations.create()}>＋ nuova</button>
    </span>
  </div>
  {#if conversations.error}<p class="err" role="alert">{conversations.error}</p>{/if}
  <div class="scroll" use:measure onscroll={(e) => (scrollTop = e.currentTarget.scrollTop)}>
    {#if list.length === 0}
      <p class="empty">{conversations.showArchived ? 'Nessuna conversazione archiviata.' : 'Nessuna conversazione. Premi “＋ nuova” per iniziare.'}</p>
    {:else if virtual}
      <div style:height="{list.length * ROW}px" style:position="relative">
        {#each visible as c, i (c.sessionId)}
          <div class="row" style:position="absolute" style:top="{(range.start + i) * ROW}px" style:height="{ROW}px" style:left="0" style:right="0">
            {@render card(c)}
          </div>
        {/each}
      </div>
    {:else}
      {#each list as c (c.sessionId)}
        <div class="row">{@render card(c)}</div>
      {/each}
    {/if}
  </div>
</div>

{#snippet card(c: import('../lib/types').Conversation)}
  <ConvCard
    conv={c}
    {now}
    active={conversations.activeId === c.sessionId}
    renaming={renamingId === c.sessionId}
    onselect={() => void conversations.select(c.sessionId)}
    onstartrename={() => (renamingId = c.sessionId)}
    onrename={(t) => {
      renamingId = null
      void conversations.rename(c.sessionId, t)
    }}
    oncancelrename={() => (renamingId = null)}
    onunlock={() => void conversations.unlockTitle(c.sessionId)}
    oncontextmenu={(ev) => {
      ev.preventDefault()
      ev.stopPropagation()
      menu = { x: ev.clientX, y: ev.clientY, id: c.sessionId, archived: c.archived, locked: c.titleLocked }
    }}
  />
{/snippet}

{#if menu}
  <div class="menu glass-strong" style:left="{menu.x}px" style:top="{menu.y}px" role="menu">
    <button role="menuitem" onclick={() => { renamingId = menu!.id; menu = null }}>Rinomina</button>
    {#if menu.locked}
      <button role="menuitem" onclick={() => { void conversations.unlockTitle(menu!.id); menu = null }}>Sblocca titolo</button>
    {/if}
    <button role="menuitem" onclick={() => { void conversations.archive(menu!.id, !menu!.archived); menu = null }}>
      {menu.archived ? 'Ripristina' : 'Archivia'}
    </button>
  </div>
{/if}

<style>
  .wrap { display: flex; flex-direction: column; height: 100%; min-height: 0; }
  .h {
    font-size: 10.5px; letter-spacing: 0.09em; text-transform: uppercase; color: var(--faint);
    margin: 2px 0 8px; display: flex; justify-content: space-between; align-items: center;
  }
  .actions { display: flex; gap: 10px; }
  .link { background: none; border: 0; padding: 0; font: inherit; letter-spacing: inherit; text-transform: inherit; color: inherit; cursor: pointer; }
  .link:hover, .link.on { color: var(--accent); }
  .scroll { flex: 1; min-height: 0; overflow-y: auto; }
  .row { margin-bottom: 7px; }
  .row:last-child { margin-bottom: 0; }
  .empty, .err { margin: 8px 2px; font-size: 12px; color: var(--faint); }
  .err { color: var(--wait); }
  .menu {
    position: fixed; z-index: 90; min-width: 160px; padding: 4px; display: flex; flex-direction: column;
    background: rgba(24, 27, 38, 0.96);
  }
  .menu button {
    text-align: left; background: none; border: 0; color: var(--txt); padding: 6px 10px;
    border-radius: 6px; font-size: 12.5px; cursor: default;
  }
  .menu button:hover { background: var(--glass2); }
</style>
