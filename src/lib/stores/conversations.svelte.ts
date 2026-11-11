import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { convArchive, convList, convMarkViewed, convRename, convStart, convUnlockTitle } from '../ipc'
import { activeCount, sortConversations, upsertConversation, waitingCount } from '../conv/order'
import type { Conversation } from '../types'

class ConversationStore {
  /** Tutte le conversazioni del progetto (anche archiviate), una per sessionId. */
  all = $state<Conversation[]>([])
  activeId = $state<string | null>(null)
  showArchived = $state(false)
  error = $state<string | null>(null)

  private projectId: string | null = null
  private unlisten: UnlistenFn | null = null
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
      this.all = await convList(projectId)
    } catch (e) {
      this.error = String(e)
    }
    if (gen !== this.generation) return
    loading = false
    for (const c of early) this.upsert(c)
  }

  stop() {
    this.generation++
    this.unlisten?.()
    this.unlisten = null
  }

  upsert(c: Conversation) {
    this.all = upsertConversation(this.all, c)
  }

  async select(id: string) {
    this.activeId = id
    try {
      await convMarkViewed(id)
    } catch (e) {
      this.error = String(e)
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
      this.error = String(e)
      return null
    }
  }

  private async run(op: () => Promise<unknown>) {
    this.error = null
    try {
      await op()
    } catch (e) {
      this.error = String(e)
    }
  }

  rename = (id: string, title: string) => this.run(() => convRename(id, title))
  unlockTitle = (id: string) => this.run(() => convUnlockTitle(id))
  archive = (id: string, archived: boolean) => this.run(() => convArchive(id, archived))
}

export const conversations = new ConversationStore()
