import type { UnlistenFn } from '@tauri-apps/api/event'
import { listenProject } from '../events'
import { convArchive, convDelete, convList, convMarkViewed, convRename, convStart, convUnlockTitle } from '../ipc'
import { errorText, UNTRUSTED } from '../errors'
import { projectStore } from './project.svelte'
import { activeCount, removeConversation, sortConversations, upsertConversation, waitingCount } from '../conv/order'
import { clearDraft } from '../conv/draft'
import type { Conversation } from '../types'
import { closeTab, loadTabs, openTab, pruneTabs, saveTabs } from '../conv/tabs'

/** Per quanto resta visibile un avviso non bloccante. */
export const NOTICE_MS = 8000

class ConversationStore {
  /** Tutte le conversazioni del progetto (anche archiviate), una per sessionId. */
  all = $state<Conversation[]>([])
  activeId = $state<string | null>(null)
  /** Conversazioni aperte come schede, in ordine; ricordate per progetto. */
  tabs = $state<string[]>([])
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
  private noticeTimer: ReturnType<typeof setTimeout> | null = null

  list = $derived(sortConversations(this.all, this.showArchived))
  waiting = $derived(waitingCount(this.all))
  active = $derived(activeCount(this.all))
  /** Le conversazioni delle schede aperte, nell'ordine delle schede. */
  openConvs = $derived(this.tabs.map((id) => this.all.find((c) => c.sessionId === id)).filter((c): c is Conversation => !!c))

  /** Carica l'elenco e resta in ascolto di `conv:upsert`. Gli eventi arrivati durante il caricamento
   *  vengono riapplicati sopra l'elenco letto, così non si perde nulla. */
  async start(projectId: string) {
    this.stop()
    if (this.projectId !== projectId) {
      this.tabs = []
      this.activeId = null
    }
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
    this.restoreTabs(projectId)
  }

  /** Schede salvate per il progetto, senza quelle di conversazioni sparite; tiene quelle aperte durante il caricamento. */
  private restoreTabs(projectId: string) {
    const saved = loadTabs(projectId)
    const known = new Set(this.all.map((c) => c.sessionId))
    let tabs = pruneTabs(saved.tabs, known)
    for (const t of this.tabs) if (known.has(t)) tabs = openTab(tabs, t)
    this.tabs = tabs
    if (!this.activeId || !tabs.includes(this.activeId)) {
      this.activeId = saved.activeId && tabs.includes(saved.activeId) ? saved.activeId : (tabs[0] ?? null)
    }
    this.saveTabs()
  }

  private saveTabs() {
    if (this.projectId) saveTabs(this.projectId, { tabs: this.tabs, activeId: this.activeId })
  }

  /** Chiude la scheda (la conversazione resta nell'elenco e Claude non si ferma). */
  close(id: string) {
    const next = closeTab(this.tabs, this.activeId, id)
    this.tabs = next.tabs
    this.activeId = next.activeId
    this.saveTabs()
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

  /** `conv:removed`: via dalla lista (anche le archiviate), bozza cancellata; la sua scheda si chiude. */
  remove(sessionId: string) {
    this.close(sessionId)
    this.all = removeConversation(this.all, sessionId)
    clearDraft(sessionId)
  }

  /** Mostra la conversazione, aprendone la scheda se manca. */
  async select(id: string) {
    this.tabs = openTab(this.tabs, id)
    this.activeId = id
    this.saveTabs()
    try {
      await convMarkViewed(id)
    } catch (e) {
      this.error = errorText(e)
    }
  }

  /** Mostra (o toglie) l'avviso; sparisce da solo dopo `NOTICE_MS`. */
  private setNotice(msg: string | null) {
    if (this.noticeTimer) clearTimeout(this.noticeTimer)
    this.noticeTimer = msg === null ? null : setTimeout(() => this.setNotice(null), NOTICE_MS)
    this.notice = msg
  }

  /** Nuova conversazione nel progetto corrente; la seleziona. Il sessionId è nuovo: nessun segnaposto in lista, arriva con `conv:upsert`. */
  async create(): Promise<string | null> {
    if (!this.projectId) return null
    this.setNotice(null)
    // cartella non fidata: il core rifiuterebbe; il pulsante è già spento, qui vale anche per la palette
    if (projectStore.untrusted.length > 0) {
      this.error = errorText(UNTRUSTED)
      return null
    }
    try {
      const id = await convStart(this.projectId)
      await this.select(id)
      return id
    } catch (e) {
      this.error = errorText(e)
      return null
    }
  }

  /** Esegue un'azione dell'utente: l'avviso precedente sparisce (tranne per `keepNotice`). */
  private async run(op: () => Promise<unknown>, keepNotice = false) {
    this.error = null
    if (!keepNotice) this.setNotice(null)
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
      this.setNotice(out.kind === 'deletedWithWarning' ? out.message : null)
    }, true)
  archive = (id: string, archived: boolean) =>
    this.run(async () => {
      await convArchive(id, archived)
      if (archived) {
        clearDraft(id)
        this.close(id)
      }
    })
}

export const conversations = new ConversationStore()
