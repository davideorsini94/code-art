import { listen } from '@tauri-apps/api/event'
import { asFsError, fsRead, fsWrite } from '../ipc'
import type { FileContent, FsChanged } from '../types'

export type DiskAction = 'reload' | 'prompt' | 'none'

/** Cosa fare quando il file cambia su disco, data la tab aperta. */
export function onDiskChange(tab: { text: string; savedText: string }, disk: FileContent): DiskAction {
  if (disk.text === tab.savedText || disk.text === tab.text) return 'none'
  return tab.text !== tab.savedText ? 'prompt' : 'reload'
}

export type Tab = {
  path: string
  name: string
  text: string // contenuto corrente nell'editor
  savedText: string // ultimo contenuto noto su disco
  mtimeMs: number | null
  error: 'too_large' | 'not_text' | 'io' | null
  prompt: boolean // "Cambiato su disco" in attesa di scelta
  diff: { before: string; after: string } | null
}

const baseName = (p: string) => p.split('/').filter(Boolean).pop() ?? p

class EditorStore {
  tabs = $state<Tab[]>([])
  active = $state<string | null>(null)
  split = $state(false)
  private watching = false

  get activeTab(): Tab | undefined {
    return this.tabs.find((t) => t.path === this.active)
  }

  isDirty(t: Tab) {
    return !t.diff && !t.error && t.text !== t.savedText
  }

  private find(path: string) {
    return this.tabs.find((t) => t.path === path)
  }

  async open(path: string) {
    const existing = this.find(path)
    if (existing && !existing.diff) {
      this.active = path
      return
    }
    const tab: Tab = { path, name: baseName(path), text: '', savedText: '', mtimeMs: null, error: null, prompt: false, diff: null }
    try {
      const c = await fsRead(path)
      tab.text = tab.savedText = c.text
      tab.mtimeMs = c.mtimeMs
    } catch (e) {
      const err = asFsError(e)
      tab.error = err.code === 'too_large' || err.code === 'not_text' ? err.code : 'io'
    }
    this.tabs = [...this.tabs.filter((t) => t.path !== path), tab]
    this.active = path
  }

  openDiff(path: string, before: string, after: string) {
    const tab: Tab = {
      path,
      name: `${baseName(path)} (diff)`,
      text: after,
      savedText: after,
      mtimeMs: null,
      error: null,
      prompt: false,
      diff: { before, after },
    }
    this.tabs = [...this.tabs.filter((t) => t.path !== path), tab]
    this.active = path
  }

  edit(path: string, text: string) {
    const t = this.find(path)
    if (t) t.text = text
  }

  /** Salva la tab; su conflitto di mtime mostra la barra "Cambiato su disco". */
  async save(path: string) {
    const t = this.find(path)
    if (!t || t.diff || t.error) return
    try {
      const text = t.text
      t.mtimeMs = await fsWrite(path, text, t.mtimeMs)
      t.savedText = text
      t.prompt = false
    } catch (e) {
      if (asFsError(e).code === 'conflict') t.prompt = true
      else throw e
    }
  }

  close(path: string) {
    const i = this.tabs.findIndex((t) => t.path === path)
    if (i < 0) return
    this.tabs = this.tabs.filter((t) => t.path !== path)
    if (this.active === path) this.active = this.tabs[Math.min(i, this.tabs.length - 1)]?.path ?? null
  }

  /** Scarta le modifiche locali e ricarica da disco. */
  async reload(path: string) {
    const t = this.find(path)
    if (!t) return
    const c = await fsRead(path)
    t.text = t.savedText = c.text
    t.mtimeMs = c.mtimeMs
    t.prompt = false
  }

  /** Tiene il testo locale: accetta l'mtime attuale del disco così il prossimo salvataggio sovrascrive. */
  async keepMine(path: string) {
    const t = this.find(path)
    if (!t) return
    const c = await fsRead(path)
    t.mtimeMs = c.mtimeMs
    t.savedText = c.text
    t.prompt = false
  }

  async applyChanged(paths: string[]) {
    const set = new Set(paths)
    for (const t of this.tabs) {
      if (t.diff || t.error || !set.has(t.path)) continue
      let disk: FileContent
      try {
        disk = await fsRead(t.path)
      } catch {
        continue
      }
      const action = onDiskChange(t, disk)
      if (action === 'reload') {
        t.text = t.savedText = disk.text
        t.mtimeMs = disk.mtimeMs
      } else if (action === 'prompt') {
        t.prompt = true
      } else if (!this.isDirty(t)) {
        t.mtimeMs = disk.mtimeMs
      }
    }
  }

  /** Ascolta `fs:changed` (una sola volta per finestra). */
  watch() {
    if (this.watching) return
    this.watching = true
    void listen<FsChanged>('fs:changed', (e) => void this.applyChanged(e.payload.paths))
  }
}

export const editor = new EditorStore()
