import { listen } from '@tauri-apps/api/event'
import { projectBranch, projectCurrent, projectMissingFolders, projectPickRelocate, projectRename, trustAccept, trustCheck } from '../ipc'
import type { Project } from '../types'

class ProjectStore {
  project = $state<Project | null>(null)
  branch = $state<string | null>(null)
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
      // Il watcher dei file (tappe successive) emette fs:changed: rileggiamo il branch.
      void listen('fs:changed', () => void this.refreshBranch())
      window.addEventListener('focus', () => void this.refreshBranch())
    } catch {
      this.project = null
    } finally {
      this.loaded = true
    }
  }

  async refreshBranch() {
    if (this.project) this.branch = await projectBranch().catch(() => null)
  }

  async refreshMissing() {
    if (this.project) this.missing = await projectMissingFolders().catch(() => [])
  }

  async refreshTrust() {
    if (this.project) this.untrusted = await trustCheck().catch(() => this.untrusted)
  }

  /** "Mi fido": il core mostra la conferma nativa. */
  async acceptTrust() {
    if (!this.project) return
    await trustAccept().catch(() => false)
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
