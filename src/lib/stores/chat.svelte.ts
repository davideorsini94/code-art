import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { convAnswer, convHistory } from '../ipc'
import { answerAsk, appendUser, initialState, reduceAll, type ChatState } from '../conv/reduce'
import type { ClaudeEvent, ConvEventPayload } from '../types'

/** Quante conversazioni tengono lo stato in memoria (le ultime viste). */
export const CACHE_SIZE = 5

const frame = (cb: () => void) => (typeof requestAnimationFrame === 'function' ? requestAnimationFrame(cb) : setTimeout(cb, 16))

/**
 * Stato delle conversazioni aperte di recente: storia (`conv_history`) + eventi `conv:event`.
 * - sessione in cache: gli eventi si accumulano e si applicano al massimo una volta per frame;
 * - sessione in caricamento: gli eventi si tengono da parte e si applicano sopra la storia appena letta;
 * - altre sessioni: gli eventi si scartano (alla prossima apertura la storia dal file li contiene già).
 */
class ChatStore {
  states = $state.raw<Record<string, ChatState>>({})
  error = $state<string | null>(null)

  private order: string[] = [] // dalla più recente
  private buffering = new Map<string, ClaudeEvent[]>()
  private queue = new Map<string, ClaudeEvent[]>()
  private scheduled = false
  private unlisten: UnlistenFn | null = null
  private generation = 0

  get(sessionId: string | null): ChatState | undefined {
    return sessionId ? this.states[sessionId] : undefined
  }

  async start() {
    this.stop()
    const gen = ++this.generation
    const un = await listen<ConvEventPayload>('conv:event', (ev) => this.onEvent(ev.payload))
    if (gen !== this.generation) return un()
    this.unlisten = un
  }

  stop() {
    this.generation++
    this.unlisten?.()
    this.unlisten = null
  }

  /** Porta in primo piano la sessione; se non è in cache ne legge la storia. */
  async open(sessionId: string): Promise<void> {
    this.order = [sessionId, ...this.order.filter((s) => s !== sessionId)]
    this.evict()
    if (this.states[sessionId] || this.buffering.has(sessionId)) return
    const gen = this.generation
    this.buffering.set(sessionId, [])
    this.error = null
    try {
      const history = await convHistory(sessionId)
      const early = this.buffering.get(sessionId) ?? []
      this.buffering.delete(sessionId)
      if (gen !== this.generation || !this.order.includes(sessionId)) return
      this.states = { ...this.states, [sessionId]: reduceAll(initialState(history), early) }
    } catch (e) {
      this.buffering.delete(sessionId)
      this.error = String(e)
    }
  }

  private evict() {
    for (const gone of this.order.splice(CACHE_SIZE)) {
      this.queue.delete(gone)
      if (gone in this.states) {
        const { [gone]: _drop, ...rest } = this.states
        this.states = rest
      }
    }
  }

  private onEvent({ sessionId, event }: ConvEventPayload) {
    const early = this.buffering.get(sessionId)
    if (early) return void early.push(event)
    if (!this.states[sessionId]) return
    const q = this.queue.get(sessionId)
    if (q) q.push(event)
    else this.queue.set(sessionId, [event])
    if (!this.scheduled) {
      this.scheduled = true
      frame(() => this.flush())
    }
  }

  /** Applica gli eventi in coda (uno stato nuovo per sessione, indipendentemente da quanti eventi sono arrivati). */
  flush() {
    this.scheduled = false
    const next = { ...this.states }
    for (const [sid, evs] of this.queue) if (next[sid]) next[sid] = reduceAll(next[sid], evs)
    this.queue.clear()
    this.states = next
  }

  private update(sessionId: string, f: (s: ChatState) => ChatState) {
    const s = this.states[sessionId]
    if (s) this.states = { ...this.states, [sessionId]: f(s) }
  }

  /** Risponde a una richiesta di Claude; a risposta accettata la segna come risposta. Rilancia l'errore. */
  async answer(sessionId: string, requestId: string, choice: string): Promise<void> {
    try {
      await convAnswer(sessionId, requestId, choice)
    } catch (e) {
      if (String(e).includes('non più aperta')) this.update(sessionId, (s) => answerAsk(s, requestId))
      throw e
    }
    this.update(sessionId, (s) => answerAsk(s, requestId))
  }

  /** Per il compositore: mostra subito il messaggio inviato (la CLI non lo rimanda come evento). */
  addUser(sessionId: string, text: string, images: string[] = []) {
    this.flush()
    this.update(sessionId, (s) => appendUser(s, `local:${crypto.randomUUID()}`, text, images))
  }
}

export const chat = new ChatStore()
