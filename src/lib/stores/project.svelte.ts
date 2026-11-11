import { listen } from '@tauri-apps/api/event'
import { projectBranch, projectCurrent, projectMissingFolders, projectRelocate, projectRename } from '../ipc'
import type { Project } from '../types'

class ProjectStore {
  project = $state<Project | null>(null)
  branch = $state<string | null>(null)
  missing = $state<string[]>([])
  loaded = $state(false)

  /** Carica il progetto della finestra; nella finestra "Apri cartella" resta null. */
  async load() {
    try {
      this.project = await projectCurrent()
      await Promise.all([this.refreshBranch(), this.refreshMissing()])
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
    if (this.project) this.branch = await projectBranch(this.project.id).catch(() => null)
  }

  async refreshMissing() {
    if (this.project) this.missing = await projectMissingFolders(this.project.id).catch(() => [])
  }

  async rename(name: string) {
    if (!this.project) return
    this.project = await projectRename(this.project.id, name)
  }

  async relocate(old: string, newPath: string) {
    if (!this.project) return
    this.project = await projectRelocate(this.project.id, old, newPath)
    await Promise.all([this.refreshBranch(), this.refreshMissing()])
  }
}

export const projectStore = new ProjectStore()
