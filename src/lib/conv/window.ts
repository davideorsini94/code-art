/**
 * Finestra di rendering per righe ad altezza variabile (`windowRange` di order.ts vale solo per altezza fissa).
 * `heights` = altezza misurata o stimata di ogni riga. Restituisce le righe da rendere e gli spazi sopra/sotto.
 */
export function variableRange(heights: number[], scrollTop: number, viewport: number, overscanPx = 600): { start: number; end: number; top: number; bottom: number } {
  const n = heights.length
  const from = scrollTop - overscanPx
  const to = scrollTop + viewport + overscanPx
  let y = 0
  let start = 0
  let top = 0
  while (start < n && y + heights[start] <= from) {
    y += heights[start]
    start++
  }
  top = y
  let end = start
  while (end < n && y < to) {
    y += heights[end]
    end++
  }
  let bottom = 0
  for (let i = end; i < n; i++) bottom += heights[i]
  return { start, end, top, bottom }
}
