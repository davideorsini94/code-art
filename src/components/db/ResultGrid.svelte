<script lang="ts">
  import { tick } from 'svelte'
  import { tooltip } from '../../lib/tooltip'
  import { addInsert, currentValue, isChanged, isDeleted, removeInsert, setCell, setInsertCell, toggleDelete, emptyEdits, type PendingEdits } from '../../lib/db/edits'
  import { MAX_ROWS, PAGE, ROWNUM_W, ROW_H, cellKey, clampWidth, estimateWidth, fmtInt, nearEnd, visibleColumns, visibleRows, type Cell } from '../../lib/db/grid'

  let {
    columns,
    rows,
    truncated = new Set<string>(),
    hasMore = false,
    loading = false,
    onmore,
    pk,
    readOnlyCols,
    edits = emptyEdits(),
    onedit,
    note,
    onopen,
    openLabel = 'Modifica documento',
  }: {
    columns: string[]
    rows: Cell[][]
    truncated?: Set<string>
    hasMore?: boolean
    loading?: boolean
    onmore?: () => void
    /** Indici delle colonne della chiave primaria: presente = griglia modificabile. */
    pk?: number[]
    /** Colonne (per indice) che non si modificano. */
    readOnlyCols?: Set<number>
    edits?: PendingEdits
    onedit?: (e: PendingEdits) => void
    /** Motivo della sola lettura, mostrato in fondo. */
    note?: string
    /** Apre la riga (indice tra le righe lette) in un editor (MongoDB). */
    onopen?: (row: number) => void
    openLabel?: string
  } = $props()

  const canEdit = $derived(pk !== undefined && pk.length > 0 && !!onedit)
  const ni = $derived(canEdit ? edits.inserts.length : 0)
  const total = $derived(ni + rows.length)
  /** Cella in modifica: `initial` è il valore di partenza (NULL, DEFAULT o testo), `touched` = l'utente ha scritto. */
  let editing = $state<{ r: number; c: number; text: string; initial: Cell | undefined; touched: boolean } | null>(null)
  let menu = $state<{ x: number; y: number; r: number } | null>(null)

  /** Valore mostrato (`undefined` = valore predefinito di una riga nuova). */
  function val(r: number, c: number): Cell | undefined {
    if (r < ni) return edits.inserts[r][c]
    const row = rows[r - ni]
    return canEdit ? currentValue(edits, row, pk!, c) : (row?.[c] ?? null)
  }
  const rowState = (r: number) => (!canEdit ? '' : r < ni ? 'ins' : isDeleted(edits, rows[r - ni], pk!) ? 'del' : '')
  const changed = (r: number, c: number) => canEdit && r >= ni && isChanged(edits, rows[r - ni], pk!, c)

  /** Cella troncata a 64 KB (righe lette): sola lettura, riscriverla salverebbe il valore accorciato. */
  const isTrunc = (r: number, c: number) => r >= ni && truncated.has(cellKey(r - ni, c))
  function startEdit(r: number, c: number) {
    if (!canEdit || rowState(r) === 'del' || isTrunc(r, c) || readOnlyCols?.has(c)) return
    sel = { r, c }
    const initial = val(r, c)
    editing = { r, c, text: initial ?? '', initial, touched: false }
  }
  /** Imposta il valore (anche NULL, col pulsante ∅ o ⌘⌫). */
  function commit(value: string | null) {
    const ed = editing
    if (!ed || !canEdit) return
    editing = null
    onedit!(ed.r < ni ? setInsertCell(edits, ed.r, ed.c, value) : setCell(edits, rows[ed.r - ni], pk!, ed.c, value))
  }
  /** Invio, Tab, clic altrove: si registra solo un testo cambiato (NULL e DEFAULT non toccati restano tali). */
  function commitTyped() {
    const ed = editing
    if (!ed) return
    const unchanged = !ed.touched || (typeof ed.initial === 'string' && ed.text === ed.initial)
    if (unchanged) editing = null
    else commit(ed.text)
  }
  function delRow(r: number) {
    if (!canEdit) return
    onedit!(r < ni ? removeInsert(edits, r) : toggleDelete(edits, rows[r - ni], pk!))
  }
  function editKey(e: KeyboardEvent) {
    e.stopPropagation()
    if (!editing) return
    if (e.key === 'Enter' && !e.shiftKey && !e.altKey) {
      // Maiusc+Invio (o ⌥Invio) va a capo nel testo
      e.preventDefault()
      commitTyped()
    } else if (e.key === 'Enter' && e.altKey) {
      e.preventDefault()
      const ta = e.currentTarget as HTMLTextAreaElement
      const at = ta.selectionStart ?? editing.text.length
      editing.text = editing.text.slice(0, at) + '\n' + editing.text.slice(ta.selectionEnd ?? at)
      editing.touched = true
    } else if (e.key === 'Escape') {
      e.preventDefault()
      editing = null
    } else if (e.key === 'Tab') {
      e.preventDefault()
      const { r, c } = editing
      commitTyped()
      if (c + 1 < columns.length) startEdit(r, c + 1)
    } else if ((e.metaKey || e.ctrlKey) && e.key === 'Backspace') {
      e.preventDefault()
      commit(null)
    }
  }
  const focusInput = (n: HTMLTextAreaElement) => {
    n.focus()
    n.select()
  }

  let viewport = $state<HTMLDivElement>()
  let scrollTop = $state(0)
  let scrollLeft = $state(0)
  let vh = $state(0)
  let vw = $state(0)
  let sel = $state<{ r: number; c: number } | null>(null)
  // larghezze scelte con il trascinamento (per indice di colonna); le altre si stimano dal contenuto
  let custom = $state<Record<number, number>>({})
  // stima fissata alla prima pagina: non cambia caricando altre righe (niente colonne che "ballano")
  const estimated = new Map<number, number>()

  const widths = $derived(
    columns.map((name, i) => {
      if (custom[i] !== undefined) return custom[i]
      let e = estimated.get(i)
      if (e === undefined) {
        e = estimateWidth(name, rows, i)
        if (rows.length > 0) estimated.set(i, e)
      }
      return e
    }),
  )
  const H = $derived(vh || 400)
  const W = $derived(vw || 800)
  const win = $derived(visibleRows(scrollTop, H, total))
  const cols = $derived(visibleColumns(scrollLeft, Math.max(0, W - ROWNUM_W), widths))
  const atLimit = $derived(rows.length >= MAX_ROWS)

  // una sola richiesta per lunghezza: finché le righe non crescono, altri scorrimenti non ne chiedono un'altra
  let requestedAt = -1
  function maybeMore() {
    if (!hasMore || loading || atLimit || !onmore) return
    if (!nearEnd(visibleRows(scrollTop, H, total, 0).end, total)) return
    if (requestedAt === rows.length) return
    requestedAt = rows.length
    onmore()
  }
  function ask() {
    if (!hasMore || loading || atLimit || !onmore) return
    requestedAt = rows.length
    onmore()
  }
  function onscroll() {
    if (!viewport) return
    scrollTop = viewport.scrollTop
    scrollLeft = viewport.scrollLeft
    maybeMore()
  }
  // un risultato corto che non riempie la finestra: chiede subito la pagina dopo
  $effect(() => {
    rows.length
    vh
    maybeMore()
  })

  function startResize(e: PointerEvent, i: number) {
    e.preventDefault()
    e.stopPropagation()
    const target = e.currentTarget as HTMLElement
    const x0 = e.clientX
    const w0 = widths[i]
    target.setPointerCapture?.(e.pointerId)
    const move = (ev: PointerEvent) => (custom[i] = clampWidth(w0 + ev.clientX - x0))
    const up = () => {
      target.removeEventListener('pointermove', move)
      target.removeEventListener('pointerup', up)
      target.removeEventListener('pointercancel', up)
    }
    target.addEventListener('pointermove', move)
    target.addEventListener('pointerup', up)
    target.addEventListener('pointercancel', up)
  }

  async function onkey(e: KeyboardEvent) {
    if (!sel) return
    if (canEdit) {
      if ((e.metaKey || e.ctrlKey) && e.key === 'Backspace') {
        e.preventDefault()
        startEdit(sel.r, sel.c)
        commit(null)
        return
      }
      if (e.key === 'Delete' || e.key === 'Backspace') {
        e.preventDefault()
        delRow(sel.r)
        return
      }
      if (e.key === 'Enter' || e.key === 'F2') {
        e.preventDefault()
        startEdit(sel.r, sel.c)
        return
      }
    } else if (onopen && e.key === 'Enter' && sel.r < rows.length) {
      e.preventDefault()
      onopen(sel.r)
      return
    }
    if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'c') {
      e.preventDefault()
      try {
        await navigator.clipboard.writeText(val(sel.r, sel.c) ?? '')
      } catch {
        /* appunti non disponibili */
      }
      return
    }
    const d = { ArrowUp: [-1, 0], ArrowDown: [1, 0], ArrowLeft: [0, -1], ArrowRight: [0, 1] }[e.key]
    if (!d) return
    e.preventDefault()
    const r = Math.min(total - 1, Math.max(0, sel.r + d[0]))
    const c = Math.min(columns.length - 1, Math.max(0, sel.c + d[1]))
    sel = { r, c }
    await tick()
    if (!viewport) return
    if (r * ROW_H < viewport.scrollTop) viewport.scrollTop = r * ROW_H
    else if ((r + 1) * ROW_H > viewport.scrollTop + H - ROW_H) viewport.scrollTop = (r + 2) * ROW_H - H
  }

  const sizeObs = (node: HTMLElement) => {
    const apply = () => {
      vh = node.clientHeight
      vw = node.clientWidth
    }
    apply()
    if (typeof ResizeObserver === 'undefined') return
    const ro = new ResizeObserver(apply)
    ro.observe(node)
    return { destroy: () => ro.disconnect() }
  }
  const visibleIdx = $derived(Array.from({ length: cols.end - cols.start }, (_, k) => cols.start + k))
  const rowIdx = $derived(Array.from({ length: win.end - win.start }, (_, k) => win.start + k))
</script>

<div class="grid" role="grid" data-no-tooltip="tabella: i controlli sono nelle celle" aria-rowcount={total + 1} aria-colcount={columns.length} tabindex="0" onkeydown={onkey}>
  <div class="viewport" bind:this={viewport} {onscroll} use:sizeObs>
    <div class="inner" style="width: {ROWNUM_W + cols.total}px">
      <div class="head" role="row" style="height: {ROW_H}px">
        <div class="num hnum" role="columnheader"></div>
        {#each visibleIdx as i (i)}
          <div class="hcell" role="columnheader" style="left: {ROWNUM_W + cols.offsets[i]}px; width: {widths[i]}px">
            <span class="txt">{columns[i]}</span>
            <!-- svelte-ignore a11y_no_static_element_interactions -->
            <span class="grip" aria-hidden="true" data-no-tooltip="maniglia di trascinamento" onpointerdown={(e) => startResize(e, i)}></span>
          </div>
        {/each}
      </div>
      <div class="body" style="height: {total * ROW_H}px">
        {#each rowIdx as r (r)}
          {@const rs = rowState(r)}
          <!-- svelte-ignore a11y_no_static_element_interactions -->
          <div class="row" class:ins={rs === 'ins'} class:del={rs === 'del'} role="row" tabindex="-1" data-no-tooltip="riga: il menu contestuale ha le azioni" oncontextmenu={(e) => { if (canEdit || (onopen && r >= ni)) { e.preventDefault(); sel = { r, c: sel?.c ?? 0 }; menu = { x: e.clientX, y: e.clientY, r } } }} style="top: {r * ROW_H}px; height: {ROW_H}px">
            <div class="num" role="rowheader">{r + 1}</div>
            {#each visibleIdx as c (c)}
              {@const v = val(r, c)}
              {@const isEd = editing?.r === r && editing?.c === c}
              <!-- svelte-ignore a11y_click_events_have_key_events -->
              <div class="cell" data-no-tooltip="cella selezionabile" class:sel={sel?.r === r && sel?.c === c} class:chg={changed(r, c)} class:editing={isEd} role="gridcell" tabindex="-1" aria-selected={sel?.r === r && sel?.c === c} style="left: {ROWNUM_W + cols.offsets[c]}px; width: {widths[c]}px" onclick={() => (sel = { r, c })} ondblclick={() => (canEdit ? startEdit(r, c) : onopen && r >= ni ? onopen(r - ni) : undefined)}>
                {#if isEd}
                  <textarea class="inp" aria-label="Valore della cella" rows={Math.min(8, editing!.text.split('\n').length)} bind:value={editing!.text} use:focusInput oninput={() => (editing!.touched = true)} onkeydown={editKey} onblur={commitTyped} use:tooltip={'Invio conferma, Maiusc+Invio va a capo, Esc annulla'}></textarea>
                  <button class="nullbtn" aria-label="Imposta NULL" use:tooltip={{ text: 'Imposta NULL', shortcut: '⌘⌫' }} onmousedown={(e) => e.preventDefault()} onclick={() => commit(null)}>∅</button>
                {:else if v === undefined}<span class="null">DEFAULT</span>
                {:else if v === null}<span class="null">NULL</span>{:else}<span class="txt">{v}</span>{/if}
                {#if isTrunc(r, c)}<span class="trunc" role="img" aria-label="Valore troncato a 64 KB" use:tooltip={canEdit ? 'Valore troncato a 64 KB: sola lettura (modificarlo salverebbe il valore accorciato)' : 'Valore troncato a 64 KB'}>…</span>{/if}
              </div>
            {/each}
          </div>
        {/each}
      </div>
    </div>
  </div>
  <div class="foot" role="status">
    <span>{fmtInt(rows.length)} {rows.length === 1 ? 'riga' : 'righe'}</span>
    {#if canEdit}<button class="more addrow" use:tooltip={'Aggiunge una riga nuova in cima'} onclick={() => { onedit!(addInsert(edits, columns.length)); sel = { r: 0, c: 0 } }}>＋ Riga</button>{/if}
    {#if note}<span class="note" data-no-tooltip="nota">· {note}</span>{/if}
    {#if atLimit}
      <span class="note">· Limite di {fmtInt(MAX_ROWS)} righe raggiunto</span>
    {:else if hasMore}
      <span class="note">· Altre righe disponibili</span>
      <button class="more" disabled={loading} use:tooltip={'Legge la pagina successiva del risultato'} onclick={ask}>{loading ? 'Caricamento…' : `Carica altre ${PAGE}`}</button>
    {/if}
  </div>
  {#if menu}
    <div class="menu" role="menu" style="left: {menu.x}px; top: {menu.y}px">
      {#if canEdit}
        <button role="menuitem" use:tooltip={'Segna la riga come eliminata (si applica con «Applica»)'} onclick={() => { delRow(menu!.r); menu = null }}>{rowState(menu.r) === 'del' ? 'Ripristina riga' : 'Elimina riga'}</button>
      {:else if onopen}
        <button role="menuitem" use:tooltip={'Apre il documento in un editor'} onclick={() => { onopen(menu!.r); menu = null }}>{openLabel}</button>
      {/if}
    </div>
  {/if}
</div>
<svelte:window onclick={() => (menu = null)} />

<style>
  .grid { display: flex; flex-direction: column; min-height: 0; height: 100%; font-size: 12px; outline: none; }
  .viewport { flex: 1; min-height: 0; overflow: auto; position: relative; }
  .inner { position: relative; min-width: 100%; }
  .head { position: sticky; top: 0; z-index: 3; background: var(--panel, #1e1e22); border-bottom: 1px solid var(--edge-soft); }
  .body { position: relative; }
  .row { position: absolute; left: 0; right: 0; border-bottom: 1px solid var(--edge-soft); }
  .num { position: sticky; left: 0; z-index: 2; width: 56px; height: 100%; box-sizing: border-box; padding: 0 8px; text-align: right; color: var(--faint); background: var(--panel, #1e1e22); line-height: 23px; font-variant-numeric: tabular-nums; border-right: 1px solid var(--edge-soft); }
  .hnum { position: absolute; left: 0; top: 0; }
  .hcell, .cell { position: absolute; top: 0; height: 100%; box-sizing: border-box; padding: 0 8px; line-height: 23px; white-space: nowrap; overflow: hidden; display: flex; align-items: center; gap: 4px; }
  .hcell { font-weight: 600; color: var(--dim); border-right: 1px solid var(--edge-soft); }
  .txt { overflow: hidden; text-overflow: ellipsis; min-width: 0; }
  .cell { border-right: 1px solid var(--edge-soft); font-family: ui-monospace, Menlo, monospace; }
  .cell.sel { outline: 1px solid var(--accent, #6aa9ff); outline-offset: -1px; background: color-mix(in srgb, var(--accent, #6aa9ff) 14%, transparent); }
  .row.ins { background: color-mix(in srgb, #4ade80 14%, transparent); }
  .row.del { background: color-mix(in srgb, #f87171 16%, transparent); text-decoration: line-through; color: var(--dim); }
  .cell.chg { background: color-mix(in srgb, #fbbf24 22%, transparent); }
  .inp { position: absolute; z-index: 6; left: 4px; right: 30px; top: 2px; min-height: 20px; box-sizing: border-box; font: inherit; line-height: 17px; color: inherit; background: var(--panel, #1e1e22); border: 1px solid var(--accent, #6aa9ff); border-radius: 4px; padding: 1px 4px; resize: none; white-space: pre; overflow: auto; }
  .cell.editing { overflow: visible; z-index: 5; }
  .cell.editing .nullbtn { position: absolute; right: 4px; top: 3px; }
  .nullbtn { flex: none; font-size: 11px; line-height: 16px; padding: 0 5px; border-radius: 4px; border: 1px solid var(--edge-soft); background: transparent; color: inherit; cursor: pointer; }
  .menu { position: fixed; z-index: 50; min-width: 150px; background: var(--panel, #1e1e22); border: 1px solid var(--edge-soft); border-radius: 8px; box-shadow: 0 8px 24px rgba(0, 0, 0, 0.4); padding: 4px; }
  .menu button { display: block; width: 100%; text-align: left; font-size: 12px; padding: 5px 10px; background: transparent; border: 0; border-radius: 5px; color: inherit; cursor: pointer; }
  .menu button:hover { background: var(--edge-soft); }
  .more.addrow { margin-left: 8px; }
  .null { color: var(--faint); font-style: italic; }
  .trunc { flex: none; padding: 0 4px; border-radius: 4px; background: var(--edge-soft); color: var(--dim); line-height: 14px; }
  .grip { position: absolute; right: -3px; top: 0; bottom: 0; width: 7px; cursor: col-resize; z-index: 4; }
  .grip:hover { background: color-mix(in srgb, var(--accent, #6aa9ff) 40%, transparent); }
  .foot { flex: none; display: flex; align-items: center; gap: 6px; padding: 4px 10px; border-top: 1px solid var(--edge-soft); color: var(--dim); font-size: 11.5px; }
  .note { color: var(--faint); }
  .more { margin-left: auto; font-size: 11.5px; padding: 2px 8px; border-radius: 6px; border: 1px solid var(--edge-soft); background: transparent; color: inherit; cursor: pointer; }
  .more:disabled { opacity: 0.6; cursor: default; }
</style>
