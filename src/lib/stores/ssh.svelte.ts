import * as ipc from '../ipc'
import { listenSshState } from '../events'
import { errorText } from '../errors'
import { projectStore } from './project.svelte'
import type { Server, ServerInput, SshConnState, SshStateEvent } from '../types'

/** Server SSH e stato delle connessioni (condivise da tutte le finestre). Nessuna password passa di qui: arriva al
 *  core con `setPassword` e non viene conservata. */
export class SshStore {
  servers = $state<Server[]>([])
  /** Stato per server; assente = `disconnected`. */
  states = $state<Record<string, { state: SshConnState; message?: string }>>({})
  /** Server per cui il core ha risposto `auth`: la vista chiede la password. */
  passwordFor = $state<string | null>(null)
  /** Ultimo errore di connessione per server (senza segreti). */
  errors = $state<Record<string, string>>({})
  notice = $state<string | null>(null)
  loaded = $state(false)
  private started = false
  private unlisten: (() => void) | undefined

  /** Una volta per finestra: ascolta `ssh:state` e legge elenco e stati. */
  async init() {
    if (this.started) return
    this.started = true
    try {
      this.unlisten = await listenSshState((ev) => this.applyState(ev.payload))
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
      this.servers = await ipc.sshServers()
    } catch (e) {
      this.notice = errorText(e)
    }
  }

  async refreshStates() {
    try {
      for (const s of await ipc.sshStates()) this.applyState(s)
    } catch {
      /* resta lo stato precedente */
    }
  }

  applyState(ev: SshStateEvent) {
    this.states[ev.id] = { state: ev.state, message: ev.message }
    if (ev.state === 'connected') delete this.errors[ev.id]
    else if (ev.state === 'error' && ev.message) this.errors[ev.id] = ev.message
  }

  stateOf(id: string): SshConnState {
    return this.states[id]?.state ?? 'disconnected'
  }

  /** Server connessi, nell'ordine dell'elenco. */
  get connected(): Server[] {
    return this.servers.filter((s) => this.stateOf(s.id) === 'connected')
  }

  async connect(id: string) {
    delete this.errors[id]
    this.states[id] = { state: 'connecting' }
    try {
      await ipc.sshConnect(id)
      if (this.passwordFor === id) this.passwordFor = null
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
      await ipc.sshDisconnect(id)
    } catch (e) {
      this.errors[id] = errorText(e)
    }
  }

  /** Salva la password nel Portachiavi (il chiamante svuota il proprio campo) e, se serviva alla connessione, riprova. */
  async setPassword(id: string, password: string, thenConnect = false) {
    await ipc.sshPasswordSet(id, password)
    await this.refresh()
    if (thenConnect) {
      this.passwordFor = null
      await this.connect(id)
    }
  }

  async clearPassword(id: string) {
    await ipc.sshPasswordClear(id)
    await this.refresh()
  }

  async save(input: ServerInput): Promise<Server> {
    const s = await ipc.sshServerSave(input)
    await this.refresh()
    return s
  }

  async remove(id: string) {
    await ipc.sshServerDelete(id)
    await this.refresh()
  }

  /** Scarica un percorso del server nella prima cartella del progetto; l'avanzamento è dei trasferimenti. */
  async download(serverId: string, path: string) {
    const dir = projectStore.project?.folders[0]
    if (!dir) {
      this.notice = 'Nessuna cartella di progetto in cui scaricare'
      return
    }
    try {
      await ipc.sshDownload(serverId, [path], dir)
      this.notice = `Download avviato in ${dir}`
    } catch (e) {
      this.notice = errorText(e)
    }
  }
}

export const ssh = new SshStore()
