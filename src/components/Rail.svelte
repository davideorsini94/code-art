<script lang="ts">
  import type { ResizableCol } from '../lib/layout'
  import { tooltip } from '../lib/tooltip'

  let { collapsed, ontoggle }: { collapsed: Set<string>; ontoggle: (col: ResizableCol) => void } = $props()

  const items: { col: ResizableCol; icon: string; label: string; what: string }[] = [
    { col: 'sidebar', icon: '◫', label: 'Progetto', what: 'i file del progetto' },
    { col: 'convlist', icon: '☰', label: 'Conversazioni', what: 'l\'elenco delle conversazioni' },
    { col: 'conv', icon: '✎', label: 'Conversazione', what: 'la conversazione' },
  ]
</script>

<nav class="glass rail">
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
