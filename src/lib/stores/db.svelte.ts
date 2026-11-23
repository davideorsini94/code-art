import * as ipc from '../ipc'
import { listenDbState } from '../events'
import { errorText } from '../errors'
import type { DbConnection, DbConnectionInput, DbKind, DbStateEvent, MongoDb, PgTree } from '../types'

export type DbConnState = DbStateEvent['state']

/** Albero di una connessione: schemi/tabelle (Postgres) o database/collezioni (MongoDB). */
export type DbTreeData = { kind: 'postgres'; tree: PgTree } | { kind: 'mongo'; dbs: MongoDb[] }

/** Richiesta di aprire una scheda query. Senza `table`/`collection` = query vuota. */
export type DbQueryRequest = {
  connectionId: string
  kind: DbKind
  schema?: string
  table?: string
  database?: string
  collection?: string
}

/** Connessioni database e loro stato (condivise da tutte le finestre). Nessuna password passa di qui: arriva al core
 *  con `setPassword` e non viene conservata. */
export class DbStore {
  connections = $state<DbConnection[]>([])
  /** Stato per connessione; assente = `disconnected`. */
  states = $state<Record<string, { state: DbConnState; message?: string }>>({})
  /** Connessione per cui il core ha risposto `auth`: la vista chiede la password. */
  passwordFor = $state<string | null>(null)
  errors = $state<Record<string, string>>({})
  /** Alberi letti dal core; restano (in grigio) a connessione caduta. */
  trees = $state<Record<string, DbTreeData>>({})
  treeErrors = $state<Record<string, string>>({})
  notice = $state<string | null>(null)
  loaded = $state(false)
  /** Ultima richiesta di apertura query; la consuma la scheda query (Task 8) e poi la azzera. */
  pendingQuery = $state<DbQueryRequest | null>(null)
  private started = false
  private unlisten: (() => void) | undefined
  private loading = new Map<string, Promise<void>>()

  /** Una volta per finestra: ascolta `db:state` e legge elenco e stati. */
  async init() {
    if (this.started) return
    this.started = true
    try {
      this.unlisten = await listenDbState((ev) => this.applyState(ev.payload))
    } catch {
      /* senza eventi restano i valori letti sotto */
    }
    await Promise.all([this.refresh(), this.refreshStates()])
    this.loaded = true
  }

  stop() {
    this.unlisten?.()
    this.unlisten = undefined
    this.started = false
  }

  async refresh() {
    try {
      this.connections = await ipc.dbConnections()
    } catch (e) {
      this.notice = errorText(e)
    }
  }

  async refreshStates() {
    try {
      for (const s of await ipc.dbStates()) this.applyState(s)
    } catch {
      /* resta lo stato precedente */
    }
  }

  applyState(ev: DbStateEvent) {
    this.states[ev.id] = { state: ev.state, message: ev.message }
    if (ev.state === 'connected') {
      delete this.errors[ev.id]
      void this.loadTree(ev.id)
    } else if (ev.state === 'error' && ev.message) this.errors[ev.id] = ev.message
  }

  stateOf(id: string): DbConnState {
    return this.states[id]?.state ?? 'disconnected'
  }

  /** (Ri)legge l'albero della connessione; richieste simultanee condividono la stessa lettura. */
  loadTree(id: string): Promise<void> {
    const running = this.loading.get(id)
    if (running) return running
    const kind = this.connections.find((c) => c.id === id)?.kind
    const p = (async () => {
      try {
        // il tipo può non essere ancora noto (evento prima dell'elenco): lo si ricava dopo il refresh
        const k = kind ?? (await this.kindOf(id))
        if (!k) return
        this.trees[id] = k === 'postgres' ? { kind: 'postgres', tree: await ipc.dbPgTree(id) } : { kind: 'mongo', dbs: await ipc.dbMongoTree(id) }
        delete this.treeErrors[id]
      } catch (e) {
        this.treeErrors[id] = errorText(e)
      } finally {
        this.loading.delete(id)
      }
    })()
    this.loading.set(id, p)
    return p
  }

  private async kindOf(id: string): Promise<DbKind | undefined> {
    if (!this.connections.some((c) => c.id === id)) await this.refresh()
    return this.connections.find((c) => c.id === id)?.kind
  }

  async connect(id: string) {
    delete this.errors[id]
    this.states[id] = { state: 'connecting' }
    try {
      await ipc.dbConnect(id)
      if (this.passwordFor === id) this.passwordFor = null
      if (this.stateOf(id) === 'connecting') this.states[id] = { state: 'connected' }
      await this.loadTree(id)
    } catch (e) {
      if (String(e) === 'auth') {
        this.states[id] = { state: 'disconnected' }
        this.passwordFor = id
        return
      }
      this.errors[id] = errorText(e)
      this.states[id] = { state: 'error', message: this.errors[id] }
    }
  }

  async disconnect(id: string) {
    try {
      await ipc.dbDisconnect(id)
    } catch (e) {
      this.errors[id] = errorText(e)
    }
  }

  /** Salva la password nel Portachiavi (il chiamante svuota il proprio campo) e, se serviva alla connessione, riprova. */
  async setPassword(id: string, password: string, thenConnect = false) {
    await ipc.dbPasswordSet(id, password)
    await this.refresh()
    if (thenConnect) {
      this.passwordFor = null
      await this.connect(id)
    }
  }

  async clearPassword(id: string) {
    await ipc.dbPasswordClear(id)
    await this.refresh()
  }

  /** Il core non chiude la sessione aperta quando una connessione si modifica (resterebbe sul vecchio indirizzo):
   *  la si disconnette prima di salvare e l'albero si ricostruisce alla prossima connessione. */
  async save(input: DbConnectionInput): Promise<DbConnection> {
    const id = input.id
    if (id && ['connected', 'connecting'].includes(this.stateOf(id))) {
      await ipc.dbDisconnect(id)
      this.states[id] = { state: 'disconnected' }
      this.notice = 'Connessione modificata: era aperta ed è stata chiusa. Riconnetti per usare i nuovi valori.'
    }
    const c = await ipc.dbConnectionSave(input)
    delete this.trees[c.id]
    delete this.treeErrors[c.id]
    await this.refresh()
    return c
  }

  async remove(id: string) {
    if (this.stateOf(id) !== 'disconnected') {
      try {
        await ipc.dbDisconnect(id)
      } catch {
        /* si elimina comunque */
      }
    }
    await ipc.dbConnectionDelete(id)
    forgetQueryTexts(id)
    delete this.trees[id]
    delete this.states[id]
    await this.refresh()
  }

  /** Chiede di aprire una scheda query; la consuma la scheda query (Task 8) leggendo `pendingQuery`. */
  openQuery(req: DbQueryRequest) {
    this.pendingQuery = req
  }
}

/** Toglie i testi delle schede query salvati per la connessione (`codeart.dbquery.<id>.*` in `localStorage`). */
export function forgetQueryTexts(id: string) {
  try {
    const prefix = `codeart.dbquery.${id}.`
    const keys: string[] = []
    for (let i = 0; i < localStorage.length; i++) {
      const k = localStorage.key(i)
      if (k?.startsWith(prefix)) keys.push(k)
    }
    for (const k of keys) localStorage.removeItem(k)
  } catch {
    /* localStorage non disponibile: niente da togliere */
  }
}

export const db = new DbStore()
