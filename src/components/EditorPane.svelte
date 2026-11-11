<script lang="ts">
  import { onMount } from 'svelte'
  import { editor, type Tab } from '../lib/stores/editor.svelte'
  import { projectStore } from '../lib/stores/project.svelte'

  type Monaco = typeof import('monaco-editor')
  type IModel = import('monaco-editor').editor.ITextModel
  type ICode = import('monaco-editor').editor.IStandaloneCodeEditor
  type IDiff = import('monaco-editor').editor.IStandaloneDiffEditor

  let monaco = $state.raw<Monaco | null>(null)
  let mainEl = $state<HTMLDivElement>()
  let splitEl = $state<HTMLDivElement>()
  let diffEl = $state<HTMLDivElement>()
  let main: ICode | undefined
  let second: ICode | undefined
  let diffEd: IDiff | undefined
  // modelli per id di tab (stabile dopo rinomina); il testo non viene mai copiato nello store a ogni tasto
  type Entry = { model: IModel; path: string; rev: number; savedVersion: number }
  const models = new Map<number, Entry>()
  const diffModels = new Map<number, [IModel, IModel]>()

  const active = $derived(editor.activeTab)
  // cartella non fidata: editor in sola lettura, niente salvataggio (il core rifiuta comunque)
  const readOnly = $derived(projectStore.untrusted.length > 0)
  const showCode = $derived(!!active && !active.diff && !active.error)

  onMount(() => editor.watch())

  // Monaco si carica solo quando si apre il primo file
  $effect(() => {
    if (editor.tabs.length > 0 && !monaco) {
      void import('../lib/monaco').then((m) => (monaco = m.monaco))
    }
  })

  const options = {
    theme: 'codeart',
    automaticLayout: true,
    minimap: { enabled: true },
    fontSize: 13,
    fontFamily: 'ui-monospace, Menlo, monospace',
    scrollBeyondLastLine: false,
    smoothScrolling: !matchMedia('(prefers-reduced-motion: reduce)').matches,
  } as const

  function addSave(ed: ICode, m: Monaco) {
    ed.addCommand(m.KeyMod.CtrlCmd | m.KeyCode.KeyS, () => {
      const t = editor.activeTab
      if (t) void editor.save(t.path)
    })
  }

  $effect(() => {
    if (!monaco || !mainEl || !diffEl || main) return
    main = monaco.editor.create(mainEl, { ...options, readOnly })
    addSave(main, monaco)
    diffEd = monaco.editor.createDiffEditor(diffEl, { ...options, readOnly: true, renderSideBySide: true })
  })

  $effect(() => {
    if (!monaco || !splitEl) return
    if (editor.split && !second) {
      second = monaco.editor.create(splitEl, { ...options, readOnly, minimap: { enabled: false } })
      addSave(second, monaco)
    } else if (!editor.split && second) {
      second.dispose()
      second = undefined
    }
  })

  $effect(() => {
    const ro = readOnly
    if (!monaco) return
    main?.updateOptions({ readOnly: ro })
    second?.updateOptions({ readOnly: ro })
  })

  function ensureModel(m: Monaco, t: Tab) {
    let entry = models.get(t.id)
    if (entry && entry.path !== t.path) {
      // file rinominato/spostato: nuovo modello (lingua dal nuovo nome) con il testo corrente, modifiche comprese
      const text = entry.model.getValue()
      entry.model.dispose()
      models.delete(t.id)
      entry = undefined
      return create(m, t, text, t.dirty)
    }
    if (!entry) return create(m, t, t.baseText, false)
    if (entry.rev !== t.rev) {
      // ricarica da disco: unico caso in cui si spinge testo nel modello
      entry.model.setValue(t.baseText)
      entry.rev = t.rev
      entry.savedVersion = entry.model.getAlternativeVersionId()
      editor.setDirty(t.id, false)
    }
    return entry.model
  }

  function create(m: Monaco, t: Tab, text: string, dirty: boolean) {
    const model = m.editor.createModel(text, undefined, m.Uri.file(t.path))
    const entry: Entry = { model, path: t.path, rev: t.rev, savedVersion: dirty ? -1 : model.getAlternativeVersionId() }
    models.set(t.id, entry)
    const id = t.id
    model.onDidChangeContent(() => editor.setDirty(id, model.getAlternativeVersionId() !== entry.savedVersion))
    editor.bind(id, {
      snapshot: () => {
        const v = model.getAlternativeVersionId()
        return { text: model.getValue(), commit: () => { entry.savedVersion = v; editor.setDirty(id, model.getAlternativeVersionId() !== v) } }
      },
    })
    return model
  }

  // sincronizza modelli e editor con le tab
  $effect(() => {
    const m = monaco
    if (!m || !main) return
    const live = new Set(editor.tabs.filter((t) => !t.diff && !t.error).map((t) => t.id))
    for (const [id, e] of models) {
      if (!live.has(id)) {
        e.model.dispose()
        models.delete(id)
        editor.bind(id, null)
      }
    }
    const liveDiff = new Set(editor.tabs.filter((t) => t.diff).map((t) => t.id))
    for (const [id, pair] of diffModels) {
      if (!liveDiff.has(id)) {
        pair.forEach((x) => x.dispose())
        diffModels.delete(id)
      }
    }
    for (const t of editor.tabs) {
      if (t.error) continue
      if (t.diff) {
        if (!diffModels.has(t.id)) {
          diffModels.set(t.id, [
            m.editor.createModel(t.diff.before, undefined, m.Uri.parse(`codeart-diff:before${t.path}`)),
            m.editor.createModel(t.diff.after, undefined, m.Uri.parse(`codeart-diff:after${t.path}`)),
          ])
        }
        continue
      }
      ensureModel(m, t)
    }
    const a = editor.activeTab
    if (a && !a.diff && !a.error) {
      const model = models.get(a.id)?.model
      if (model) {
        if (main.getModel() !== model) main.setModel(model)
        if (second && second.getModel() !== model) second.setModel(model)
      }
    } else if (a?.diff) {
      const pair = diffModels.get(a.id)
      if (pair) diffEd?.setModel({ original: pair[0], modified: pair[1] })
    }
  })
</script>

<div class="pane">
  <div class="tabs" role="tablist">
    {#each editor.tabs as t (t.id)}
      <div class="tab" class:on={t.path === editor.active} role="tab" aria-selected={t.path === editor.active} tabindex="0"
        onclick={() => (editor.active = t.path)} onkeydown={(e) => e.key === 'Enter' && (editor.active = t.path)}>
        <span class="tname">{t.name}</span>
        {#if editor.isDirty(t)}<span class="dot" title="Modifiche non salvate"></span>{/if}
        <button class="x" aria-label="Chiudi {t.name}" onclick={(e) => { e.stopPropagation(); editor.close(t.path) }}>×</button>
      </div>
    {/each}
    <span class="grow"></span>
    {#if editor.tabs.length > 0}
      <button class="tool" class:on={editor.split} onclick={() => (editor.split = !editor.split)} title="Vista divisa">Dividi</button>
    {/if}
  </div>

  {#if active?.prompt}
    <div class="bar" role="alert">
      <span>Cambiato su disco</span>
      <button onclick={() => void editor.reload(active.path)}>Ricarica</button>
      <button onclick={() => void editor.keepMine(active.path)}>Tieni il mio</button>
    </div>
  {/if}

  <div class="body">
    {#if editor.tabs.length === 0}
      <p class="empty">Apri un file dall'albero</p>
    {:else if active?.error === 'too_large' || active?.error === 'not_text'}
      <p class="empty">File non modificabile qui</p>
    {:else if active?.error === 'forbidden'}
      <p class="empty">Percorso fuori dal progetto</p>
    {:else if active?.error}
      <p class="empty">Impossibile aprire il file</p>
    {/if}
    <div class="editors" class:hidden={!showCode}>
      <div class="ed" bind:this={mainEl}></div>
      <div class="ed" class:hidden={!editor.split} bind:this={splitEl}></div>
    </div>
    <div class="ed diff" class:hidden={!active?.diff} bind:this={diffEl}></div>
  </div>
</div>

<style>
  .pane { display: flex; flex-direction: column; height: 100%; min-height: 0; }
  .tabs { display: flex; align-items: center; gap: 2px; padding: 6px 6px 0; overflow-x: auto; flex: none; }
  .tab {
    display: flex; align-items: center; gap: 6px; padding: 5px 8px 5px 12px; border-radius: 8px 8px 0 0;
    font-size: 12px; color: var(--dim); cursor: default; white-space: nowrap; transition: background var(--dur);
  }
  .tab:hover { background: var(--glass); }
  .tab.on { background: var(--glass2); color: var(--txt); }
  .dot { width: 7px; height: 7px; border-radius: 50%; background: var(--wait); }
  .x { background: none; border: 0; color: var(--faint); font-size: 14px; line-height: 1; padding: 0 3px; border-radius: 4px; cursor: default; }
  .x:hover { background: var(--glass2); color: var(--txt); }
  .grow { flex: 1; }
  .tool { background: none; border: 1px solid var(--edge-soft); color: var(--dim); font-size: 11px; padding: 3px 9px; border-radius: 6px; cursor: default; }
  .tool.on { color: var(--accent); border-color: var(--accent); }
  .bar {
    display: flex; align-items: center; gap: 10px; padding: 7px 12px; font-size: 12px; flex: none;
    background: rgba(255, 180, 84, 0.14); color: var(--wait);
  }
  .bar button { background: var(--glass2); border: 1px solid var(--edge-soft); color: var(--txt); border-radius: 6px; padding: 3px 10px; font-size: 12px; cursor: default; }
  .body { position: relative; flex: 1; min-height: 0; }
  .editors { display: flex; height: 100%; }
  .ed { flex: 1; min-width: 0; height: 100%; }
  .diff { position: absolute; inset: 0; }
  .editors .ed + .ed { border-left: 1px solid var(--edge-soft); }
  .hidden { display: none; }
  .empty { position: absolute; inset: 0; display: grid; place-items: center; margin: 0; color: var(--faint); font-size: 13px; pointer-events: none; }
</style>
