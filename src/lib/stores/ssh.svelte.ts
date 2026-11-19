import * as ipc from '../ipc'
import { listenProject, listenSshState } from '../events'
import { errorText } from '../errors'
import { projectStore } from './project.svelte'
import type { Server, ServerInput, SshConnState, SshStateEvent, TransferEvent } from '../types'

/** Trasferimento mostrato nel pannello: l'ultimo evento ricevuto più la destinazione (per aggiornare l'elenco a fine). */
export type TransferView = TransferEvent & { dest?: { serverId: string; dir: string; remote: boolean } }

const HIDE_AFTER_MS = 8000
/** Errore di `ssh_connect` quando l'utente annulla il dialogo del server nuovo (`conn::CANCELLED`). */
const CANCELLED = 'Connessione annullata'

/** Radice assoluta dalla cartella iniziale (`~`, `~/…` o assoluta) e dalla home del server, senza `/` finali. */
export function resolveRoot(startDir: string | null | undefined, home: string): string {
  const h = home.replace(/\/+$/, '')
  const d = startDir?.trim() || '~'
  const abs = d === '~' ? h || '/' : d.startsWith('~/') ? `${h}/${d.slice(2)}` : d
  return abs.replace(/\/+$/, '') || '/'
}

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
  /** Trasferimenti attivi o recenti, in ordine di avvio. */
  transfers = $state<TransferView[]>([])
  /** Radice (assoluta) dell'albero di ogni server connesso; si dimentica quando il server si modifica o si elimina. */
  roots = $state<Record<string, string>>({})
  /** Cartella del server da rileggere dopo un trasferimento finito (la ascolta `RemoteTree`). */
  reloadDir = $state<{ serverId: string; dir: string; n: number } | null>(null)
  private started = false
  private unlistenTransfer: (() => void) | undefined
  private pendingDest = new Map<string, TransferView['dest']>()
  private hideTimers = new Map<string, ReturnType<typeof setTimeout>>()
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
    await this.listenTransfers()
  }

  /** Eventi `ssh:transfer` del progetto della finestra (serve il progetto caricato). */
  private async listenTransfers() {
    const pid = projectStore.project?.id
    if (!pid || this.unlistenTransfer) return
    try {
      this.unlistenTransfer = await listenProject<TransferEvent>('ssh:transfer', pid, (e) => this.applyTransfer(e.payload))
    } catch {
      /* senza eventi i trasferimenti non mostrano l'avanzamento */
    }
  }

  applyTransfer(ev: TransferEvent) {
    const i = this.transfers.findIndex((t) => t.transferId === ev.transferId)
    const dest = i >= 0 ? this.transfers[i].dest : this.pendingDest.get(ev.transferId)
    const view: TransferView = { ...ev, dest }
    if (i >= 0) this.transfers[i] = view
    else this.transfers.push(view)
    if (ev.state !== 'running') this.finished(view)
  }

  private finished(t: TransferView) {
    if (t.dest?.remote) this.reloadDir = { serverId: t.dest.serverId, dir: t.dest.dir, n: (this.reloadDir?.n ?? 0) + 1 }
    if (t.state === 'error' || t.errors?.length) return // con errori resta finché non lo si chiude
    clearTimeout(this.hideTimers.get(t.transferId))
    this.hideTimers.set(t.transferId, setTimeout(() => this.dismiss(t.transferId), HIDE_AFTER_MS))
  }

  dismiss(id: string) {
    clearTimeout(this.hideTimers.get(id))
    this.hideTimers.delete(id)
    this.pendingDest.delete(id)
    this.transfers = this.transfers.filter((t) => t.transferId !== id)
  }

  async cancelTransfer(id: string) {
    try {
      await ipc.sshTransferCancel(id)
    } catch (e) {
      this.notice = errorText(e)
    }
  }

  /** Dal progetto al server. */
  async upload(serverId: string, localPaths: string[], remoteDir: string) {
    try {
      const id = await ipc.sshUpload(serverId, localPaths, remoteDir)
      this.track(id, { serverId, dir: remoteDir, remote: true })
    } catch (e) {
      this.notice = errorText(e)
    }
  }

  /** Dal server a una cartella del Mac. */
  async downloadTo(serverId: string, remotePaths: string[], localDir: string) {
    try {
      const id = await ipc.sshDownload(serverId, remotePaths, localDir)
      this.track(id, { serverId, dir: localDir, remote: false })
    } catch (e) {
      this.notice = errorText(e)
    }
  }

  private track(id: string, dest: NonNullable<TransferView['dest']>) {
    const t = this.transfers.find((x) => x.transferId === id)
    if (t) {
      t.dest = dest // l'evento è arrivato prima della risposta
      if (t.state !== 'running') this.finished(t)
    } else this.pendingDest.set(id, dest)
  }

  stop() {
    this.unlisten?.()
    this.unlisten = undefined
    this.unlistenTransfer?.()
    this.unlistenTransfer = undefined
    for (const h of this.hideTimers.values()) clearTimeout(h)
    this.hideTimers.clear()
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
      if (String(e) === CANCELLED) {
        // dialogo del server nuovo annullato: non è un errore
        this.states[id] = { state: 'disconnected' }
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

  /** Un server modificato si disconnette nel core (la connessione aperta porterebbe al vecchio indirizzo): l'albero
   *  si dimentica e si ricostruisce alla prossima connessione. */
  async save(input: ServerInput): Promise<Server> {
    const s = await ipc.sshServerSave(input)
    delete this.roots[s.id]
    await this.refresh()
    return s
  }

  async remove(id: string) {
    await ipc.sshServerDelete(id)
    delete this.roots[id]
    await this.refresh()
  }

  /** Scarica un percorso del server nella prima cartella del progetto; l'avanzamento è dei trasferimenti. */
  async download(serverId: string, path: string) {
    const dir = projectStore.project?.folders[0]
    if (!dir) {
      this.notice = 'Nessuna cartella di progetto in cui scaricare'
      return
    }
    this.notice = null
    await this.downloadTo(serverId, [path], dir)
    if (!this.notice) this.notice = `Download avviato in ${dir}`
  }
}

export const ssh = new SshStore()
