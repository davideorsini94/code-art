// Logica pura della griglia dei risultati: finestra visibile (righe e colonne), larghezze, unione delle colonne MongoDB
// e accodamento delle pagine. Nessun DOM qui.

export const ROW_H = 24
export const PAGE = 500
/** Righe lette al massimo per risultato (lo stesso limite del core). */
export const MAX_ROWS = 100_000
export const MIN_COL = 60
export const MAX_COL = 400
/** Larghezza della colonna dei numeri di riga. */
export const ROWNUM_W = 56

export type Cell = string | null
export type GridData = {
  columns: string[]
  rows: Cell[][]
  /** Celle troncate, chiave `riga:colonna` (indici globali). */
  truncated: Set<string>
}

export const cellKey = (r: number, c: number) => `${r}:${c}`
export const emptyGrid = (): GridData => ({ columns: [], rows: [], truncated: new Set() })

/** Righe da disegnare: da `start` (incluso) a `end` (escluso), con un margine sopra e sotto. */
export function visibleRows(scrollTop: number, viewportH: number, rowCount: number, overscan = 6): { start: number; end: number } {
  if (rowCount <= 0) return { start: 0, end: 0 }
  const first = Math.floor(Math.max(0, scrollTop) / ROW_H)
  const last = Math.ceil((Math.max(0, scrollTop) + Math.max(0, viewportH)) / ROW_H)
  return { start: Math.max(0, first - overscan), end: Math.min(rowCount, last + overscan) }
}

/** Colonne da disegnare con le posizioni (somme cumulative delle larghezze); `offsets[i]` = sinistra della colonna i. */
export function visibleColumns(scrollLeft: number, viewportW: number, widths: number[], overscan = 1): { start: number; end: number; offsets: number[]; total: number } {
  const offsets: number[] = []
  let x = 0
  for (const w of widths) {
    offsets.push(x)
    x += w
  }
  const left = Math.max(0, scrollLeft)
  const right = left + Math.max(0, viewportW)
  let start = 0
  while (start < widths.length - 1 && offsets[start] + widths[start] <= left) start++
  let end = start
  while (end < widths.length && offsets[end] < right) end++
  return { start: Math.max(0, start - overscan), end: Math.min(widths.length, end + overscan), offsets, total: x }
}

export const clampWidth = (w: number) => Math.min(MAX_COL, Math.max(MIN_COL, Math.round(w)))

/** Larghezza iniziale di una colonna dall'intestazione e dai valori della prima pagina (al massimo 50 righe). */
export function estimateWidth(name: string, rows: Cell[][], col: number, charW = 7.5, pad = 28): number {
  let chars = name.length + 2
  for (let i = 0; i < Math.min(rows.length, 50); i++) {
    const v = rows[i][col]
    chars = Math.max(chars, v === null || v === undefined ? 4 : Math.min(v.length, 80))
  }
  return clampWidth(chars * charW + pad)
}

/** Unisce le colonne di una pagina MongoDB: ordine di prima comparsa, `_id` sempre per prima. */
export function mergeColumns(current: string[], incoming: string[]): string[] {
  const out = [...current]
  for (const n of incoming) if (!out.includes(n)) out.push(n)
  const i = out.indexOf('_id')
  if (i > 0) {
    out.splice(i, 1)
    out.unshift('_id')
  }
  return out
}

type Page = { columns?: { name: string }[]; rows: Cell[][]; truncated?: [number, number][] }

/** Accoda una pagina. Postgres: le colonne sono quelle della prima pagina (anche duplicate) e le righe si accodano
 *  così come sono. MongoDB (`byName`): le colonne di ogni pagina si uniscono a quelle note e le righe si riallineano
 *  per nome (campo assente = null). */
export function appendPage(data: GridData, page: Page, byName: boolean): GridData {
  const pageCols = (page.columns ?? []).map((c) => c.name)
  const offset = data.rows.length
  if (!byName) {
    const columns = data.columns.length ? data.columns : pageCols
    const truncated = new Set(data.truncated)
    for (const [r, c] of page.truncated ?? []) truncated.add(cellKey(offset + r, c))
    return { columns, rows: data.rows.concat(page.rows), truncated }
  }
  const columns = mergeColumns(data.columns, pageCols)
  // colonne spostate (solo `_id` che arriva dopo): si riallineano le righe già lette e le celle troncate
  const prefixSame = data.columns.every((n, i) => columns[i] === n)
  let rows = data.rows
  let truncated = new Set(data.truncated)
  if (!prefixSame) {
    const idx = data.columns.map((n) => columns.indexOf(n))
    rows = data.rows.map((row) => {
      const out: Cell[] = new Array(columns.length).fill(null)
      row.forEach((v, i) => (out[idx[i]] = v))
      return out
    })
    truncated = new Set([...truncated].map((k) => { const [r, c] = k.split(':').map(Number); return cellKey(r, idx[c]) }))
  }
  const map = pageCols.map((n) => columns.indexOf(n))
  const added = page.rows.map((row) => {
    const out: Cell[] = new Array(columns.length).fill(null)
    row.forEach((v, i) => (out[map[i]] = v ?? null))
    return out
  })
  for (const [r, c] of page.truncated ?? []) if (map[c] !== undefined) truncated.add(cellKey(offset + r, map[c]))
  return { columns, rows: rows.concat(added), truncated }
}

/** Serve la pagina successiva? Quando l'ultima riga visibile è a meno di `threshold` righe dalla fine. */
export function nearEnd(endRow: number, rowCount: number, threshold = 10): boolean {
  return rowCount > 0 && endRow >= rowCount - threshold
}

/** Valore di una cella (celle oltre la lunghezza della riga = null). */
export const cellValue = (rows: Cell[][], r: number, c: number): Cell => rows[r]?.[c] ?? null

/** Identificatore SQL citato: doppi apici raddoppiati. */
export const quoteIdent = (s: string) => `"${s.replace(/"/g, '""')}"`

export const selectAllSql = (schema: string, table: string) => `SELECT * FROM ${quoteIdent(schema)}.${quoteIdent(table)};`

/** Oracle: senza `;` finale (ORA-00911) e con i nomi esatti del catalogo. */
export const selectAllOracle = (owner: string, table: string) => `SELECT * FROM ${quoteIdent(owner)}.${quoteIdent(table)}`


/** Numero con separatore delle migliaia all'italiana (senza dipendere dalla lingua del sistema). */
export const fmtInt = (n: number) => String(n).replace(/\B(?=(\d{3})+(?!\d))/g, '.')
