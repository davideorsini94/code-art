import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { convArchive, convList, convMarkViewed, convRename, convStart, convUnlockTitle } from '../ipc'
import { errorText } from '../errors'
import { activeCount, sortConversations, upsertConversation, waitingCount } from '../conv/order'
import { clearDraft } from '../conv/draft'
import type { Conversation } from '../types'

class ConversationStore {
  /** Tutte le conversazioni del progetto (anche archiviate), una per sessionId. */
  all = $state<Conversation[]>([])
  activeId = $state<string | null>(null)
  showArchived = $state(false)
  error = $state<string | null>(null)
  /** Comandi slash e modello correnti per sessione, dall'ultimo `init` visto in questa esecuzione. */
  inits = $state.raw<Record<string, { slashCommands: string[]; model: string; at: number }>>({})

  private projectId: string | null = null
  private unlisten: UnlistenFn | null = null
  private unlistenFocus: UnlistenFn | null = null
  private generation = 0

  list = $derived(sortConversations(this.all, this.showArchived))
  waiting = $derived(waitingCount(this.all))
  active = $derived(activeCount(this.all))

  /** Carica l'elenco e resta in ascolto di `conv:upsert`. Gli eventi arrivati durante il caricamento
   *  vengono riapplicati sopra l'elenco letto, così non si perde nulla. */
  async start(projectId: string) {
    this.stop()
    this.projectId = projectId
    const gen = ++this.generation
    let loading = true
    const early: Conversation[] = []
    this.error = null
    try {
      const un = await listen<Conversation>('conv:upsert', (ev) => {
        if (gen !== this.generation || ev.payload.projectId !== projectId) return
        if (loading) early.push(ev.payload)
        else this.upsert(ev.payload)
      })
      if (gen !== this.generation) return un()
      this.unlisten = un
      // clic su una notifica: il backend porta in primo piano la finestra e chiede di aprire la conversazione
      const unFocus = await listen<{ sessionId: string }>('conv:focus', (ev) => {
        if (gen === this.generation) void this.select(ev.payload.sessionId)
      })
      if (gen !== this.generation) return unFocus()
      this.unlistenFocus = unFocus
      this.all = await convList(projectId)
    } catch (e) {
      this.error = errorText(e)
    }
    if (gen !== this.generation) return
    loading = false
    for (const c of early) this.upsert(c)
  }

  stop() {
    this.generation++
    this.unlisten?.()
    this.unlisten = null
    this.unlistenFocus?.()
    this.unlistenFocus = null
  }

  /** Da ogni evento `init` (i comandi e il modello possono cambiare). */
  setInit(sessionId: string, slashCommands: string[], model: string) {
    this.inits = { ...this.inits, [sessionId]: { slashCommands, model, at: Date.now() } }
  }

  upsert(c: Conversation) {
    this.all = upsertConversation(this.all, c)
  }

  async select(id: string) {
    this.activeId = id
    try {
      await convMarkViewed(id)
    } catch (e) {
      this.error = errorText(e)
    }
  }

  /** Nuova conversazione nel progetto corrente; la seleziona. Il sessionId è nuovo: nessun segnaposto in lista, arriva con `conv:upsert`. */
  async create(): Promise<string | null> {
    if (!this.projectId) return null
    try {
      const id = await convStart(this.projectId)
      await this.select(id)
      return id
    } catch (e) {
      this.error = errorText(e)
      return null
    }
  }

  private async run(op: () => Promise<unknown>) {
    this.error = null
    try {
      await op()
    } catch (e) {
      this.error = errorText(e)
    }
  }

  rename = (id: string, title: string) => this.run(() => convRename(id, title))
  unlockTitle = (id: string) => this.run(() => convUnlockTitle(id))
  archive = (id: string, archived: boolean) =>
    this.run(async () => {
      await convArchive(id, archived)
      if (archived) clearDraft(id)
    })
}

export const conversations = new ConversationStore()
