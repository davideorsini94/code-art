export type ResizableCol = 'sidebar' | 'convlist' | 'conv'

export interface LayoutState {
  widths: Record<ResizableCol, number>
  collapsed: ResizableCol[]
}

export const STORAGE_KEY = 'codeart.layout'
export const MIN_WIDTH = 160
export const DEFAULT_LAYOUT: LayoutState = {
  widths: { sidebar: 218, convlist: 270, conv: 420 },
  collapsed: [],
}

/** Spazio orizzontale che non va alle colonne ridimensionabili: barra laterale (46), tre maniglie (6), margini
 *  della griglia (20) e larghezza minima dell'editor (240, come `minmax(240px, 1fr)` in App). */
export const EDITOR_RESERVE = 46 + 3 * 6 + 20 + 240

/** Larghezze adattate alla finestra: se le colonne aperte non ci stanno (layout salvato su uno schermo più grande),
 *  si riducono in proporzione, mai sotto `MIN_WIDTH`. Le colonne chiuse restano com'erano. */
export function clampWidths(widths: Record<ResizableCol, number>, collapsed: ResizableCol[], windowWidth: number): Record<ResizableCol, number> {
  const open = (Object.keys(widths) as ResizableCol[]).filter((c) => !collapsed.includes(c))
  const used = open.reduce((n, c) => n + widths[c], 0)
  const available = windowWidth - EDITOR_RESERVE
  if (used <= available || used === 0) return { ...widths }
  const k = Math.max(0, available) / used
  const out = { ...widths }
  for (const c of open) out[c] = Math.max(MIN_WIDTH, Math.floor(widths[c] * k))
  return out
}

/** Almeno una tra `convlist` e `conv` deve restare aperta. */
export function canCollapse(col: string, collapsed: Set<string>): boolean {
  if (col === 'convlist') return !collapsed.has('conv')
  if (col === 'conv') return !collapsed.has('convlist')
  return true
}

export function loadLayout(storage: Pick<Storage, 'getItem'> = localStorage): LayoutState {
  try {
    const raw = storage.getItem(STORAGE_KEY)
    if (!raw) return structuredClone(DEFAULT_LAYOUT)
    const p = JSON.parse(raw)
    const widths = { ...DEFAULT_LAYOUT.widths }
    for (const k of Object.keys(widths) as ResizableCol[]) {
      if (typeof p?.widths?.[k] === 'number') widths[k] = Math.max(MIN_WIDTH, p.widths[k])
    }
    const collapsed = Array.isArray(p?.collapsed)
      ? (p.collapsed.filter((c: unknown) => c === 'sidebar' || c === 'convlist' || c === 'conv') as ResizableCol[])
      : []
    // stato salvato incoerente: non si possono avere chiuse entrambe
    if (collapsed.includes('convlist') && collapsed.includes('conv')) {
      return { widths, collapsed: collapsed.filter((c) => c !== 'conv') }
    }
    return { widths, collapsed }
  } catch {
    return structuredClone(DEFAULT_LAYOUT)
  }
}

export function saveLayout(state: LayoutState, storage: Pick<Storage, 'setItem'> = localStorage): void {
  try {
    storage.setItem(STORAGE_KEY, JSON.stringify(state))
  } catch {
    /* storage non disponibile: il layout resta solo in memoria */
  }
}
