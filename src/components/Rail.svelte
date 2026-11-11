<script lang="ts">
  import type { ResizableCol } from '../lib/layout'

  let { collapsed, ontoggle }: { collapsed: Set<string>; ontoggle: (col: ResizableCol) => void } = $props()

  const items: { col: ResizableCol; icon: string; label: string }[] = [
    { col: 'sidebar', icon: '◫', label: 'Progetto' },
    { col: 'convlist', icon: '☰', label: 'Conversazioni' },
    { col: 'conv', icon: '✎', label: 'Conversazione' },
  ]
</script>

<nav class="glass rail">
  {#each items as it (it.col)}
    <button class:on={!collapsed.has(it.col)} title={it.label} aria-label={it.label} onclick={() => ontoggle(it.col)}>
      {it.icon}
    </button>
  {/each}
  <button class="bottom" title="Impostazioni" aria-label="Impostazioni">⚙</button>
</nav>

<style>
  .rail { display: flex; flex-direction: column; align-items: center; gap: 8px; padding: 8px 0; }
  button {
    width: 32px; height: 32px; border-radius: 10px; border: 0; background: none;
    display: grid; place-items: center; font-size: 15px; color: var(--dim); cursor: pointer;
    transition: background var(--dur) ease, color var(--dur) ease;
  }
  button.on { background: var(--glass2); color: var(--txt); box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.2); }
  .bottom { margin-top: auto; }
</style>
