import type { ChatItem, ClaudeEvent, Todo } from '../types'

/** Un todo ricostruito dai `todoDelta`: l'id serve per fondere le variazioni. */
export type TodoItem = Todo & { id: string }
export type ChatState = { items: ChatItem[]; todos: TodoItem[]; contextUsedPct: number | null }

export const initialState = (history: ChatItem[]): ChatState => ({ items: history, todos: [], contextUsedPct: null })

const STATUSES: Todo['status'][] = ['pending', 'in_progress', 'completed']
const asStatus = (s: string): Todo['status'] => (STATUSES.includes(s as Todo['status']) ? (s as Todo['status']) : 'pending')

/** Percorso del file toccato da uno strumento, se l'input ne ha uno. */
export function filePathOf(input: unknown): string | null {
  if (!input || typeof input !== 'object') return null
  const o = input as Record<string, unknown>
  for (const k of ['file_path', 'notebook_path']) if (typeof o[k] === 'string' && o[k]) return o[k] as string
  return null
}

function indexOfItem(items: ChatItem[], pred: (i: ChatItem) => boolean): number {
  for (let i = items.length - 1; i >= 0; i--) if (pred(items[i])) return i
  return -1
}

/** Applica un evento a una bozza (`items`/`todos` già copiati dal chiamante). Gli elementi non si mutano mai: si sostituiscono. */
function apply(d: ChatState, ev: ClaudeEvent): void {
  switch (ev.kind) {
    case 'textDelta':
    case 'thinking': {
      const kind = ev.kind === 'textDelta' ? 'assistant' : 'thinking'
      const i = indexOfItem(d.items, (x) => x.kind === kind && x.id === ev.id)
      if (i >= 0) {
        const cur = d.items[i] as Extract<ChatItem, { kind: 'assistant' | 'thinking' }>
        d.items[i] = { ...cur, text: cur.text + ev.text }
      }
      else d.items.push({ kind, id: ev.id, text: ev.text })
      return
    }
    case 'toolStart': {
      if (indexOfItem(d.items, (x) => x.kind === 'tool' && x.id === ev.id) >= 0) return
      d.items.push({ kind: 'tool', id: ev.id, name: ev.name, input: ev.input, result: null, ok: null, parentId: ev.parentId, filePath: filePathOf(ev.input) })
      return
    }
    case 'toolEnd': {
      const i = indexOfItem(d.items, (x) => x.kind === 'tool' && x.id === ev.id)
      if (i >= 0) d.items[i] = { ...(d.items[i] as Extract<ChatItem, { kind: 'tool' }>), ok: ev.ok, result: ev.result }
      return
    }
    case 'ask': {
      if (indexOfItem(d.items, (x) => x.kind === 'ask' && x.requestId === ev.requestId) >= 0) return
      d.items.push({ kind: 'ask', id: `ask:${ev.requestId}`, requestId: ev.requestId, askType: ev.askType, prompt: ev.prompt, options: ev.options, answered: false })
      return
    }
    case 'askCancelled':
      markAnswered(d, ev.requestId)
      return
    case 'todoDelta': {
      if (ev.status === 'deleted') {
        d.todos = d.todos.filter((t) => t.id !== ev.id)
        return
      }
      const i = d.todos.findIndex((t) => t.id === ev.id)
      if (i < 0) d.todos.push({ id: ev.id, text: ev.text ?? '', status: asStatus(ev.status ?? 'pending') })
      else d.todos[i] = { ...d.todos[i], text: ev.text ?? d.todos[i].text, status: ev.status ? asStatus(ev.status) : d.todos[i].status }
      return
    }
    case 'turnEnd':
      d.contextUsedPct = ev.contextUsedPct
      return
    case 'unknown':
      d.items.push({ kind: 'raw', id: `raw:${d.items.length}:${ev.label}`, label: ev.label, json: ev.json })
      return
    case 'init':
    case 'controlResponse':
      return
  }
}

function markAnswered(d: ChatState, requestId: string) {
  const i = indexOfItem(d.items, (x) => x.kind === 'ask' && x.requestId === requestId)
  if (i >= 0) d.items[i] = { ...(d.items[i] as Extract<ChatItem, { kind: 'ask' }>), answered: true }
}

const draft = (s: ChatState): ChatState => ({ items: [...s.items], todos: [...s.todos], contextUsedPct: s.contextUsedPct })

/** Funzione pura: lo stato in ingresso non viene modificato. Un evento che riferisce un id inesistente non fa nulla. */
export function reduce(state: ChatState, ev: ClaudeEvent): ChatState {
  const d = draft(state)
  apply(d, ev)
  return d
}

/** Applica più eventi con una sola copia (usato per unire gli eventi arrivati nello stesso frame). */
export function reduceAll(state: ChatState, evs: ClaudeEvent[]): ChatState {
  const d = draft(state)
  for (const ev of evs) apply(d, ev)
  return d
}

/** Segna come risposta una richiesta (dopo `convAnswer` riuscito). */
export function answerAsk(state: ChatState, requestId: string): ChatState {
  const d = draft(state)
  markAnswered(d, requestId)
  return d
}

/** Aggiunge un messaggio dell'utente (la CLI non lo rimanda come evento). */
export function appendUser(state: ChatState, id: string, text: string, images: string[]): ChatState {
  const d = draft(state)
  d.items.push({ kind: 'user', id, text, images })
  return d
}

export type Row = { item: ChatItem; children: ChatItem[] }

/** Lista piatta -> righe di primo livello; gli strumenti con `parentId` si annidano sotto il padre (se esiste). */
export function buildRows(items: ChatItem[]): Row[] {
  const rows: Row[] = []
  const byId = new Map<string, Row>()
  for (const it of items) if (it.kind === 'tool') byId.set(it.id, { item: it, children: [] })
  for (const it of items) {
    if (it.kind === 'tool' && it.parentId) {
      const parent = byId.get(it.parentId)
      if (parent && parent !== byId.get(it.id)) {
        parent.children.push(it)
        continue
      }
    }
    rows.push(it.kind === 'tool' ? byId.get(it.id)! : { item: it, children: [] })
  }
  return rows
}

/**
 * Fine turno: sostituisce lo stato con la storia riletta dal file (risolve duplicati e messaggi tagliati tra
 * storia e eventi live), mantenendo ciò che la storia non porta: todo, contesto e richieste ancora senza risposta.
 */
export function reconcile(live: ChatState, history: ChatItem[]): ChatState {
  const known = new Set(history.filter((i) => i.kind === 'ask').map((i) => (i as Extract<ChatItem, { kind: 'ask' }>).requestId))
  const asks = live.items.filter((i) => i.kind === 'ask' && !i.answered && !known.has(i.requestId))
  return { items: [...history, ...asks], todos: live.todos, contextUsedPct: live.contextUsedPct }
}
