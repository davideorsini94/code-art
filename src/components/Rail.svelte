<script lang="ts">
  import type { ResizableCol } from '../lib/layout'
  import { tooltip } from '../lib/tooltip'
  import { sidebar, type SidebarViewId } from '../lib/stores/sidebar.svelte'

  let { collapsed, ontoggle }: { collapsed: Set<string>; ontoggle: (col: ResizableCol) => void } = $props()

  const views: { id: SidebarViewId; icon: string; label: string; what: string; shortcut: string }[] = [
    { id: 'files', icon: '◫', label: 'Esplora', what: 'i file del progetto', shortcut: '⌘⇧E' },
    { id: 'search', icon: '⌕', label: 'Cerca', what: 'la ricerca nel progetto', shortcut: '⌘⇧F' },
    { id: 'git', icon: '⑂', label: 'Git', what: 'il controllo del codice', shortcut: '⌘⇧G' },
  ]
  const items: { col: ResizableCol; icon: string; label: string; what: string }[] = [
    { col: 'convlist', icon: '☰', label: 'Conversazioni', what: 'l\'elenco delle conversazioni' },
    { col: 'conv', icon: '✎', label: 'Conversazione', what: 'la conversazione' },
  ]

  const sidebarOpen = $derived(!collapsed.has('sidebar'))

  // vista attiva con il pannello aperto: lo chiude; altrimenti apre il pannello su quella vista
  function pick(id: SidebarViewId) {
    if (sidebarOpen && sidebar.view === id) return ontoggle('sidebar')
    sidebar.setView(id)
    if (!sidebarOpen) ontoggle('sidebar')
  }
</script>

<nav class="glass rail">
  {#each views as v (v.id)}
    {@const active = sidebarOpen && sidebar.view === v.id}
    <button
      class:on={active}
      aria-pressed={active}
      use:tooltip={{ text: `${active ? 'Nascondi' : 'Mostra'} ${v.what}`, shortcut: v.shortcut }}
      aria-label={v.label}
      onclick={() => pick(v.id)}
    >
      {v.icon}
    </button>
  {/each}
  {#each items as it (it.col)}
    <button class:on={!collapsed.has(it.col)} use:tooltip={`${collapsed.has(it.col) ? 'Mostra' : 'Nascondi'} ${it.what}`} aria-label={it.label} onclick={() => ontoggle(it.col)}>
      {it.icon}
    </button>
  {/each}
  <button class="bottom" use:tooltip={'Impostazioni: in arrivo'} aria-label="Impostazioni" aria-disabled="true">⚙</button>
</nav>

<style>
  .rail { display: flex; flex-direction: column; align-items: center; gap: 8px; padding: 8px 0; }
  button {
    width: 32px; height: 32px; border-radius: 10px; border: 0; background: none;
    display: grid; place-items: center; font-size: 15px; color: var(--dim); cursor: pointer;
    transition: background var(--dur) ease, color var(--dur) ease;
  }
  button.on { background: var(--glass2); color: var(--txt); box-shadow: inset 0 1px 0 var(--hi); }
  .bottom { margin-top: auto; }
  .bottom[aria-disabled='true'] { opacity: 0.4; cursor: default; }
</style>
