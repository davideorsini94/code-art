<script lang="ts">
  import { untrack } from 'svelte'
  import { editor, type Tab } from '../lib/stores/editor.svelte'
  import { projectStore } from '../lib/stores/project.svelte'
  import { iconFor } from '../lib/fileicons'
  import { isBinaryViewer, type MarkdownMode } from '../lib/viewers'
  import { previewPolicy } from '../lib/md-policy'
  import FileIcon from './FileIcon.svelte'
  import { tooltip } from '../lib/tooltip'
  import { relativeToFolders } from '../lib/conv/paths'

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
  const isTextTab = (t: Tab) => !t.diff && (t.viewer === 'text' || t.viewer === 'markdown') && !t.error
  // markdown: Anteprima (predefinita), Modifica o Affiancate; ricordato per file
  const mdActive = $derived(active && active.viewer === 'markdown' && !active.error && !active.diff ? active : undefined)
  const mdMode = $derived<MarkdownMode>(mdActive ? editor.modeOf(mdActive.path) : 'edit')
  const showCode = $derived(!!active && isTextTab(active) && (!mdActive || mdMode !== 'preview'))
  const showPreview = $derived(!!mdActive && mdMode !== 'edit')
  const showBinary = $derived(!!active && !active.diff && (active.viewer === 'binary' || active.error === 'too_large' || active.error === 'not_text'))
  const showViewer = $derived(!!active && !active.diff && !active.error && isBinaryViewer(active.viewer) && active.viewer !== 'binary')
  const MODES: [MarkdownMode, string, string][] = [['preview', 'Anteprima', 'Mostra solo l\'anteprima'], ['edit', 'Modifica', 'Mostra solo il codice'], ['split', 'Affiancate', 'Codice e anteprima affiancati']]

  $effect(() => {
    const pid = projectStore.project?.id
    if (pid) editor.watch(pid)
  })

  // testo dell'anteprima markdown: quello corrente dell'editor (modifiche non salvate comprese), con 150 ms di attesa
  let mdText = $state('')
  let modelTick = $state(0)
  // documenti grandi: il render è sincrono, quindi l'anteprima si fa solo su richiesta (una volta, poi "Aggiorna")
  const policy = $derived(previewPolicy(mdText.length))
  let manual = $state<{ id: number; text: string } | null>(null)
  function renderNow() {
    const t = mdActive
    if (!t) return
    manual = { id: t.id, text: models.get(t.id)?.model.getValue() ?? t.baseText }
  }
  $effect(() => {
    const t = mdActive
    void modelTick
    if (!t || mdMode === 'edit') return
    const entry = models.get(t.id)
    if (!entry) {
      mdText = t.baseText
      return
    }
    mdText = entry.model.getValue()
    let timer: ReturnType<typeof setTimeout> | undefined
    const sub = entry.model.onDidChangeContent(() => {
      clearTimeout(timer)
      timer = setTimeout(() => (mdText = entry.model.getValue()), Math.max(150, untrack(() => policy.debounceMs)))
    })
    return () => {
      clearTimeout(timer)
      sub.dispose()
    }
  })

  // ⌘S a livello di finestra per la scheda attiva modificabile (anche con il focus nell'albero, su una scheda o con la
  // sola anteprima in vista). Con il focus in Monaco salva il suo comando: qui niente, per non salvare due volte.
  function onWindowKey(e: KeyboardEvent) {
    if (!e.metaKey || e.ctrlKey || e.altKey || e.shiftKey || e.key.toLowerCase() !== 's') return
    if (e.defaultPrevented || (document.activeElement as Element | null)?.closest?.('.monaco-editor')) return
    const t = editor.activeTab
    if (!t || !isTextTab(t)) return
    e.preventDefault()
    void editor.save(t.path).catch(() => {})
  }

  // Monaco si carica solo quando si apre il primo file di testo
  $effect(() => {
    if (editor.tabs.some((t) => t.diff || isTextTab(t)) && !monaco) {
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
    const modelsBefore = models.size
    const live = new Set(editor.tabs.filter(isTextTab).map((t) => t.id))
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
      if (t.error || (!t.diff && !isTextTab(t))) continue
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
    if (models.size !== modelsBefore) modelTick++
    const a = editor.activeTab
    if (a && isTextTab(a)) {
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

<svelte:window onkeydown={onWindowKey} />

<div class="pane">
  <div class="tabs" role="tablist">
    {#each editor.tabs as t (t.id)}
      <div class="tab" class:on={t.key === editor.active} role="tab" use:tooltip={t.diff ? `Modifiche: ${relativeToFolders(t.path, projectStore.project?.folders ?? [])}` : relativeToFolders(t.path, projectStore.project?.folders ?? [])} aria-selected={t.key === editor.active} tabindex="0"
        onclick={() => (editor.active = t.key)} onkeydown={(e) => e.key === 'Enter' && (editor.active = t.key)}
        onauxclick={(e) => { if (e.button === 1) { e.preventDefault(); editor.requestClose(t.key) } }}>
        {#if !t.diff}{@const spec = iconFor(t.name, false, false)}<span class="ticon"><FileIcon id={spec.id} dim={spec.dim} size={14} /></span>{/if}
        <span class="tname">{t.name}</span>
        {#if t.dirty}<span class="dot" role="img" use:tooltip={{ text: 'Modifiche non salvate', shortcut: '⌘S' }}></span>{/if}
        <button class="x" aria-label="Chiudi {t.name}" use:tooltip={'Chiudi scheda'} onclick={(e) => { e.stopPropagation(); editor.requestClose(t.key) }}>×</button>
      </div>
    {/each}
    <span class="grow"></span>
    {#if editor.tabs.length > 0}
      <button class="tool" class:on={editor.split} use:tooltip={editor.split ? 'Chiudi la vista divisa' : 'Dividi l\'editor'} onclick={() => (editor.split = !editor.split)}>Dividi</button>
    {/if}
  </div>

  {#if mdActive}
    <div class="modebar" role="radiogroup" aria-label="Modo di visualizzazione">
      {#each MODES as [m, label, tip] (m)}
        <button class="seg" class:on={mdMode === m} use:tooltip={tip} role="radio" aria-checked={mdMode === m} onclick={() => editor.setMode(mdActive.path, m)}>{label}</button>
      {/each}
    </div>
  {/if}

  {#if editor.closing && active && editor.closing === active.key}
    <div class="bar" role="alert">
      <span>Modifiche non salvate in {active.name}</span>
      <button use:tooltip={{ text: 'Salva il file e chiudi la scheda', shortcut: '⌘S' }} aria-disabled={readOnly} onclick={() => !readOnly && void editor.resolveClose('save')}>Salva</button>
      <button use:tooltip={'Chiudi la scheda perdendo le modifiche'} onclick={() => void editor.resolveClose('discard')}>Scarta</button>
      <button use:tooltip={'Tieni la scheda aperta'} onclick={() => void editor.resolveClose('cancel')}>Annulla</button>
    </div>
  {/if}

  {#if active?.prompt}
    <div class="bar" role="alert">
      <span>Cambiato su disco</span>
      <button use:tooltip={'Scarta le tue modifiche e rileggi il file dal disco'} onclick={() => void editor.reload(active.path)}>Ricarica</button>
      <button use:tooltip={'Tieni la tua versione: il prossimo salvataggio sovrascrive il disco'} onclick={() => void editor.keepMine(active.path)}>Tieni il mio</button>
    </div>
  {/if}

  <div class="body">
    {#if editor.tabs.length === 0}
      <p class="empty">Apri un file dall'albero</p>
    {:else if showBinary && active}
      {#await import('./BinaryPanel.svelte') then m}
        {#key active.id}<m.default path={active.path} name={active.name} />{/key}
      {/await}
    {:else if active?.error === 'forbidden'}
      <p class="empty">Percorso fuori dal progetto</p>
    {:else if active?.error}
      <p class="empty">Impossibile aprire il file</p>
    {/if}
    <div class="editors" class:hidden={!showCode} data-testid="editors">
      <div class="ed" bind:this={mainEl}></div>
      <div class="ed" class:hidden={!editor.split || mdMode === 'split'} bind:this={splitEl}></div>
    </div>
    {#if showPreview && mdActive}
      <div class="side" class:full={mdMode === 'preview'}>
        {#if !policy.large}
          {#await import('./MarkdownPreview.svelte') then m}
            <m.default text={mdText} path={mdActive.path} />
          {/await}
        {:else}
          <div class="bigdoc">
            {#if manual && manual.id === mdActive.id}
              <div class="bigbar"><span>Documento grande: l'anteprima non si aggiorna da sola</span><button use:tooltip={'Rigenera l\'anteprima con il testo attuale'} onclick={renderNow}>Aggiorna anteprima</button></div>
              <div class="bigview">
                {#await import('./MarkdownPreview.svelte') then m}
                  <m.default text={manual.text} path={mdActive.path} />
                {/await}
              </div>
            {:else}
              <p>Documento grande: anteprima su richiesta</p>
              <button use:tooltip={'Genera l\'anteprima di questo documento grande (può richiedere tempo)'} onclick={renderNow}>Mostra anteprima</button>
            {/if}
          </div>
        {/if}
      </div>
    {/if}
    {#if showViewer && active}
      <div class="side full">
        {#key active.id}
          {#if active.viewer === 'image'}
            {#await import('./ImageViewer.svelte') then m}
              <m.default path={active.path} name={active.name} rev={active.rev} onAsText={() => void editor.open(active.path, true)} />
            {/await}
          {:else if active.viewer === 'pdf'}
            {#await import('./PdfViewer.svelte') then m}
              <m.default path={active.path} rev={active.rev} />
            {/await}
          {:else if active.viewer === 'media'}
            {#await import('./MediaViewer.svelte') then m}
              <m.default path={active.path} name={active.name} rev={active.rev} />
            {/await}
          {/if}
        {/key}
      </div>
    {/if}
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
  .body { position: relative; flex: 1; min-height: 0; display: flex; }
  .editors { display: flex; height: 100%; flex: 1; min-width: 0; }
  .side { flex: 1; min-width: 0; height: 100%; border-left: 1px solid var(--edge-soft); }
  .side.full { border-left: 0; }
  .bigdoc { height: 100%; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 10px; color: var(--dim); font-size: 13px; }
  .bigdoc p { margin: 0; }
  .bigdoc button, .bigbar button { background: var(--glass2); border: 1px solid var(--edge-soft); color: var(--txt); border-radius: 6px; padding: 3px 10px; font-size: 12px; cursor: default; }
  .bigbar { display: flex; align-items: center; gap: 10px; align-self: stretch; padding: 6px 12px; font-size: 11px; border-bottom: 1px solid var(--edge-soft); }
  .bigview { flex: 1; min-height: 0; align-self: stretch; }
  .ticon { display: inline-flex; }
  .modebar { display: inline-flex; align-self: flex-start; margin: 6px 10px; padding: 2px; gap: 2px; background: var(--glass); border: 1px solid var(--edge-soft); border-radius: 8px; flex: none; }
  .seg { background: none; border: 0; color: var(--dim); font-size: 11px; padding: 3px 10px; border-radius: 6px; cursor: default; }
  .seg.on { background: var(--glass2); color: var(--txt); }
  .ed { flex: 1; min-width: 0; height: 100%; }
  .diff { position: absolute; inset: 0; }
  .editors .ed + .ed { border-left: 1px solid var(--edge-soft); }
  .hidden { display: none; }
  .empty { position: absolute; inset: 0; display: grid; place-items: center; margin: 0; color: var(--faint); font-size: 13px; pointer-events: none; }
</style>
