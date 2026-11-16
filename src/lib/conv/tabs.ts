/** Schede delle conversazioni aperte (come i file aperti in un editor), ricordate per progetto. */

export interface TabsState {
  tabs: string[]
  activeId: string | null
}

const key = (projectId: string) => `codeart.convtabs.${projectId}`

/** Aggiunge in coda; se c'è già restituisce lo stesso array. */
export function openTab(tabs: string[], id: string): string[] {
  return tabs.includes(id) ? tabs : [...tabs, id]
}

/** Chiude `id`. Se era l'attiva diventa attiva la vicina a destra, altrimenti quella a sinistra, altrimenti nessuna. */
export function closeTab(tabs: string[], activeId: string | null, id: string): TabsState {
  const i = tabs.indexOf(id)
  if (i < 0) return { tabs, activeId }
  const rest = tabs.filter((t) => t !== id)
  if (activeId !== id) return { tabs: rest, activeId }
  return { tabs: rest, activeId: rest[Math.min(i, rest.length - 1)] ?? null }
}

/** Tiene solo le schede di conversazioni ancora presenti. */
export function pruneTabs(tabs: string[], known: Set<string>): string[] {
  return tabs.filter((t) => known.has(t))
}

export function loadTabs(projectId: string): TabsState {
  try {
    const p = JSON.parse(localStorage.getItem(key(projectId)) ?? 'null')
    const tabs = Array.isArray(p?.tabs) ? p.tabs.filter((t: unknown): t is string => typeof t === 'string') : []
    const activeId = typeof p?.activeId === 'string' ? p.activeId : null
    return { tabs, activeId }
  } catch {
    return { tabs: [], activeId: null }
  }
}

export function saveTabs(projectId: string, state: TabsState): void {
  try {
    localStorage.setItem(key(projectId), JSON.stringify(state))
  } catch {
    /* storage non disponibile: le schede restano solo in memoria */
  }
}
