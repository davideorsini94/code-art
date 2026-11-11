import { listenProject } from '../events'
import { asFsError, fsRead, fsWrite } from '../ipc'
import { projectStore } from './project.svelte'
import type { FileContent, FsChanged } from '../types'
import { previewPolicy } from '../md-policy'
import { isBinaryViewer, viewerFor, type MarkdownMode, type Viewer } from '../viewers'

export type DiskAction = 'reload' | 'prompt' | 'none'

/**
 * Cosa fare quando il file cambia su disco. `baseText` è l'ultimo testo noto su disco,
 * `dirty` dice se l'editor ha modifiche locali: il testo dell'editor non viene mai confrontato.
 */
export function onDiskChange(tab: { dirty: boolean; baseText: string }, disk: FileContent): DiskAction {
  if (disk.text === tab.baseText) return 'none'
  return tab.dirty ? 'prompt' : 'reload'
}

/** Sposta le tab il cui percorso è `from` o sta dentro `from` (cartella rinominata/spostata). */
export function repath<T extends { path: string; name: string; diff: unknown }>(tabs: T[], from: string, to: string): T[] {
  return tabs.map((t) => {
    if (t.path !== from && !t.path.startsWith(from + '/')) return t
    const path = to + t.path.slice(from.length)
    return { ...t, path, name: t.diff ? `${baseName(path)} (diff)` : baseName(path) }
  })
}

/** Collegamento al modello Monaco: il testo si legge solo quando serve (salvataggio). */
export type Binding = { snapshot: () => { text: string; commit: () => void } }

export type Tab = {
  id: number // stabile anche dopo rinomina
  path: string
  name: string
  baseText: string // ultimo testo noto su disco (anche testo da caricare nel modello)
  rev: number // incrementa a ogni ricarica: il pannello rimette baseText nel modello
  dirty: boolean
  mtimeMs: number | null
  error: 'too_large' | 'not_text' | 'forbidden' | 'io' | null
  prompt: boolean // "Cambiato su disco" in attesa di scelta
  diff: { before: string; after: string } | null
  viewer: Viewer
}

function baseName(p: string) {
  return p.split('/').filter(Boolean).pop() ?? p
}
let nextId = 1

class EditorStore {
  tabs = $state<Tab[]>([])
  active = $state<string | null>(null)
  split = $state(false)
  /** Modo di visualizzazione dei file markdown, ricordato per file finché l'app resta aperta. */
  private modes = $state<Record<string, MarkdownMode>>({})
  private reloadTimers = new Map<string, ReturnType<typeof setTimeout>>()
  private watching = false
  private bindings = new Map<number, Binding>()

  get activeTab(): Tab | undefined {
    return this.tabs.find((t) => t.path === this.active)
  }

  modeOf(path: string): MarkdownMode {
    return this.modes[path] ?? 'preview'
  }

  setMode(path: string, mode: MarkdownMode) {
    this.modes[path] = mode
  }

  isDirty(t: Tab) {
    return t.dirty
  }

  bind(id: number, b: Binding | null) {
    if (b) this.bindings.set(id, b)
    else this.bindings.delete(id)
  }

  setDirty(id: number, dirty: boolean) {
    const t = this.tabs.find((x) => x.id === id)
    if (t && t.dirty !== dirty) t.dirty = dirty
  }

  private find(path: string) {
    return this.tabs.find((t) => t.path === path)
  }

  private blank(path: string): Tab {
    return { id: nextId++, path, name: baseName(path), baseText: '', rev: 0, dirty: false, mtimeMs: null, error: null, prompt: false, diff: null, viewer: viewerFor(baseName(path)) }
  }

  /** `asText`: apre come testo (es. un SVG da modificare) anche se il tipo avrebbe un'anteprima. */
  async open(path: string, asText = false) {
    const existing = this.find(path)
    if (existing && !existing.diff && !(asText && existing.viewer !== 'text')) {
      this.active = path
      return
    }
    const tab = this.blank(path)
    if (asText) tab.viewer = 'text'
    // immagini, PDF, audio/video e binari: i byte li legge il visualizzatore (fs_read_bytes), niente testo
    if (isBinaryViewer(tab.viewer)) {
      this.tabs = [...this.tabs.filter((t) => t.path !== path), tab]
      this.active = path
      return
    }
    try {
      const c = await fsRead(path)
      tab.baseText = c.text
      tab.mtimeMs = c.mtimeMs
      // documento markdown grande: si apre in "Modifica" (l'anteprima si chiede a mano)
      if (tab.viewer === 'markdown' && !(path in this.modes) && previewPolicy(c.text.length).large) this.modes[path] = 'edit'
    } catch (e) {
      const err = asFsError(e)
      tab.error = err.code === 'too_large' || err.code === 'not_text' || err.code === 'forbidden' ? err.code : 'io'
    }
    this.tabs = [...this.tabs.filter((t) => t.path !== path), tab]
    this.active = path
  }

  openDiff(path: string, before: string, after: string) {
    const tab: Tab = { ...this.blank(path), name: `${baseName(path)} (diff)`, baseText: after, diff: { before, after } }
    this.tabs = [...this.tabs.filter((t) => t.path !== path), tab]
    this.active = path
  }

  /** Salva la tab; su conflitto di mtime (o file sparito) mostra la barra "Cambiato su disco". */
  async save(path: string) {
    const t = this.find(path)
    if (!t || t.diff || t.error || isBinaryViewer(t.viewer) || projectStore.untrusted.length > 0) return // sola lettura
    const snap = this.bindings.get(t.id)?.snapshot()
    if (!snap) return
    try {
      t.mtimeMs = await fsWrite(path, snap.text, t.mtimeMs)
      t.baseText = snap.text
      snap.commit()
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
    clearTimeout(this.reloadTimers.get(path))
    this.reloadTimers.delete(path)
    if (this.active === path) this.active = this.tabs[Math.min(i, this.tabs.length - 1)]?.path ?? null
  }

  /** Chiude le tab di un file o di tutti i file dentro una cartella (dopo "sposta nel Cestino"). */
  closeUnder(path: string) {
    for (const t of [...this.tabs]) if (t.path === path || t.path.startsWith(path + '/')) this.close(t.path)
  }

  /** Dopo rinomina/spostamento: le tab seguono il nuovo percorso. */
  repath(from: string, to: string) {
    this.tabs = repath(this.tabs, from, to)
    if (this.active && (this.active === from || this.active.startsWith(from + '/'))) {
      this.active = to + this.active.slice(from.length)
    }
  }

  /** Scarta le modifiche locali e ricarica da disco. */
  async reload(path: string) {
    const t = this.find(path)
    if (!t || isBinaryViewer(t.viewer)) return
    try {
      const c = await fsRead(path)
      t.baseText = c.text
      t.mtimeMs = c.mtimeMs
      t.rev++
      t.dirty = false
      t.prompt = false
    } catch {
      // file sparito: resta la barra, l'utente può "Tieni il mio" per ricrearlo
    }
  }

  /** Tiene il testo locale: accetta lo stato attuale del disco così il prossimo salvataggio sovrascrive. */
  async keepMine(path: string) {
    const t = this.find(path)
    if (!t) return
    try {
      const c = await fsRead(path)
      t.mtimeMs = c.mtimeMs
      t.baseText = c.text
    } catch {
      t.mtimeMs = null // file sparito: il salvataggio lo ricrea
    }
    t.prompt = false
  }

  async applyChanged(paths: string[]) {
    const set = new Set(paths)
    for (const t of this.tabs) {
      if (t.diff || !set.has(t.path)) continue
      if (isBinaryViewer(t.viewer)) {
        this.scheduleViewerReload(t.path)
        continue
      }
      if (t.error) continue
      let disk: FileContent
      try {
        disk = await fsRead(t.path)
      } catch {
        continue
      }
      const action = onDiskChange(t, disk)
      if (action === 'reload') {
        t.baseText = disk.text
        t.mtimeMs = disk.mtimeMs
        t.rev++
      } else if (action === 'prompt') {
        t.prompt = true
      } else if (!t.dirty) {
        t.mtimeMs = disk.mtimeMs
      }
    }
  }

  /** Anteprime di byte: a ogni cambiamento su disco il visualizzatore rilegge (con attesa, per non rileggere a raffica). */
  private scheduleViewerReload(path: string) {
    clearTimeout(this.reloadTimers.get(path))
    this.reloadTimers.set(
      path,
      setTimeout(() => {
        this.reloadTimers.delete(path)
        const t = this.find(path)
        if (t) t.rev++
      }, 250),
    )
  }

  /** Ascolta `fs:changed` del progetto della finestra (una sola volta). */
  watch(projectId: string) {
    if (this.watching) return
    this.watching = true
    void listenProject<FsChanged>('fs:changed', projectId, (e) => void this.applyChanged(e.payload.paths))
  }
}

export const editor = new EditorStore()
