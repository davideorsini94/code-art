<script lang="ts">
  import { onDestroy, onMount, untrack } from 'svelte'
  import Icon from '../Icon.svelte'
  import ResultGrid from './ResultGrid.svelte'
  import EditBar from './EditBar.svelte'
  import DocEditor from './DocEditor.svelte'
  import { tooltip } from '../../lib/tooltip'
  import { errorText } from '../../lib/errors'
  import * as ipc from '../../lib/ipc'
  import { db } from '../../lib/stores/db.svelte'
  import { editor, type Tab } from '../../lib/stores/editor.svelte'
  import { appendPage, emptyGrid, fmtInt, selectAllOracle, selectAllSql, type GridData } from '../../lib/db/grid'
  import { editCount, emptyEdits, toRowEdits, type PendingEdits } from '../../lib/db/edits'
  import type { DbEditableTable, DbHistoryEntry, DbQueryResult, PgColumn } from '../../lib/types'

  type Monaco = typeof import('monaco-editor')
  type ICode = import('monaco-editor').editor.IStandaloneCodeEditor

  let { tab }: { tab: Tab } = $props()
  // la scheda non cambia identità per tutta la vita del componente (la chiave è l'`id`)
  const q = untrack(() => tab.query!)
  const connId = q.connectionId
  const isOra = q.kind === 'oracle'
  const isSql = q.kind === 'postgres' || isOra
  const storeKey = `codeart.dbquery.${connId}.${q.persistKey}`
  const conn = $derived(db.connections.find((c) => c.id === connId))

  // ---- testo (salvato a ogni modifica) ----
  type MongoText = { database: string; collection: string; mode: 'find' | 'aggregate'; filter: string; sort: string; projection: string; pipeline: string }
  const defaultSql = q.table !== undefined ? (isOra ? selectAllOracle(q.schema ?? '', q.table) : selectAllSql(q.schema ?? 'public', q.table)) : ''
  const defaultMongo = (): MongoText => ({ database: q.database ?? '', collection: q.collection ?? '', mode: 'find', filter: '', sort: '', projection: '', pipeline: '[\n  \n]' })

  function stored(): string | null {
    try {
      return localStorage.getItem(storeKey)
    } catch {
      return null
    }
  }
  function parseMongo(raw: string): MongoText | null {
    try {
      const o = JSON.parse(raw)
      if (o && typeof o === 'object' && typeof o.collection === 'string') return { ...defaultMongo(), ...o }
    } catch {
      /* testo non valido */
    }
    return null
  }
  const saved = stored()
  let sql = $state(isSql ? (saved ?? defaultSql) : '')
  let mg = $state<MongoText>(!isSql && saved ? (parseMongo(saved) ?? defaultMongo()) : defaultMongo())
  // il testo salvato diverso da quello predefinito non si riesegue da solo (potrebbe scrivere dati)
  const restoredCustom = isSql ? saved !== null && saved !== defaultSql : saved !== null

  $effect(() => {
    const v = isSql ? sql : JSON.stringify(mg)
    try {
      localStorage.setItem(storeKey, v)
    } catch {
      /* spazio o permessi: si perde solo il ripristino */
    }
  })

  // ---- Monaco ----
  let monaco = $state.raw<Monaco | null>(null)
  const eds: Record<string, ICode> = {}
  onMount(() => {
    void import('../../lib/monaco').then((m) => (monaco = m.monaco))
  })
  onDestroy(() => {
    for (const e of Object.values(eds)) e.dispose()
  })

  /** Crea un editor Monaco nel nodo; `field` lega il testo allo stato. */
  function mount(node: HTMLElement, p: { field: string; language: string; value: string; set: (v: string) => void; compact?: boolean }) {
    const m = monaco!
    const ed = m.editor.create(node, {
      value: p.value,
      language: p.language,
      theme: 'codeart',
      automaticLayout: true,
      minimap: { enabled: false },
      fontSize: p.compact ? 12 : 13,
      fontFamily: 'ui-monospace, Menlo, monospace',
      scrollBeyondLastLine: false,
      ...(p.compact ? { lineNumbers: 'off', glyphMargin: false, folding: false, lineDecorationsWidth: 4, overviewRulerLanes: 0, renderLineHighlight: 'none', scrollbar: { vertical: 'auto', horizontal: 'hidden' } } : {}),
    } as import('monaco-editor').editor.IStandaloneEditorConstructionOptions)
    ed.onDidChangeModelContent(() => p.set(ed.getValue()))
    ed.addCommand(m.KeyMod.CtrlCmd | m.KeyCode.Enter, () => void run())
    eds[p.field] = ed
    return {
      destroy() {
        ed.dispose()
        delete eds[p.field]
      },
    }
  }
  function setField(field: string, v: string) {
    if (eds[field]) eds[field].setValue(v)
  }

  // ---- esecuzione ----
  let grid = $state.raw<GridData>(emptyGrid())
  let resultKey = $state(0)
  let resultId = $state<number | undefined>()
  let hasMore = $state(false)
  let running = $state(false)
  let paging = $state(false)
  let error = $state<string | null>(null)
  let info = $state<string | null>(null)
  let notices = $state<string[]>([])
  let affected = $state<number[] | null>(null)
  let attempted = $state(false)
  let hasGrid = $state(false)
  let runSeq = 0

  // ---- modifica in griglia (Postgres) ----
  let edits = $state.raw<PendingEdits>(emptyEdits())
  let pgCols = $state.raw<PgColumn[] | null>(null)
  // la tabella letta dal testo eseguito, decisa dal core (una sola `SELECT * FROM t` con al più WHERE/ORDER BY/LIMIT/OFFSET)
  let editTarget = $state.raw<DbEditableTable | null>(null)
  let colsSeq = 0
  function loadCols(t: DbEditableTable | null) {
    const seq = ++colsSeq
    pgCols = null
    if (!t) return
    void Promise.resolve()
      .then(() => (isOra ? ipc.dbOraColumns(connId, t.schema, t.table) : ipc.dbPgColumns(connId, t.schema, t.table)))
      .then((c) => {
        if (seq === colsSeq) pgCols = c
      })
      .catch(() => {})
  }
  const editInfo = $derived.by((): { pk?: number[]; note?: string; ro?: Set<number> } => {
    if (!isSql || !hasGrid) return {}
    if (!editTarget) return { note: 'Sola lettura: risultato di una query' }
    const et = editTarget
    const t = db.trees[connId]
    const kind = t && t.kind !== 'mongo' ? t.tree.schemas.find((x) => x.name === et.schema)?.tables.find((x) => x.name === et.table)?.kind : undefined
    if (kind === 'view') return { note: 'Sola lettura: vista' }
    if (!pgCols) return {}
    const names = pgCols.filter((c) => c.isPk).map((c) => c.name)
    if (names.length === 0) return { note: 'Sola lettura: tabella senza chiave primaria' }
    const idx = names.map((n) => grid.columns.indexOf(n))
    if (idx.includes(-1)) return { note: 'Sola lettura: il risultato non contiene la chiave primaria' }
    // Oracle: le colonne che il core non sa riscrivere (LOB, RAW, LONG, INTERVAL, TIMESTAMP WITH LOCAL TIME ZONE…)
    // restano in sola lettura; una chiave di quei tipi rende la tabella non modificabile
    if (isOra && pgCols.some((c) => c.isPk && c.editable === false)) return { note: 'Sola lettura: chiave primaria di un tipo non modificabile' }
    const ro = new Set<number>()
    if (isOra) for (const c of pgCols) if (c.editable === false) ro.add(grid.columns.indexOf(c.name))
    return { pk: idx, ro }
  })
  const pendingCount = $derived(editCount(edits))
  const rowEdits = $derived(editInfo.pk ? toRowEdits(edits, grid.columns, editInfo.pk) : [])

  // modifiche in sospeso = scheda "modificata": chiuderla chiede Applica / Scarta / Annulla
  $effect(() => {
    editor.setDirty(tab.id, pendingCount > 0)
  })
  /** Applica le modifiche in sospeso (dalla barra di chiusura): `true` se applicate. */
  async function applyPending(): Promise<boolean> {
    if (!editTarget || rowEdits.length === 0) return pendingCount === 0
    error = null
    info = null
    try {
      await (isOra ? ipc.dbOraApplyEdits : ipc.dbPgApplyEdits)(connId, editTarget.schema, editTarget.table, rowEdits)
      edits = emptyEdits()
      return true
    } catch (e) {
      const msg = errorText(e)
      if (/annullat/i.test(msg)) info = 'Applicazione annullata: le modifiche restano in sospeso'
      else error = msg
      return false
    }
  }
  editor.setApplier(untrack(() => tab.key), applyPending)
  onDestroy(() => editor.setApplier(untrack(() => tab.key), undefined))

  // ---- documenti MongoDB ----
  let docs = $state.raw<(string | null)[]>([])
  let ids = $state.raw<(string | null)[]>([])
  let target = $state({ database: '', collection: '', find: false })
  let docEdit = $state<{ doc: string | null; id: string | null; database: string; collection: string } | null>(null)
  const canOpenDoc = $derived(!isSql && target.find && hasGrid)
  function openDoc(r: number) {
    if (docs[r] === undefined) return
    if (docs[r] === null) {
      info = 'Documento più grande di 1 MB: non si modifica da qui'
      return
    }
    docEdit = { doc: docs[r], id: ids[r] ?? null, database: target.database, collection: target.collection }
  }
  function newDoc() {
    if (mg.collection) docEdit = { doc: null, id: null, database: mg.database, collection: mg.collection }
  }
  function docDone() {
    docEdit = null
    void run()
  }
  async function applied() {
    edits = emptyEdits()
    await run()
  }

  const offline = $derived(db.stateOf(connId) !== 'connected')
  const showClosed = $derived(attempted && offline)

  const mongoOptions = $derived.by(() => {
    const t = db.trees[connId]
    const list = t?.kind === 'mongo' ? t.dbs.flatMap((d) => d.collections.map((c) => ({ database: d.database, collection: c }))) : []
    if (mg.collection && !list.some((o) => o.database === mg.database && o.collection === mg.collection)) list.unshift({ database: mg.database, collection: mg.collection })
    return list
  })

  function setResult(res: DbQueryResult) {
    const hasRows = res.resultId !== undefined || res.columns.length > 0
    hasGrid = hasRows
    grid = hasRows ? appendPage(emptyGrid(), res, !isSql) : emptyGrid()
    resultKey++
    edits = emptyEdits()
    docs = res.docs ?? []
    ids = res.ids ?? []
    if (isSql) {
      editTarget = res.editableTable ?? null
      loadCols(editTarget)
    }
    resultId = res.resultId
    editor.setQueryResult(tab.key, res.resultId)
    hasMore = res.hasMore
    notices = res.notices ?? []
    affected = hasRows ? null : (res.affected ?? [])
  }

  async function run() {
    if (running) return
    info = null
    if (offline) {
      attempted = true
      return
    }
    let text: string
    if (isSql) {
      text = sql
      if (!text.trim()) return
    } else {
      if (!mg.collection) {
        error = 'Scegli una collezione'
        return
      }
      text = JSON.stringify(mg)
    }
    attempted = false
    running = true
    error = null
    const seq = ++runSeq
    try {
      const res = isSql
        ? await ipc.dbQuery(connId, text)
        : mg.mode === 'find'
          ? await ipc.dbMongoFind(connId, mg.database, mg.collection, mg.filter, mg.sort, mg.projection)
          : await ipc.dbMongoAggregate(connId, mg.database, mg.collection, mg.pipeline)
      void Promise.resolve(ipc.dbHistoryAdd(connId, text)).catch(() => {})
      if (seq === runSeq) {
        target = { database: mg.database, collection: mg.collection, find: !isSql && mg.mode === 'find' }
        setResult(res)
      }
    } catch (e) {
      const msg = errorText(e)
      // "annullato": l'utente ha rifiutato la conferma o ha annullato la query: niente di allarmante
      if (/annullat/i.test(msg)) info = 'Esecuzione annullata'
      else error = msg
    } finally {
      running = false
    }
  }

  async function cancel() {
    try {
      await ipc.dbQueryCancel(connId)
    } catch (e) {
      error = errorText(e)
    }
  }

  async function more() {
    if (resultId === undefined || paging || !hasMore) return
    const id = resultId
    paging = true
    try {
      const page = await ipc.dbQueryMore(id)
      if (id !== resultId) return
      grid = appendPage(grid, page, !isSql)
      if (page.docs) docs = docs.concat(page.docs)
      if (page.ids) ids = ids.concat(page.ids)
      hasMore = page.hasMore
      if (page.notices?.length) notices = [...notices, ...page.notices]
    } catch (e) {
      // un errore di un risultato ormai sostituito (riesecuzione durante la pagina) non riguarda quello mostrato
      if (id !== resultId) return
      error = errorText(e)
      // scaduto o chiuso per inattività: niente altre richieste, si riesegue
      if (/riesegui/i.test(error)) hasMore = false
    } finally {
      paging = false
    }
  }

  async function doExport(format: 'csv' | 'json') {
    if (resultId === undefined) return
    try {
      const out = await ipc.dbExport(resultId, format)
      if (!out) return
      // il core ha letto fino in fondo per esportare ma tiene il risultato: si può riesportare, non sfogliare oltre
      info = `Esportate ${fmtInt(out.rows)} righe in ${out.path}. Il risultato è stato letto fino in fondo: per vedere altre righe nella griglia riesegui la query.`
      hasMore = false
    } catch (e) {
      error = errorText(e)
    }
  }

  // apertura da tabella/collezione: si esegue una volta (mai un testo ripristinato diverso dal predefinito)
  onMount(() => {
    if (!q.autoRun) return
    q.autoRun = false
    if (restoredCustom) return
    void run()
  })

  // ---- cronologia ----
  let histOpen = $state(false)
  let history = $state<DbHistoryEntry[]>([])
  async function toggleHistory() {
    histOpen = !histOpen
    if (!histOpen) return
    try {
      history = (await ipc.dbHistory(connId)).slice().sort((a, b) => b.atMs - a.atMs).slice(0, 200)
    } catch (e) {
      error = errorText(e)
    }
  }
  function historyLabel(text: string) {
    if (!isSql) {
      const m = parseMongo(text)
      if (m) return `${m.collection} · ${m.mode === 'find' ? (m.filter.trim() || '{}') : m.pipeline.replace(/\s+/g, ' ').trim()}`
    }
    return text.replace(/\s+/g, ' ').trim()
  }
  function pick(h: DbHistoryEntry) {
    histOpen = false
    if (isSql) {
      sql = h.text
      setField('sql', h.text)
      return
    }
    const m = parseMongo(h.text)
    if (!m) return
    mg = m
    setField('filter', m.filter)
    setField('sort', m.sort)
    setField('projection', m.projection)
    setField('pipeline', m.pipeline)
  }

  function pickCollection(v: string) {
    const [database, collection] = JSON.parse(v) as [string, string]
    mg.database = database
    mg.collection = collection
  }

  // ---- divisore ----
  let topH = $state(240)
  function dragSplit(e: PointerEvent) {
    e.preventDefault()
    const el = e.currentTarget as HTMLElement
    const y0 = e.clientY
    const h0 = topH
    el.setPointerCapture?.(e.pointerId)
    const move = (ev: PointerEvent) => (topH = Math.min(600, Math.max(90, h0 + ev.clientY - y0)))
    const up = () => {
      el.removeEventListener('pointermove', move)
      el.removeEventListener('pointerup', up)
    }
    el.addEventListener('pointermove', move)
    el.addEventListener('pointerup', up)
  }
  function splitKey(e: KeyboardEvent) {
    if (e.key === 'ArrowUp') topH = Math.max(90, topH - 20)
    else if (e.key === 'ArrowDown') topH = Math.min(600, topH + 20)
  }

  function onkey(e: KeyboardEvent) {
    if ((e.metaKey || e.ctrlKey) && e.key === 'Enter') {
      e.preventDefault()
      void run()
    }
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="qt" data-no-tooltip="contenitore: la scorciatoia ⌘↵ vale per tutta la scheda" onkeydown={onkey}>
  <div class="bar">
    <button class="run" disabled={running} use:tooltip={{ text: 'Esegui', shortcut: '⌘↵' }} onclick={() => void run()}><Icon name="play" size={13} /> Esegui</button>
    {#if running}
      <button use:tooltip={'Interrompe la query in corso'} onclick={() => void cancel()}>Annulla</button>
      <span class="dim" role="status">In esecuzione…</span>
    {/if}
    <span class="grow"></span>
    <span class="dim cn">{conn?.name ?? 'Connessione'}</span>
    {#if !isSql}<button disabled={!mg.collection || mg.mode !== 'find'} use:tooltip={'Inserisce un nuovo documento nella collezione scelta'} onclick={newDoc}>Nuovo documento</button>{/if}
    <button class:on={histOpen} aria-expanded={histOpen} use:tooltip={'Ultime 200 query eseguite su questa connessione'} onclick={() => void toggleHistory()}><Icon name="history" size={13} /> Cronologia</button>
    <button disabled={resultId === undefined} use:tooltip={'Esporta il risultato in CSV (tutte le righe, anche non caricate)'} onclick={() => void doExport('csv')}><Icon name="download" size={13} /> CSV</button>
    <button disabled={resultId === undefined} use:tooltip={'Esporta il risultato in JSON (tutte le righe, anche non caricate)'} onclick={() => void doExport('json')}><Icon name="download" size={13} /> JSON</button>
  </div>

  {#if histOpen}
    <div class="hist" role="listbox" aria-label="Cronologia">
      {#each history as h (h.id)}
        <button class="hitem" role="option" aria-selected="false" use:tooltip={'Sostituisce il testo dell\'editor con questa query'} onclick={() => pick(h)}>{historyLabel(h.text)}</button>
      {:else}
        <p class="dim pad">Nessuna query nella cronologia</p>
      {/each}
    </div>
  {/if}

  <div class="top" style="height: {topH}px">
    {#if isSql}
      {#if monaco}<div class="ed" use:mount={{ field: 'sql', language: 'sql', value: sql, set: (v) => (sql = v) }}></div>{:else}<p class="dim pad">Caricamento editor…</p>{/if}
    {:else}
      <div class="mform">
        <select aria-label="Collezione" use:tooltip={'Collezione su cui eseguire la ricerca'} value={JSON.stringify([mg.database, mg.collection])} onchange={(e) => pickCollection(e.currentTarget.value)}>
          {#if !mg.collection}<option value={JSON.stringify(['', ''])}>Scegli una collezione…</option>{/if}
          {#each mongoOptions as o (o.database + '.' + o.collection)}
            <option value={JSON.stringify([o.database, o.collection])}>{o.database}.{o.collection}</option>
          {/each}
        </select>
        <div class="seg" role="radiogroup" aria-label="Modo">
          <button role="radio" aria-checked={mg.mode === 'find'} class:on={mg.mode === 'find'} use:tooltip={'Cerca documenti con filtro, ordinamento e proiezione'} onclick={() => (mg.mode = 'find')}>Ricerca</button>
          <button role="radio" aria-checked={mg.mode === 'aggregate'} class:on={mg.mode === 'aggregate'} use:tooltip={'Esegue una pipeline di aggregazione'} onclick={() => (mg.mode = 'aggregate')}>Aggregazione</button>
        </div>
      </div>
      {#if monaco}
        {#if mg.mode === 'find'}
          <div class="trio">
            {#each [['filter', 'Filtro', '{ "campo": "valore" }'], ['sort', 'Ordinamento', '{ "campo": 1 }'], ['projection', 'Proiezione', '{ "campo": 1 }']] as [f, label, hint] (f)}
              <div class="field">
                <span class="lbl">{label} <span class="faint">{hint}</span></span>
                <div class="ed" use:mount={{ field: f, language: 'json', value: mg[f as 'filter' | 'sort' | 'projection'], compact: true, set: (v) => (mg[f as 'filter' | 'sort' | 'projection'] = v) }}></div>
              </div>
            {/each}
          </div>
        {:else}
          <div class="field grow1"><span class="lbl">Pipeline <span class="faint">array JSON di stadi</span></span><div class="ed" use:mount={{ field: 'pipeline', language: 'json', value: mg.pipeline, set: (v) => (mg.pipeline = v) }}></div></div>
        {/if}
      {:else}<p class="dim pad">Caricamento editor…</p>{/if}
    {/if}
  </div>

  <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
  <div class="split" role="separator" aria-orientation="horizontal" aria-label="Ridimensiona editor e risultati" tabindex="0" data-no-tooltip="divisore trascinabile" onpointerdown={dragSplit} onkeydown={splitKey}></div>

  <div class="res">
    {#if showClosed}
      <div class="msg warn" role="alert">Connessione chiusa · <button class="link" use:tooltip={'Riapre la connessione: il testo resta'} onclick={() => void db.connect(connId)}>Riconnetti</button></div>
    {/if}
    {#if error}<div class="msg err" role="alert">{error}</div>{/if}
    {#if info}<div class="msg" role="status">{info}</div>{/if}
    {#each notices as n}<div class="msg note" role="status">{n}</div>{/each}
    {#if hasGrid}
      {#if isSql && editInfo.pk && editTarget && pendingCount > 0}
        <EditBar {connId} oracle={isOra} schema={editTarget!.schema} table={editTarget!.table} edits={rowEdits} count={pendingCount} onapplied={() => void applied()} onclear={() => (edits = emptyEdits())} />
      {/if}
      {#key resultKey}
        <ResultGrid columns={grid.columns} rows={grid.rows} truncated={grid.truncated} {hasMore} loading={paging} onmore={() => void more()} pk={editInfo.pk} readOnlyCols={editInfo.ro} {edits} onedit={(e) => (edits = e)} note={editInfo.note} onopen={canOpenDoc ? openDoc : undefined} />
      {/key}
    {:else if affected}
      <ul class="aff">
        {#if affected.length === 0}<li>Eseguito, nessun risultato</li>{/if}
        {#each affected as n, i}<li>{affected.length > 1 ? `Istruzione ${i + 1}: ` : ''}{fmtInt(n)} {n === 1 ? 'riga interessata' : 'righe interessate'}</li>{/each}
      </ul>
    {:else if !running && !error && !showClosed}
      <p class="dim pad">Esegui una query per vedere i risultati</p>
    {/if}
  </div>
  {#if docEdit}
    <DocEditor {connId} database={docEdit.database} collection={docEdit.collection} doc={docEdit.doc} idJson={docEdit.id} onclose={() => (docEdit = null)} ondone={docDone} />
  {/if}
</div>

<style>
  .qt { position: relative; display: flex; flex-direction: column; height: 100%; min-height: 0; }
  .bar { display: flex; align-items: center; gap: 6px; padding: 6px 8px; flex: none; border-bottom: 1px solid var(--edge-soft); }
  .bar button, .mform select, .seg button { display: inline-flex; align-items: center; gap: 4px; font-size: 12px; padding: 3px 9px; border-radius: 6px; border: 1px solid var(--edge-soft); background: transparent; color: inherit; cursor: pointer; }
  .bar button:disabled { opacity: 0.5; cursor: default; }
  .bar button.on, .seg button.on { background: var(--edge-soft); }
  .run { color: var(--work, #4ade80); }
  .grow { flex: 1; }
  .dim { color: var(--dim); font-size: 12px; }
  .faint { color: var(--faint); font-weight: 400; }
  .cn { max-width: 180px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .pad { padding: 8px 10px; margin: 0; }
  .hist { position: absolute; right: 8px; top: 38px; z-index: 20; width: min(520px, 90%); max-height: 320px; overflow: auto; background: var(--panel, #1e1e22); border: 1px solid var(--edge-soft); border-radius: 8px; box-shadow: 0 8px 24px rgba(0, 0, 0, 0.4); }
  .hitem { display: block; width: 100%; text-align: left; padding: 6px 10px; font: 12px ui-monospace, Menlo, monospace; background: transparent; border: 0; border-bottom: 1px solid var(--edge-soft); color: inherit; cursor: pointer; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .hitem:hover { background: var(--edge-soft); }
  .top { flex: none; display: flex; flex-direction: column; min-height: 0; }
  .ed { flex: 1; min-height: 0; }
  .mform { display: flex; align-items: center; gap: 8px; padding: 6px 8px; flex: none; }
  .seg { display: inline-flex; }
  .seg button { border-radius: 0; }
  .seg button:first-child { border-radius: 6px 0 0 6px; }
  .seg button:last-child { border-radius: 0 6px 6px 0; border-left: 0; }
  .trio { display: flex; gap: 8px; padding: 0 8px 6px; flex: 1; min-height: 0; }
  .field { display: flex; flex-direction: column; flex: 1; min-width: 0; min-height: 0; padding: 0 8px 6px; }
  .trio .field { padding: 0; }
  .grow1 { flex: 1; }
  .lbl { font-size: 11px; color: var(--dim); padding: 0 0 2px; }
  .split { flex: none; height: 6px; cursor: row-resize; background: var(--edge-soft); }
  .split:hover, .split:focus-visible { background: var(--accent, #6aa9ff); outline: none; }
  .res { flex: 1; min-height: 0; display: flex; flex-direction: column; }
  .res > :global(.grid) { flex: 1; }
  .msg { padding: 6px 10px; font-size: 12px; border-bottom: 1px solid var(--edge-soft); flex: none; white-space: pre-wrap; }
  .msg.err { color: var(--err, #f87171); }
  .msg.warn { color: var(--wait, #fbbf24); }
  .msg.note { color: var(--dim); }
  .link { background: none; border: 0; padding: 0; color: var(--accent, #6aa9ff); text-decoration: underline; cursor: pointer; font: inherit; }
  .aff { margin: 0; padding: 10px 24px; font-size: 12.5px; }
</style>
