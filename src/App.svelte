<script lang="ts">
  import Titlebar from './components/Titlebar.svelte'
  import Rail from './components/Rail.svelte'
  import MissingFolder from './components/MissingFolder.svelte'
  import TrustBanner from './components/TrustBanner.svelte'
  import FileTree from './components/FileTree.svelte'
  import EditorPane from './components/EditorPane.svelte'
  import TerminalPane from './components/TerminalPane.svelte'
  import ClaudeMissing from './components/ClaudeMissing.svelte'
  import ConvList from './components/ConvList.svelte'
  import ConvPane from './components/ConvPane.svelte'
  import Palette from './components/Palette.svelte'
  import { palette } from './lib/palette/state.svelte'
  import { hooks } from './lib/palette/search'
  import { conversations } from './lib/stores/conversations.svelte'
  import { projectStore } from './lib/stores/project.svelte'
  import { projectPickAndOpen } from './lib/ipc'
  import { onMount } from 'svelte'
  import { canCollapse, loadLayout, saveLayout, MIN_WIDTH, type ResizableCol } from './lib/layout'

  let claudeFound = $state(false)

  onMount(() => {
    void projectStore.load().then(() => {
      if (projectStore.project) void conversations.start(projectStore.project.id)
    })
    return () => conversations.stop()
  })

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

  let termVisible = $state(false)
  let termHeight = $state(240)
  const MIN_TERM = 100

  // ⌃` mostra/nasconde il terminale (in fase di cattura: vale anche con il focus dentro xterm)
  onMount(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.ctrlKey && !e.metaKey && !e.altKey && (e.key === '`' || e.code === 'Backquote')) {
        e.preventDefault()
        e.stopPropagation()
        termVisible = !termVisible
      }
    }
    window.addEventListener('keydown', onKey, true)
    hooks.toggleTerminal = () => (termVisible = !termVisible)
    return () => window.removeEventListener('keydown', onKey, true)
  })

  // ⌘P file, ⌘⇧P comandi, ⌘K tutto (in fase di cattura: vale anche con il focus in Monaco o xterm)
  onMount(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!e.metaKey || e.ctrlKey || e.altKey || !projectStore.project) return
      const k = e.key.toLowerCase()
      const mode = k === 'k' && !e.shiftKey ? 'all' : k === 'p' ? (e.shiftKey ? 'commands' : 'files') : null
      if (!mode) return
      e.preventDefault()
      e.stopPropagation()
      palette.show(mode)
    }
    window.addEventListener('keydown', onKey, true)
    return () => window.removeEventListener('keydown', onKey, true)
  })

  function startTermResize(e: PointerEvent) {
    const startY = e.clientY
    const startH = termHeight
    const max = () => Math.max(MIN_TERM, window.innerHeight - 200)
    const move = (ev: PointerEvent) => {
      termHeight = Math.min(max(), Math.max(MIN_TERM, startH - (ev.clientY - startY)))
    }
    const up = () => {
      window.removeEventListener('pointermove', move)
      window.removeEventListener('pointerup', up)
    }
    window.addEventListener('pointermove', move)
    window.addEventListener('pointerup', up)
  }

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
  {#if projectStore.loaded && !projectStore.project}
    <main class="welcome">
      <div class="glass welcome-box">
        <h1>Code Art</h1>
        <button onclick={() => projectPickAndOpen()}>Apri cartella</button>
      </div>
    </main>
  {:else}
  <TrustBanner />
  <main class="body" style:grid-template-columns={template}>
    <div class="slot rail"><Rail {collapsed} ontoggle={toggle} /></div>
    {#each cols as { col, label } (col)}
      {#if !collapsed.has(col)}
        <section class="glass col" data-slot={col} aria-label={label}>
          {#if col === 'sidebar'}<FileTree />{/if}
          {#if col === 'conv'}<ConvPane />{/if}
          {#if col === 'convlist'}
            <ClaudeMissing onresult={(f) => (claudeFound = f)} />
            {#if claudeFound}<ConvList />{/if}
          {/if}
        </section>
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <div class="handle" onpointerdown={(e) => startResize(e, col)}></div>
      {:else}
        <span></span><span></span>
      {/if}
    {/each}
    <section class="surface-opaque col editor" data-slot="editor" aria-label="Editor">
      <div class="editor-area"><EditorPane /></div>
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div class="vhandle" class:hidden={!termVisible} onpointerdown={startTermResize}></div>
      <div class="term-area" class:hidden={!termVisible} style:height="{termHeight}px"><TerminalPane visible={termVisible} /></div>
    </section>
  </main>
  {/if}
  <MissingFolder />
  <Palette />
</div>

<style>
  .app { display: flex; flex-direction: column; height: 100%; }
  .body { flex: 1; min-height: 0; display: grid; padding: 10px; }
  .col { min-width: 0; overflow: hidden; padding: 12px; }
  .col.editor { padding: 0; display: flex; flex-direction: column; }
  .editor-area { flex: 1; min-height: 0; }
  .term-area { flex: none; min-height: 0; }
  .vhandle { flex: none; height: 5px; margin-top: -2px; cursor: row-resize; position: relative; z-index: 1; }
  .vhandle:hover { background: var(--edge-soft); }
  .hidden { display: none; }
  .slot.rail { display: contents; }
  .welcome { flex: 1; display: grid; place-items: center; }
  .welcome-box { padding: 32px 48px; display: flex; flex-direction: column; align-items: center; gap: 16px; }
  .welcome-box h1 { margin: 0; font-size: 22px; }
  .welcome-box button {
    padding: 8px 20px;
    border-radius: 8px;
    border: 1px solid var(--edge-soft);
    background: var(--glass);
    color: inherit;
    cursor: pointer;
  }
  .handle { cursor: col-resize; }
  .handle:hover { background: var(--edge-soft); border-radius: 3px; }
</style>
