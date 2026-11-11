import type { UnlistenFn } from '@tauri-apps/api/event'
import { listenProject } from '../events'
import { searchCancel, searchReplace, searchStart } from '../ipc'
import { errorText } from '../errors'
import { editor } from './editor.svelte'
import type { ReplaceOutcome, SearchHit, SearchQuery, SearchSummary } from '../types'

export const DEBOUNCE_MS = 250
export type SearchStatus = 'idle' | 'running' | 'done' | 'error'
export type SearchGroup = { path: string; hits: SearchHit[] }

type HitsEvent = { id: number; hits: SearchHit[] }
type DoneEvent = { id: number; summary: SearchSummary }

const globs = (s: string) => s.split(',').map((g) => g.trim()).filter(Boolean)

/** Ricerca nel progetto della finestra: campi, opzioni, risultati raggruppati per file (ordine di arrivo). */
export class SearchStore {
  text = $state('')
  replacement = $state('')
  regex = $state(false)
  caseSensitive = $state(false)
  wholeWord = $state(false)
  include = $state('')
  exclude = $state('')
  status = $state<SearchStatus>('idle')
  error = $state<string | null>(null)
  truncated = $state(false)
  hitCount = $state(0)
  groups = $state.raw<SearchGroup[]>([])
  collapsed = $state<Record<string, boolean>>({})
  /** Esito dell'ultima sostituzione, per il messaggio sotto il campo. */
  outcome = $state<(ReplaceOutcome & { dirtySkipped: number }) | null>(null)
  replacing = $state(false)
  /** Incrementa quando va riportato il focus sul campo di ricerca (⌘⇧F). */
  focusTick = $state(0)

  private current: number | null = null
  private starting = false
  private buffered: (['hits', HitsEvent] | ['done', DoneEvent])[] = []
  private timer: ReturnType<typeof setTimeout> | null = null
  private index = new Map<string, number>()
  private unlisten: UnlistenFn[] = []
  private gen = 0

  get fileCount() {
    return this.groups.length
  }

  /** Si mette in ascolto degli eventi di ricerca di questo progetto (sulla sola finestra corrente). */
  async bind(projectId: string) {
    this.unbind()
    const gen = ++this.gen
    const subs = await Promise.all([
      listenProject<HitsEvent>('search:hits', projectId, (e) => this.onEvent(['hits', e.payload])),
      listenProject<DoneEvent>('search:done', projectId, (e) => this.onEvent(['done', e.payload])),
    ])
    if (gen !== this.gen) subs.forEach((u) => u())
    else this.unlisten = subs
  }

  unbind() {
    this.gen++
    this.unlisten.forEach((u) => u())
    this.unlisten = []
    if (this.timer) clearTimeout(this.timer)
    this.timer = null
  }

  query(): SearchQuery {
    return { text: this.text, regex: this.regex, caseSensitive: this.caseSensitive, wholeWord: this.wholeWord, include: globs(this.include), exclude: globs(this.exclude) }
  }

  /** Cambia il testo (o un'opzione) e ripianifica la ricerca dopo 250 ms; `now` = subito (testo preso dalla selezione). */
  schedule(now = false) {
    if (this.timer) clearTimeout(this.timer)
    this.timer = null
    this.outcome = null
    if (now) void this.run()
    else this.timer = setTimeout(() => void this.run(), DEBOUNCE_MS)
  }

  setText(text: string, now = false) {
    this.text = text
    this.schedule(now)
  }

  /** Annulla la ricerca in corso, svuota i risultati e (se c'è testo) ne avvia una nuova. */
  async run() {
    this.timer = null
    const old = this.current
    this.current = null
    if (old !== null) void searchCancel(old).catch(() => {})
    this.reset()
    if (!this.text) {
      this.status = 'idle'
      return
    }
    this.status = 'running'
    this.starting = true
    this.buffered = []
    const gen = this.gen
    try {
      const id = await searchStart(this.query())
      if (gen !== this.gen) return
      this.current = id
      this.starting = false
      const pending = this.buffered
      this.buffered = []
      pending.forEach((e) => this.onEvent(e))
    } catch (e) {
      this.starting = false
      this.status = 'error'
      this.error = errorText(e)
    }
  }

  private reset() {
    this.groups = []
    this.index = new Map()
    this.collapsed = {}
    this.hitCount = 0
    this.truncated = false
    this.error = null
  }

  /** Gli eventi possono arrivare prima della risposta di `search_start`: si tengono da parte e si filtrano per id. */
  private onEvent(e: ['hits', HitsEvent] | ['done', DoneEvent]) {
    if (this.starting) {
      this.buffered.push(e)
      return
    }
    if (e[1].id !== this.current) return
    if (e[0] === 'hits') return this.addHits(e[1].hits)
    const s = (e[1] as DoneEvent).summary
    this.truncated = s.truncated
    this.current = null
    if (s.error) {
      this.status = 'error'
      this.error = s.error
    } else this.status = 'done'
  }

  private addHits(hits: SearchHit[]) {
    const next = this.groups.slice()
    const touched = new Map<number, SearchHit[]>()
    for (const h of hits) {
      let i = this.index.get(h.path)
      if (i === undefined) {
        i = next.length
        this.index.set(h.path, i)
        next.push({ path: h.path, hits: [] })
      }
      const list = touched.get(i) ?? []
      list.push(h)
      touched.set(i, list)
    }
    for (const [i, list] of touched) next[i] = { path: next[i].path, hits: next[i].hits.concat(list) }
    this.groups = next
    this.hitCount += hits.length
  }

  toggleGroup(path: string) {
    this.collapsed[path] = !this.collapsed[path]
  }

  /** File con modifiche non salvate nell'editor: non si sostituisce lì. */
  private dirtyPaths() {
    return new Set(editor.tabs.filter((t) => t.dirty && !t.diff).map((t) => t.path))
  }

  /** Sostituisce in tutti i file dei risultati (esclusi quelli con modifiche non salvate); il core ricalcola e chiede conferma. */
  async replaceAll() {
    if (this.replacing || this.groups.length === 0) return
    const dirty = this.dirtyPaths()
    const paths = this.groups.map((g) => g.path).filter((p) => !dirty.has(p))
    const dirtySkipped = this.groups.length - paths.length
    if (paths.length === 0) {
      this.outcome = { files: 0, replacements: 0, skipped: [], cancelled: false, dirtySkipped }
      return
    }
    this.replacing = true
    try {
      const out = await searchReplace(this.query(), this.replacement, paths)
      this.outcome = { ...out, dirtySkipped }
      if (!out.cancelled) await this.run()
    } catch (e) {
      this.outcome = null
      this.status = 'error'
      this.error = errorText(e)
    } finally {
      this.replacing = false
    }
  }
}

export const search = new SearchStore()
