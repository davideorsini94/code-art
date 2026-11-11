import type { Conversation, ConvStatus } from '../types'

const RANK: Record<ConvStatus, number> = { waiting: 0, working: 1, idle: 2 }

/** Ordine della colonna: waiting, working, idle; nel gruppo lastActivity decrescente. Le archiviate sono escluse
 *  (o, con `archivedOnly`, sono le uniche mostrate). Non muta l'input. */
export function sortConversations(list: Conversation[], archivedOnly = false): Conversation[] {
  return list
    .filter((c) => c.archived === archivedOnly)
    .sort((a, b) => RANK[a.status] - RANK[b.status] || b.lastActivity - a.lastActivity)
}

export const waitingCount = (list: Conversation[]): number => list.filter((c) => !c.archived && c.status === 'waiting').length

/** Conversazioni "attive" = working + waiting (non archiviate). */
export const activeCount = (list: Conversation[]): number => list.filter((c) => !c.archived && c.status !== 'idle').length

/** Inserisce o sostituisce per `sessionId`; restituisce una nuova lista. */
export function upsertConversation(list: Conversation[], c: Conversation): Conversation[] {
  const i = list.findIndex((x) => x.sessionId === c.sessionId)
  if (i < 0) return [...list, c]
  const next = list.slice()
  next[i] = c
  return next
}

/** Oltre questo numero di schede la lista mostra solo quelle visibili. */
export const WINDOW_THRESHOLD = 50
const OVERSCAN = 6

/** Intervallo [start, end) di schede da rendere; altezza scheda fissa. */
export function windowRange(count: number, scrollTop: number, viewport: number, rowHeight: number): { start: number; end: number } {
  if (count <= WINDOW_THRESHOLD) return { start: 0, end: count }
  const first = Math.floor(scrollTop / rowHeight)
  const visible = Math.ceil(viewport / rowHeight)
  return { start: Math.max(0, first - OVERSCAN), end: Math.min(count, first + visible + OVERSCAN) }
}

export const waitingLabel = (n: number): string => `${n} ${n === 1 ? 'ti aspetta' : 'ti aspettano'}`
export const activeLabel = (n: number): string => (n === 1 ? '1 conversazione attiva' : `${n} conversazioni attive`)
