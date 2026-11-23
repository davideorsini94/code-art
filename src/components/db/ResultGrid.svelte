<script lang="ts">
  import { tick } from 'svelte'
  import { tooltip } from '../../lib/tooltip'
  import { MAX_ROWS, PAGE, ROWNUM_W, ROW_H, cellKey, cellValue, clampWidth, estimateWidth, fmtInt, nearEnd, visibleColumns, visibleRows, type Cell } from '../../lib/db/grid'

  let {
    columns,
    rows,
    truncated = new Set<string>(),
    hasMore = false,
    loading = false,
    onmore,
  }: { columns: string[]; rows: Cell[][]; truncated?: Set<string>; hasMore?: boolean; loading?: boolean; onmore?: () => void } = $props()

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
  const win = $derived(visibleRows(scrollTop, H, rows.length))
  const cols = $derived(visibleColumns(scrollLeft, Math.max(0, W - ROWNUM_W), widths))
  const atLimit = $derived(rows.length >= MAX_ROWS)

  // una sola richiesta per lunghezza: finché le righe non crescono, altri scorrimenti non ne chiedono un'altra
  let requestedAt = -1
  function maybeMore() {
    if (!hasMore || loading || atLimit || !onmore) return
    if (!nearEnd(visibleRows(scrollTop, H, rows.length, 0).end, rows.length)) return
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
    if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'c') {
      e.preventDefault()
      try {
        await navigator.clipboard.writeText(cellValue(rows, sel.r, sel.c) ?? '')
      } catch {
        /* appunti non disponibili */
      }
      return
    }
    const d = { ArrowUp: [-1, 0], ArrowDown: [1, 0], ArrowLeft: [0, -1], ArrowRight: [0, 1] }[e.key]
    if (!d) return
    e.preventDefault()
    const r = Math.min(rows.length - 1, Math.max(0, sel.r + d[0]))
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

<div class="grid" role="grid" data-no-tooltip="tabella: i controlli sono nelle celle" aria-rowcount={rows.length + 1} aria-colcount={columns.length} tabindex="0" onkeydown={onkey}>
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
      <div class="body" style="height: {rows.length * ROW_H}px">
        {#each rowIdx as r (r)}
          <div class="row" role="row" style="top: {r * ROW_H}px; height: {ROW_H}px">
            <div class="num" role="rowheader">{r + 1}</div>
            {#each visibleIdx as c (c)}
              {@const v = cellValue(rows, r, c)}
              <!-- svelte-ignore a11y_click_events_have_key_events -->
              <div class="cell" data-no-tooltip="cella selezionabile" class:sel={sel?.r === r && sel?.c === c} role="gridcell" tabindex="-1" aria-selected={sel?.r === r && sel?.c === c} style="left: {ROWNUM_W + cols.offsets[c]}px; width: {widths[c]}px" onclick={() => (sel = { r, c })}>
                {#if v === null}<span class="null">NULL</span>{:else}<span class="txt">{v}</span>{/if}
                {#if truncated.has(cellKey(r, c))}<span class="trunc" role="img" aria-label="Valore troncato a 64 KB" use:tooltip={'Valore troncato a 64 KB'}>…</span>{/if}
              </div>
            {/each}
          </div>
        {/each}
      </div>
    </div>
  </div>
  <div class="foot" role="status">
    <span>{fmtInt(rows.length)} {rows.length === 1 ? 'riga' : 'righe'}</span>
    {#if atLimit}
      <span class="note">· Limite di {fmtInt(MAX_ROWS)} righe raggiunto</span>
    {:else if hasMore}
      <span class="note">· Altre righe disponibili</span>
      <button class="more" disabled={loading} use:tooltip={'Legge la pagina successiva del risultato'} onclick={ask}>{loading ? 'Caricamento…' : `Carica altre ${PAGE}`}</button>
    {/if}
  </div>
</div>

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
  .null { color: var(--faint); font-style: italic; }
  .trunc { flex: none; padding: 0 4px; border-radius: 4px; background: var(--edge-soft); color: var(--dim); line-height: 14px; }
  .grip { position: absolute; right: -3px; top: 0; bottom: 0; width: 7px; cursor: col-resize; z-index: 4; }
  .grip:hover { background: color-mix(in srgb, var(--accent, #6aa9ff) 40%, transparent); }
  .foot { flex: none; display: flex; align-items: center; gap: 6px; padding: 4px 10px; border-top: 1px solid var(--edge-soft); color: var(--dim); font-size: 11.5px; }
  .note { color: var(--faint); }
  .more { margin-left: auto; font-size: 11.5px; padding: 2px 8px; border-radius: 6px; border: 1px solid var(--edge-soft); background: transparent; color: inherit; cursor: pointer; }
  .more:disabled { opacity: 0.6; cursor: default; }
</style>
