import { listenProject } from '../events'
import { asFsError, dbQueryClose, fsRead, fsWrite, sshRead, sshWrite } from '../ipc'
import { projectStore } from './project.svelte'
import type { DbKind, FileContent, FsChanged } from '../types'
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

/** Chiave di una tab: il percorso per i file, `diff:` + percorso per i diff (così un diff si apre accanto al file). */
export const tabKey = (path: string, diff: boolean) => (diff ? `diff:${path}` : path)

/** Chiave di una scheda di un file sul server: non collide mai con i percorsi locali. */
export const remoteKey = (serverId: string, path: string) => `ssh:${serverId}:${path}`

/** Sposta le tab il cui percorso è `from` o sta dentro `from` (cartella rinominata/spostata). */
export function repath<T extends { path: string; name: string; diff: unknown; key?: string; remote?: unknown }>(tabs: T[], from: string, to: string): T[] {
  return tabs.map((t) => {
    if (t.remote || (t.path !== from && !t.path.startsWith(from + '/'))) return t
    const path = to + t.path.slice(from.length)
    return { ...t, path, key: tabKey(path, !!t.diff), name: t.diff ? `${baseName(path)} (diff)` : baseName(path) }
  })
}

/** Sposta le schede di file del server `serverId` il cui percorso è `from` o sta dentro `from` (rinomina/spostamento
 *  sul server): cambiano percorso, chiave e nome, la versione resta (`mv` non la cambia). */
export function repathRemote<T extends { path: string; name: string; key: string; remote?: { serverId: string; path: string; version: string } }>(
  tabs: T[],
  serverId: string,
  from: string,
  to: string,
): T[] {
  return tabs.map((t) => {
    const r = t.remote
    if (!r || r.serverId !== serverId || (r.path !== from && !r.path.startsWith(from + '/'))) return t
    const path = to + r.path.slice(from.length)
    return { ...t, path, key: remoteKey(serverId, path), name: baseName(path), remote: { ...r, path } }
  })
}

/** Collegamento al modello Monaco: il testo si legge solo quando serve (salvataggio). */
export type Binding = { snapshot: () => { text: string; commit: () => void } }

/** Chiave di una scheda query: `db:<connessione>:<n>`; non collide con percorsi locali né `ssh:`. */
export const queryKey = (connectionId: string, n: number) => `db:${connectionId}:${n}`

/** Scheda query: a cosa è collegata. `persistKey` identifica il testo salvato (tabella/collezione o query nuova). */
export type QueryInfo = {
  connectionId: string
  kind: DbKind
  schema?: string
  table?: string
  database?: string
  collection?: string
  n: number
  persistKey: string
  /** Esegui subito all'apertura (apertura da una tabella/collezione); la scheda lo azzera dopo il primo avvio. */
  autoRun: boolean
  /** Risultato aperto nel core: lo si chiude alla chiusura della scheda. */
  resultId?: number
}

export type Tab = {
  id: number // stabile anche dopo rinomina
  key: string // identità della scheda (`tabKey`): file e diff dello stesso file sono schede diverse
  path: string
  name: string
  baseText: string // ultimo testo noto su disco (anche testo da caricare nel modello)
  rev: number // incrementa a ogni ricarica: il pannello rimette baseText nel modello
  dirty: boolean
  mtimeMs: number | null
  error: 'too_large' | 'not_text' | 'forbidden' | 'io' | 'remote_preview' | null
  /** Testo dell'ultimo errore (apertura o salvataggio di un file sul server). */
  message?: string
  /** Avviso su un file del server (es. eliminato dall'albero remoto): resta finché non si salva o ricarica. */
  notice?: string
  /** File sul server: `path` è il percorso remoto, `version` quella da ridare a `sshWrite`. */
  remote?: { serverId: string; path: string; version: string }
  prompt: boolean // "Cambiato su disco" in attesa di scelta
  /** `editable`: il lato modificato (cartella di lavoro) si può cambiare e salvare; il lato originale resta in sola lettura. */
  diff: { before: string; after: string; editable?: boolean } | null
  viewer: Viewer
  /** Scheda query database: il testo si salva da solo; è "modificata" solo con modifiche della griglia non applicate. */
  query?: QueryInfo
}

function baseName(p: string) {
  return p.split('/').filter(Boolean).pop() ?? p
}
let nextId = 1

class EditorStore {
  tabs = $state<Tab[]>([])
  /** Chiave della scheda attiva (`tabKey`). */
  active = $state<string | null>(null)
  /** Scheda con modifiche non salvate di cui si è chiesta la chiusura: barra "Salva / Scarta / Annulla". */
  closing = $state<string | null>(null)
  /** Schede con modifiche non salvate (il core lo sa, per chiedere prima di chiudere la finestra o uscire). */
  dirtyCount = $derived(this.tabs.filter((t) => t.dirty).length)
  split = $state(false)
  /** Posizione da mostrare appena la scheda `key` ha il suo modello (la consuma `EditorPane`). */
  reveal = $state<{ key: string; line: number; col: number } | null>(null)
  /** Testo selezionato nell'editor con il focus (lo imposta `EditorPane`); vuoto altrimenti. */
  activeSelection: (() => string) | null = null
  /** Modo di visualizzazione dei file markdown, ricordato per file finché l'app resta aperta. */
  private modes = $state<Record<string, MarkdownMode>>({})
  private reloadTimers = new Map<string, ReturnType<typeof setTimeout>>()
  private watching = false
  private bindings = new Map<number, Binding>()
  /** Salvataggio in corso per scheda (id) e quello in coda dietro di lui: due ⌘S di fila non partono insieme (sul
   *  server il secondo userebbe la versione vecchia e troverebbe un falso conflitto). */
  private saving = new Map<number, Promise<void>>()
  private queuedSave = new Map<number, Promise<void>>()

  get activeTab(): Tab | undefined {
    return this.tabs.find((t) => t.key === this.active)
  }

  modeOf(path: string): MarkdownMode {
    return this.modes[path] ?? 'preview'
  }

  setMode(path: string, mode: MarkdownMode) {
    this.modes[path] = mode
  }

  bind(id: number, b: Binding | null) {
    if (b) this.bindings.set(id, b)
    else this.bindings.delete(id)
  }

  setDirty(id: number, dirty: boolean) {
    const t = this.tabs.find((x) => x.id === id)
    if (t && t.dirty !== dirty) t.dirty = dirty
  }

  /** Scheda del file (non di un diff) con quel percorso. */
  private find(path: string) {
    return this.tabs.find((t) => t.key === tabKey(path, false))
  }

  private blank(path: string): Tab {
    return { id: nextId++, key: tabKey(path, false), path, name: baseName(path), baseText: '', rev: 0, dirty: false, mtimeMs: null, error: null, prompt: false, diff: null, viewer: viewerFor(baseName(path)) }
  }

  /** `asText`: apre come testo (es. un SVG da modificare) anche se il tipo avrebbe un'anteprima. */
  async open(path: string, asText = false) {
    const existing = this.find(path)
    if (existing && !(asText && existing.viewer !== 'text')) {
      this.active = existing.key
      return
    }
    // riaprire come testo un file già aperto: la scheda si sostituisce (non ha modifiche: era un'anteprima)
    const tab = this.blank(path)
    if (asText) tab.viewer = 'text'
    // immagini, PDF, audio/video e binari: i byte li legge il visualizzatore (fs_read_bytes), niente testo
    if (isBinaryViewer(tab.viewer)) {
      this.tabs = [...this.tabs.filter((t) => t.key !== tab.key), tab]
      this.active = tab.key
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
    this.tabs = [...this.tabs.filter((t) => t.key !== tab.key), tab]
    this.active = tab.key
  }

  private queryCount = new Map<string, number>()

  /** Apre una scheda query (da una tabella/collezione o vuota). La stessa tabella/collezione riporta in primo piano
   *  la scheda già aperta. */
  openQuery(req: { connectionId: string; kind: DbKind; schema?: string; table?: string; database?: string; collection?: string }) {
    const persistKey = req.table !== undefined ? `t:${req.schema ?? ''}.${req.table}` : req.collection !== undefined ? `c:${req.database ?? ''}.${req.collection}` : ''
    if (persistKey) {
      const existing = this.tabs.find((t) => t.query && t.query.connectionId === req.connectionId && t.query.persistKey === persistKey)
      if (existing) {
        this.active = existing.key
        return
      }
    }
    const n = (this.queryCount.get(req.connectionId) ?? 0) + 1
    this.queryCount.set(req.connectionId, n)
    const key = queryKey(req.connectionId, n)
    const name = req.table ?? req.collection ?? `Query ${n}`
    const query: QueryInfo = { ...req, n, persistKey: persistKey || `new${n}`, autoRun: !!persistKey }
    const tab: Tab = { ...this.blank(key), key, name, query }
    this.tabs = [...this.tabs, tab]
    this.active = key
  }

  /** Schede query: chi applica le modifiche della griglia in sospeso (dalla barra di chiusura). `true` = applicate. */
  private appliers = new Map<string, () => Promise<boolean>>()
  setApplier(key: string, apply: (() => Promise<boolean>) | undefined) {
    if (apply) this.appliers.set(key, apply)
    else this.appliers.delete(key)
  }

  setQueryResult(key: string, resultId: number | undefined) {
    const t = this.tabs.find((x) => x.key === key)
    if (t?.query) t.query.resultId = resultId
  }

  /** Apre un file del server (solo testo: anteprime e binari non sono disponibili da remoto). */
  async openRemote(serverId: string, path: string) {
    const key = remoteKey(serverId, path)
    const existing = this.tabs.find((t) => t.key === key)
    if (existing) {
      this.active = key
      return
    }
    const name = baseName(path)
    let viewer = viewerFor(name)
    if (viewer === 'markdown') viewer = 'text'
    const tab: Tab = { ...this.blank(path), key, viewer, remote: { serverId, path, version: '' } }
    if (isBinaryViewer(viewer)) {
      tab.error = 'remote_preview'
      tab.message = 'Anteprima non disponibile per i file remoti'
    } else {
      try {
        const c = await sshRead(serverId, path)
        tab.baseText = c.text
        tab.remote = { serverId, path, version: c.version }
      } catch (e) {
        const err = asFsError(e)
        tab.error = err.code === 'too_large' || err.code === 'not_text' ? err.code : 'io'
        tab.message = err.message
      }
    }
    this.tabs = [...this.tabs, tab]
    this.active = key
  }

  /** Salva un file del server: su `conflict` mostra la barra, su altri errori (es. connessione caduta) la scheda resta modificata. */
  private async saveRemote(t: Tab, text: string, commit: () => void) {
    const r = t.remote!
    try {
      const version = await sshWrite(r.serverId, r.path, text, r.version)
      t.remote = { ...r, version }
      t.baseText = text
      t.message = undefined
      t.notice = undefined
      t.prompt = false
      commit()
    } catch (e) {
      const err = asFsError(e)
      if (err.code === 'conflict') t.prompt = true
      else t.message = err.message
    }
  }

  /** Apre il file e porta il cursore su riga e colonna (1-based, come Monaco). I markdown si aprono in "Modifica". */
  async openAt(path: string, line: number, col: number) {
    this.reveal = null
    if (viewerFor(baseName(path)) === 'markdown') this.setMode(path, 'edit')
    await this.open(path)
    const t = this.find(path)
    if (t && !t.error) this.reveal = { key: t.key, line, col }
  }

  /** Diff in una scheda propria accanto al file (una sola per file): la scheda del file, anche con modifiche, resta.
   *  `editable`: il lato modificato si può cambiare e salvare (con controllo dell'mtime, come un file); se il file non si
   *  riesce a leggere come testo, il diff resta in sola lettura. */
  async openDiff(path: string, before: string, after: string, opts: { editable?: boolean } = {}) {
    // un diff con modifiche non salvate non si sostituisce (le perderebbe): si riporta in primo piano
    const open = this.tabs.find((t) => t.key === tabKey(path, true))
    if (open?.dirty) {
      this.active = open.key
      return
    }
    let mtimeMs: number | null = null
    let editable = false
    if (opts.editable) {
      try {
        const c = await fsRead(path)
        after = c.text
        mtimeMs = c.mtimeMs
        editable = true
      } catch {
        /* file sparito o non testo: solo lettura */
      }
    }
    const tab: Tab = { ...this.blank(path), key: tabKey(path, true), name: `${baseName(path)} (diff)`, baseText: after, mtimeMs, diff: editable ? { before, after, editable } : { before, after } }
    this.tabs = [...this.tabs.filter((t) => t.key !== tab.key), tab]
    this.active = tab.key
  }

  /** Salva la tab; su conflitto di mtime (o file sparito) mostra la barra "Cambiato su disco". Un salvataggio chiesto
   *  mentre un altro della stessa scheda è in corso parte dopo di lui (con il testo di quel momento); più richieste
   *  nel frattempo diventano una sola. */
  save(path: string, key = tabKey(path, false)): Promise<void> {
    const t = this.tabs.find((x) => x.key === key)
    if (!t) return Promise.resolve()
    const id = t.id
    const queued = this.queuedSave.get(id)
    if (queued) return queued
    const running = this.saving.get(id)
    if (!running) return this.startSave(id)
    const next = running
      .catch(() => {})
      .then(() => {
        this.queuedSave.delete(id)
        return this.startSave(id)
      })
    this.queuedSave.set(id, next)
    return next
  }

  private startSave(id: number): Promise<void> {
    const p: Promise<void> = this.saveNow(id).finally(() => {
      if (this.saving.get(id) === p) this.saving.delete(id)
    })
    this.saving.set(id, p)
    return p
  }

  private async saveNow(id: number) {
    const t = this.tabs.find((x) => x.id === id)
    if (!t || t.query || (t.diff && !t.diff.editable) || t.error || isBinaryViewer(t.viewer) || projectStore.untrusted.length > 0) return // sola lettura
    const snap = this.bindings.get(t.id)?.snapshot()
    if (!snap) return
    if (t.remote) return this.saveRemote(t, snap.text, snap.commit)
    try {
      t.mtimeMs = await fsWrite(t.path, snap.text, t.mtimeMs)
      t.baseText = snap.text
      snap.commit()
      t.prompt = false
    } catch (e) {
      if (asFsError(e).code === 'conflict') t.prompt = true
      else throw e
    }
  }

  /** Chiude la scheda (chiave) senza chiedere. */
  close(key: string) {
    const i = this.tabs.findIndex((t) => t.key === key)
    if (i < 0) return
    const t = this.tabs[i]
    this.tabs = this.tabs.filter((x) => x.key !== key)
    this.appliers.delete(key)
    if (t.query?.resultId !== undefined) void Promise.resolve(dbQueryClose(t.query.resultId)).catch(() => {})
    if (this.closing === key) this.closing = null
    if (!t.diff) {
      clearTimeout(this.reloadTimers.get(t.path))
      this.reloadTimers.delete(t.path)
    }
    if (this.active === key) this.active = this.tabs[Math.min(i, this.tabs.length - 1)]?.key ?? null
  }

  /** Chiusura chiesta dall'utente (× o clic centrale): con modifiche non salvate prima la barra di conferma. */
  requestClose(key: string) {
    const t = this.tabs.find((x) => x.key === key)
    if (!t) return
    if (!t.dirty) return this.close(key)
    this.active = key
    this.closing = key
  }

  /** Risposta alla barra "Modifiche non salvate": salva e chiude (se il salvataggio riesce), scarta e chiude, o annulla. */
  async resolveClose(choice: 'save' | 'discard' | 'cancel') {
    const key = this.closing
    const t = this.tabs.find((x) => x.key === key)
    if (!key || !t || choice === 'cancel') {
      this.closing = null
      return
    }
    if (choice === 'save' && t.query) {
      // scheda query: "Applica" le modifiche della griglia (conferma nativa del core); rifiutata o fallita = la scheda resta
      const apply = this.appliers.get(key)
      if (!apply || !(await apply())) return
    } else if (choice === 'save') {
      try {
        await this.save(t.path, t.key)
      } catch {
        return // errore di scrittura: la scheda resta, con la barra
      }
      if (t.dirty || t.prompt) return // conflitto o sola lettura: la scheda resta
    }
    this.close(key)
  }

  /** Chiude le tab di un file o di tutti i file dentro una cartella (dopo "sposta nel Cestino"). */
  closeUnder(path: string) {
    for (const t of [...this.tabs]) if (!t.remote && (t.path === path || t.path.startsWith(path + '/'))) this.close(t.key)
  }

  /** Dopo rinomina/spostamento sul server: le schede di quei file seguono il nuovo percorso. */
  repathRemote(serverId: string, from: string, to: string) {
    const active = this.activeTab
    this.tabs = repathRemote(this.tabs, serverId, from, to)
    if (active) this.active = this.tabs.find((t) => t.id === active.id)?.key ?? this.active
  }

  /** Dopo un'eliminazione sul server: le schede di quei file restano con il loro testo, modificate, senza versione
   *  (salvare ricrea il file) e con un avviso. */
  markRemoteRemoved(serverId: string, path: string) {
    for (const t of this.tabs) {
      const r = t.remote
      if (!r || r.serverId !== serverId || (r.path !== path && !r.path.startsWith(path + '/'))) continue
      t.remote = { ...r, version: '' }
      t.dirty = true
      t.prompt = false
      t.notice = 'Eliminato sul server: salvando lo ricrei (se la cartella esiste ancora)'
    }
  }

  /** Dopo rinomina/spostamento: le tab seguono il nuovo percorso. */
  repath(from: string, to: string) {
    const active = this.activeTab
    this.tabs = repath(this.tabs, from, to)
    if (active) this.active = this.tabs.find((t) => t.id === active.id)?.key ?? this.active
  }

  /** Scarta le modifiche locali e ricarica da disco. */
  async reload(path: string, key = tabKey(path, false)) {
    const t = this.tabs.find((x) => x.key === key)
    if (!t || isBinaryViewer(t.viewer)) return
    if (t.remote) {
      try {
        const c = await sshRead(t.remote.serverId, t.remote.path)
        t.baseText = c.text
        t.remote = { ...t.remote, version: c.version }
        t.rev++
        t.dirty = false
        t.prompt = false
        t.message = undefined
        t.notice = undefined
      } catch (e) {
        t.message = asFsError(e).message
      }
      return
    }
    try {
      const c = await fsRead(path)
      t.baseText = c.text
      if (t.diff) t.diff = { ...t.diff, after: c.text }
      t.mtimeMs = c.mtimeMs
      t.rev++
      t.dirty = false
      t.prompt = false
    } catch {
      // file sparito: resta la barra, l'utente può "Tieni il mio" per ricrearlo
    }
  }

  /** Chiude la barra di conflitto senza scegliere: le modifiche restano nella scheda. */
  dismissPrompt(key: string) {
    const t = this.tabs.find((x) => x.key === key)
    if (t) t.prompt = false
  }

  /** Tiene il testo locale: accetta lo stato attuale del disco così il prossimo salvataggio sovrascrive. */
  async keepMine(path: string, key = tabKey(path, false)) {
    const t = this.tabs.find((x) => x.key === key)
    if (!t) return
    if (t.remote) {
      // sovrascrivi: si prende la versione attuale del server e si riscrive il testo locale
      const snap = this.bindings.get(t.id)?.snapshot()
      if (!snap) return
      try {
        const c = await sshRead(t.remote.serverId, t.remote.path)
        t.remote = { ...t.remote, version: c.version }
      } catch (e) {
        t.message = asFsError(e).message
        return
      }
      return this.saveRemote(t, snap.text, snap.commit)
    }
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
      if (t.remote || t.query || (t.diff && !t.diff.editable) || !set.has(t.path)) continue
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
        if (t.diff) t.diff = { ...t.diff, after: disk.text }
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
