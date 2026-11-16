import { listenProject } from '../events'
import { projectBranch, projectCurrent, projectMissingFolders, projectPickRelocate, projectRename, trustAccept, trustCheck } from '../ipc'
import type { Project } from '../types'

class ProjectStore {
  project = $state<Project | null>(null)
  branch = $state<string | null>(null)
  /** Repository del branch mostrato (il selettore agisce su questo). */
  branchRepo = $state<string | null>(null)
  /** File attivo nell'editor: con più repository decide quale branch mostrare. */
  private activeFile: string | null = null
  missing = $state<string[]>([])
  /** Cartelle non fidate: finché ce n'è una, Claude e terminale sono disattivati (il core rifiuta). */
  untrusted = $state<string[]>([])
  loaded = $state(false)

  /** Carica il progetto della finestra; nella finestra "Apri cartella" resta null. */
  async load() {
    try {
      this.project = await projectCurrent()
      await Promise.all([this.refreshBranch(), this.refreshMissing(), this.refreshTrust()])
      // prima apertura di una cartella mai vista: domanda nativa (la risposta la registra il core)
      if (this.untrusted.length > 0) void this.acceptTrust()
      // il watcher dei file emette fs:changed (anche per `.git/HEAD`): rileggiamo il branch
      void listenProject('fs:changed', this.project.id, () => void this.refreshBranch())
      window.addEventListener('focus', () => void this.refreshBranch())
    } catch {
      this.project = null
    } finally {
      this.loaded = true
    }
  }

  async refreshBranch() {
    if (!this.project) return
    const b = await projectBranch(this.activeFile).catch(() => null)
    this.branch = b?.name ?? null
    this.branchRepo = b?.repo ?? null
  }

  /** Il file attivo dell'editor è cambiato: con più repository il branch mostrato segue il file. */
  setActiveFile(path: string | null) {
    if (path === this.activeFile) return
    this.activeFile = path
    void this.refreshBranch()
  }

  async refreshMissing() {
    if (this.project) this.missing = await projectMissingFolders().catch(() => [])
  }

  async refreshTrust() {
    if (this.project) this.untrusted = await trustCheck().catch(() => this.untrusted)
  }

  /** Conferma nativa mostrata dal core: automatica all'apertura, oppure dal "Mi fido" del banner (`fromClick`).
   *  Dopo un rifiuto il core non la ripropone in automatico e ignora il banner per 10 s. */
  async acceptTrust(fromClick = false) {
    if (!this.project) return
    await trustAccept(fromClick).catch(() => false)
    await this.refreshTrust()
  }

  async rename(name: string) {
    if (!this.project) return
    this.project = await projectRename(name)
  }

  /** Nuova posizione della cartella `old`, scelta con il selettore nativo. */
  async relocate(old: string) {
    if (!this.project) return
    const p = await projectPickRelocate(old)
    if (!p) return
    this.project = p
    await Promise.all([this.refreshBranch(), this.refreshMissing(), this.refreshTrust()])
  }
}

export const projectStore = new ProjectStore()
