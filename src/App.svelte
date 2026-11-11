<script lang="ts">
  import Titlebar from './components/Titlebar.svelte'
  import Rail from './components/Rail.svelte'
  import { canCollapse, loadLayout, saveLayout, MIN_WIDTH, type ResizableCol } from './lib/layout'

  const initial = loadLayout()
  let widths = $state(initial.widths)
  let collapsed = $state(new Set<string>(initial.collapsed))

  $effect(() => {
    saveLayout({ widths: { ...widths }, collapsed: [...collapsed] as ResizableCol[] })
  })

  function toggle(col: ResizableCol) {
    const next = new Set(collapsed)
    if (next.has(col)) next.delete(col)
    else if (canCollapse(col, collapsed)) next.add(col)
    collapsed = next
  }

  const cols: { col: ResizableCol; label: string }[] = [
    { col: 'sidebar', label: 'Progetto' },
    { col: 'convlist', label: 'Conversazioni' },
    { col: 'conv', label: 'Conversazione' },
  ]

  // colonna chiusa: 0px; altrimenti larghezza salvata. Dopo ogni colonna c'è una maniglia da 6px.
  const template = $derived(
    ['46px', ...cols.flatMap(({ col }) => (collapsed.has(col) ? ['0px', '0px'] : [`${widths[col]}px`, '6px'])), 'minmax(240px, 1fr)'].join(' '),
  )

  function startResize(e: PointerEvent, col: ResizableCol) {
    const startX = e.clientX
    const startW = widths[col]
    const move = (ev: PointerEvent) => {
      widths[col] = Math.max(MIN_WIDTH, startW + ev.clientX - startX)
    }
    const up = () => {
      window.removeEventListener('pointermove', move)
      window.removeEventListener('pointerup', up)
    }
    window.addEventListener('pointermove', move)
    window.addEventListener('pointerup', up)
  }
</script>

<div class="app">
  <Titlebar />
  <main class="body" style:grid-template-columns={template}>
    <div class="slot rail"><Rail {collapsed} ontoggle={toggle} /></div>
    {#each cols as { col, label } (col)}
      {#if !collapsed.has(col)}
        <section class="glass col" data-slot={col} aria-label={label}></section>
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <div class="handle" onpointerdown={(e) => startResize(e, col)}></div>
      {:else}
        <span></span><span></span>
      {/if}
    {/each}
    <section class="glass col" data-slot="editor" aria-label="Editor"></section>
  </main>
</div>

<style>
  .app { display: flex; flex-direction: column; height: 100%; }
  .body { flex: 1; min-height: 0; display: grid; padding: 10px; }
  .col { min-width: 0; overflow: hidden; padding: 12px; }
  .slot.rail { display: contents; }
  .handle { cursor: col-resize; }
  .handle:hover { background: var(--edge-soft); border-radius: 3px; }
</style>
