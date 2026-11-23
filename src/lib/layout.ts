export type ResizableCol = 'sidebar' | 'conv'

export interface LayoutState {
  widths: Record<ResizableCol, number>
  collapsed: ResizableCol[]
}

export const STORAGE_KEY = 'codeart.layout'
export const MIN_WIDTH = 160
export const DEFAULT_LAYOUT: LayoutState = {
  widths: { sidebar: 260, conv: 460 },
  collapsed: [],
}

/** Spazio orizzontale che non va alle colonne ridimensionabili: barra laterale (46), due maniglie (6), margini
 *  della griglia (20) e larghezza minima dell'editor (240, come `minmax(240px, 1fr)` in App). */
export const EDITOR_RESERVE = 46 + 2 * 6 + 20 + 240

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

const COLS: ResizableCol[] = ['sidebar', 'conv']
const FLEX = 'minmax(240px, 1fr)'

/** L'area dei file (editor e terminali) è chiusa quando non c'è niente da mostrare, purché resti aperta un'altra
 *  colonna: la finestra non resta mai vuota. */
export function editorColumnShown(collapsed: ResizableCol[], hasContent: boolean): boolean {
  return hasContent || COLS.every((c) => collapsed.includes(c))
}

/** Colonne della griglia di App: barra icone, poi ogni colonna (0px se chiusa) con la sua maniglia da 6px, poi
 *  l'area dei file che prende lo spazio libero. Ad area chiusa lo spazio libero va all'ultima colonna aperta, la
 *  cui maniglia sparisce (non c'è niente da ridimensionare alla sua destra). */
export function gridTemplate(widths: Record<ResizableCol, number>, collapsed: ResizableCol[], editorCol: boolean): string {
  const last = editorCol ? null : [...COLS].reverse().find((c) => !collapsed.includes(c))
  const parts = COLS.flatMap((c) => (collapsed.includes(c) ? ['0px', '0px'] : c === last ? [FLEX, '0px'] : [`${widths[c]}px`, '6px']))
  return ['46px', ...parts, ...(editorCol ? [FLEX] : [])].join(' ')
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
      ? (p.collapsed.filter((c: unknown) => c === 'sidebar' || c === 'conv') as ResizableCol[])
      : []
    // la vecchia colonna `convlist` (ora vista laterale) viene ignorata
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
