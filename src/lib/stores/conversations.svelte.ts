import type { UnlistenFn } from '@tauri-apps/api/event'
import { listenProject } from '../events'
import { convArchive, convDelete, convList, convMarkViewed, convRename, convStart, convUnlockTitle } from '../ipc'
import { errorText } from '../errors'
import { activeCount, nextActiveAfterRemoval, removeConversation, sortConversations, upsertConversation, waitingCount } from '../conv/order'
import { clearDraft } from '../conv/draft'
import type { Conversation } from '../types'

class ConversationStore {
  /** Tutte le conversazioni del progetto (anche archiviate), una per sessionId. */
  all = $state<Conversation[]>([])
  activeId = $state<string | null>(null)
  showArchived = $state(false)
  error = $state<string | null>(null)
  /** Avviso non bloccante (es. eliminata ma con un residuo in `~/.claude`). */
  notice = $state<string | null>(null)
  /** Comandi slash e modello correnti per sessione, dall'ultimo `init` visto in questa esecuzione. */
  inits = $state.raw<Record<string, { slashCommands: string[]; model: string; at: number }>>({})

  private projectId: string | null = null
  private unlisten: UnlistenFn | null = null
  private unlistenFocus: UnlistenFn | null = null
  private unlistenRemoved: UnlistenFn | null = null
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
    const removedEarly: string[] = []
    this.error = null
    try {
      const un = await listenProject<Conversation>('conv:upsert', projectId, (ev) => {
        if (gen !== this.generation || ev.payload.projectId !== projectId) return
        if (loading) early.push(ev.payload)
        else this.upsert(ev.payload)
      })
      if (gen !== this.generation) return un()
      this.unlisten = un
      // clic su una notifica: il backend porta in primo piano la finestra e chiede di aprire la conversazione
      const unFocus = await listenProject<{ sessionId: string }>('conv:focus', projectId, (ev) => {
        if (gen === this.generation) void this.select(ev.payload.sessionId)
      })
      if (gen !== this.generation) return unFocus()
      this.unlistenFocus = unFocus
      const unRemoved = await listenProject<{ sessionId: string }>('conv:removed', projectId, (ev) => {
        if (gen !== this.generation) return
        if (loading) removedEarly.push(ev.payload.sessionId)
        else this.remove(ev.payload.sessionId)
      })
      if (gen !== this.generation) return unRemoved()
      this.unlistenRemoved = unRemoved
      this.all = await convList(projectId)
    } catch (e) {
      this.error = errorText(e)
    }
    if (gen !== this.generation) return
    loading = false
    for (const c of early) this.upsert(c)
    for (const id of removedEarly) this.remove(id)
  }

  stop() {
    this.generation++
    this.unlisten?.()
    this.unlisten = null
    this.unlistenFocus?.()
    this.unlistenFocus = null
    this.unlistenRemoved?.()
    this.unlistenRemoved = null
  }

  /** Da ogni evento `init` (i comandi e il modello possono cambiare). */
  setInit(sessionId: string, slashCommands: string[], model: string) {
    this.inits = { ...this.inits, [sessionId]: { slashCommands, model, at: Date.now() } }
  }

  upsert(c: Conversation) {
    this.all = upsertConversation(this.all, c)
  }

  /** `conv:removed`: via dalla lista (anche le archiviate), bozza cancellata; se era l'attiva si passa alla
   *  scheda successiva o a nessuna. */
  remove(sessionId: string) {
    this.activeId = nextActiveAfterRemoval(this.list, this.activeId, sessionId)
    this.all = removeConversation(this.all, sessionId)
    clearDraft(sessionId)
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
  /** La conferma è la finestra nativa aperta dal core: qui nessun "sei sicuro?". La rimozione dalla lista arriva
   *  con `conv:removed`. */
  delete = (id: string) =>
    this.run(async () => {
      const out = await convDelete(id)
      if (out.kind === 'busy') return // altra conferma già aperta: niente cambia, nemmeno un avviso esistente
      this.notice = out.kind === 'deletedWithWarning' ? out.message : null
    })
  archive = (id: string, archived: boolean) =>
    this.run(async () => {
      await convArchive(id, archived)
      if (archived) clearDraft(id)
    })
}

export const conversations = new ConversationStore()
