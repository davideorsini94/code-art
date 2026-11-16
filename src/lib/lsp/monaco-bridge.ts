/** Ponte tra Monaco e i language server: sincronizzazione dei documenti, completamento, hover, firma, definizione e
 *  diagnostica. Un server per linguaggio (il core ne tiene uno per progetto e linguaggio e lo riavvia da solo). Tutto
 *  ciò che sta fuori da Monaco e dal core entra come dipendenza (`BridgeDeps`), così il ponte si prova con un server finto. */
import type * as M from 'monaco-editor'
import { LspClient } from './client'
import { LANG_LABEL, LSP_LANGS, languageForPath, type LspLang } from './languages'

export type LspStatus = 'avvio' | 'pronto' | 'fermo'

/** Il canale verso il core (comandi `lsp_*` ed eventi `lsp:*`). */
export interface LspApi {
  start(lang: LspLang): Promise<{ id: number; rootUri: string }>
  send(id: number, message: string): Promise<void>
  stop(id: number): Promise<void>
  onMessage(cb: (id: number, message: string) => void): Promise<() => void>
  onExit(cb: (id: number, code: number) => void): Promise<() => void>
}

export interface BridgeDeps {
  monaco: Pick<typeof M, 'languages' | 'editor' | 'Uri'>
  api: LspApi
  /** Cartelle del progetto: nessun file fuori da qui viene aperto. */
  folders: () => string[]
  /** Apre il file nell'editor sulla riga e colonna (1-based). */
  openFile: (path: string, line: number, col: number) => void
  /** Markdown non fidato -> HTML sanificato (`renderMarkdown`). */
  render: (markdown: string) => string
  onStatus: (lang: LspLang, status: LspStatus, detail?: string) => void
  /** Riga informativa (server mancante, progetto non fidato): il ponte la chiede una sola volta per linguaggio. */
  notify: (lang: LspLang, text: string) => void
  debounceMs?: number
}

const OWNER = 'codeart-lsp'
const CHANGE_DEBOUNCE_MS = 150
/** `initialize` può essere lento (jdtls indicizza il progetto): molto più del timeout delle richieste. */
const INIT_TIMEOUT_MS = 90_000

type Doc = {
  model: M.editor.ITextModel
  path: string
  uri: string
  lang: LspLang
  version: number
  opened: boolean
  timer: ReturnType<typeof setTimeout> | undefined
  subs: { dispose(): void }[]
}

type Session = {
  lang: LspLang
  state: LspStatus
  id: number
  rootUri: string
  client: LspClient | null
  caps: any
  gen: number
  starting: Promise<void> | null
}

const errText = (e: unknown) => (typeof e === 'string' ? e : e instanceof Error ? e.message : String(e))

/** URI `file:` locale -> percorso decodificato; null per ogni altro URI. */
export function fileUriToPath(uri: string): string | null {
  try {
    const u = new URL(uri)
    if (u.protocol !== 'file:' || (u.hostname !== '' && u.hostname !== 'localhost')) return null
    return decodeURIComponent(u.pathname)
  } catch {
    return null
  }
}

/** URI `file:` di un percorso con la codifica del core (crate `url`, come `rootUri`): spazi e accenti in percentuale. */
export function pathToFileUri(path: string): string {
  const u = new URL('file:///')
  u.pathname = path
  return u.href
}

const lspPos = (p: { lineNumber: number; column: number }) => ({ line: p.lineNumber - 1, character: p.column - 1 })
const monRange = (r: any): M.IRange => ({ startLineNumber: r.start.line + 1, startColumn: r.start.character + 1, endLineNumber: r.end.line + 1, endColumn: r.end.character + 1 })

// LSP CompletionItemKind (1..25) -> monaco.languages.CompletionItemKind
const KIND = [18, 0, 1, 2, 3, 4, 5, 7, 8, 9, 12, 13, 15, 17, 28, 19, 20, 21, 23, 16, 14, 6, 10, 11, 24]
const SEVERITY: Record<number, number> = { 1: 8, 2: 4, 3: 2, 4: 1 } // Error, Warning, Info, Hint

function fenced(code: string, lang = ''): string {
  const longest = Math.max(0, ...(code.match(/`+/g) ?? []).map((r) => r.length))
  const fence = '`'.repeat(Math.max(3, longest + 1))
  return `${fence}${lang}\n${code}\n${fence}`
}

/** Un contenuto LSP (`MarkedString` o `MarkupContent`) come sorgente markdown. */
function markdownSource(c: any): string {
  if (typeof c === 'string') return c
  if (!c || typeof c.value !== 'string') return ''
  if (typeof c.language === 'string') return fenced(c.value, c.language)
  return c.kind === 'plaintext' ? fenced(c.value) : c.value
}

export class LspBridge {
  private sessions = new Map<LspLang, Session>()
  private byId = new Map<number, Session>()
  private docs = new Map<string, Doc>()
  private notified = new Set<LspLang>()
  private disposables: { dispose(): void }[] = []
  private offs: (() => void)[] = []
  private listening: Promise<void>
  private readonly debounce: number

  constructor(private readonly deps: BridgeDeps) {
    this.debounce = deps.debounceMs ?? CHANGE_DEBOUNCE_MS
    this.listening = Promise.all([
      deps.api.onMessage((id, message) => this.byId.get(id)?.client?.handle(message)),
      deps.api.onExit((id) => this.onExit(id)),
    ]).then((offs) => {
      this.offs = offs
    })
    this.listening.catch(() => {}) // se l'ascolto fallisce l'errore esce da startSession/dispose, non come rifiuto non gestito
    this.register()
  }

  // ---------- documenti ----------

  /** Un modello di file normale (mai il lato originale di un diff) di un linguaggio con server. */
  attach(model: M.editor.ITextModel, path: string): void {
    const lang = languageForPath(path)
    if (!lang || model.uri.scheme !== 'file') return
    const key = model.uri.path
    if (this.docs.has(key) || !this.inProject(key)) return
    const doc: Doc = { model, path: key, uri: pathToFileUri(key), lang, version: 1, opened: false, timer: undefined, subs: [] }
    this.docs.set(key, doc)
    doc.subs.push(model.onDidChangeContent(() => this.scheduleChange(doc)), model.onWillDispose(() => this.detach(model)))
    const s = this.sessions.get(lang)
    if (s?.state === 'pronto') this.openDoc(s, doc)
    else if (!s || (s.state === 'fermo' && s.id === 0)) void this.startSession(lang)
  }

  detach(model: M.editor.ITextModel): void {
    const doc = this.docs.get(model.uri.path)
    if (!doc || doc.model !== model) return
    clearTimeout(doc.timer)
    doc.subs.forEach((d) => d.dispose())
    this.docs.delete(doc.path)
    const s = this.sessions.get(doc.lang)
    if (doc.opened && s?.state === 'pronto') s.client?.notify('textDocument/didClose', { textDocument: { uri: doc.uri } })
    this.deps.monaco.editor.setModelMarkers(model, OWNER, [])
  }

  didSave(model: M.editor.ITextModel): void {
    const doc = this.docs.get(model.uri.path)
    const s = doc && this.sessions.get(doc.lang)
    if (!doc || !doc.opened || s?.state !== 'pronto') return
    this.flush(doc)
    s.client?.notify('textDocument/didSave', { textDocument: { uri: doc.uri }, text: model.getValue() })
  }

  private openDoc(s: Session, doc: Doc) {
    if (doc.opened) return
    doc.opened = true
    doc.version = 1
    s.client?.notify('textDocument/didOpen', { textDocument: { uri: doc.uri, languageId: doc.lang, version: doc.version, text: doc.model.getValue() } })
  }

  private scheduleChange(doc: Doc) {
    if (!doc.opened) return // il didOpen porta già il testo corrente
    clearTimeout(doc.timer)
    doc.timer = setTimeout(() => this.flush(doc), this.debounce)
  }

  /** Invia subito il `didChange` in attesa (prima di ogni richiesta, perché il server veda il testo corrente). */
  private flush(doc: Doc) {
    if (doc.timer === undefined) return
    clearTimeout(doc.timer)
    doc.timer = undefined
    const s = this.sessions.get(doc.lang)
    if (!doc.opened || s?.state !== 'pronto') return
    doc.version++
    s.client?.notify('textDocument/didChange', { textDocument: { uri: doc.uri, version: doc.version }, contentChanges: [{ text: doc.model.getValue() }] })
  }

  // ---------- sessioni ----------

  private status(s: Session, state: LspStatus, detail?: string) {
    s.state = state
    this.deps.onStatus(s.lang, state, detail)
  }

  private startSession(lang: LspLang): Promise<void> {
    const prev = this.sessions.get(lang)
    if (prev?.starting) return prev.starting
    const s: Session = prev ?? { lang, state: 'avvio', id: 0, rootUri: '', client: null, caps: {}, gen: 0, starting: null }
    this.sessions.set(lang, s)
    this.status(s, 'avvio')
    s.starting = (async () => {
      try {
        await this.listening
        const r = await this.deps.api.start(lang)
        s.id = r.id
        s.rootUri = r.rootUri
        s.client = new LspClient((m) => this.deps.api.send(r.id, m))
        s.client.onNotification('textDocument/publishDiagnostics', (p) => this.diagnostics(p))
        this.byId.set(r.id, s)
      } catch (e) {
        const text = errText(e)
        this.status(s, 'fermo', text)
        if (!this.notified.has(lang)) {
          this.notified.add(lang)
          this.deps.notify(lang, text)
        }
        return
      }
      await this.handshake(s)
    })().finally(() => {
      s.starting = null
    })
    return s.starting
  }

  private async handshake(s: Session): Promise<void> {
    const client = s.client!
    const gen = ++s.gen
    if (s.state !== 'avvio') this.status(s, 'avvio')
    for (const d of this.docs.values()) if (d.lang === s.lang) d.opened = false
    try {
      const r: any = await client.request('initialize', this.initParams(s), INIT_TIMEOUT_MS)
      if (gen !== s.gen) return
      s.caps = r?.capabilities ?? {}
      client.notify('initialized', {})
      this.status(s, 'pronto')
      for (const d of [...this.docs.values()]) if (d.lang === s.lang) this.openDoc(s, d)
    } catch (e) {
      if (gen !== s.gen) return
      this.status(s, 'fermo', errText(e))
    }
  }

  private initParams(s: Session) {
    const name = s.rootUri.replace(/\/+$/, '').split('/').pop() ?? ''
    const md = ['markdown', 'plaintext']
    return {
      processId: null,
      clientInfo: { name: 'Code Art' },
      rootUri: s.rootUri,
      workspaceFolders: [{ uri: s.rootUri, name }],
      capabilities: {
        general: { positionEncodings: ['utf-16'] },
        workspace: { configuration: true, workspaceFolders: true },
        textDocument: {
          synchronization: { didSave: true, dynamicRegistration: false },
          completion: {
            completionItem: { snippetSupport: true, documentationFormat: md, deprecatedSupport: true, resolveSupport: { properties: ['documentation', 'detail', 'additionalTextEdits'] } },
          },
          hover: { contentFormat: md },
          signatureHelp: { signatureInformation: { documentationFormat: md, parameterInformation: { labelOffsetSupport: true } } },
          definition: { linkSupport: true },
          publishDiagnostics: { tagSupport: { valueSet: [1, 2] } },
        },
      },
    }
  }

  /** Uscita inattesa: il core ha già riavviato il processo con lo stesso id (se aveva budget): si rifà l'handshake. */
  private onExit(id: number) {
    const s = this.byId.get(id)
    if (!s?.client) return
    s.client.reset('language server riavviato')
    for (const d of this.docs.values()) {
      if (d.lang !== s.lang) continue
      clearTimeout(d.timer)
      d.timer = undefined
      this.deps.monaco.editor.setModelMarkers(d.model, OWNER, [])
    }
    void this.handshake(s)
  }

  async dispose(): Promise<void> {
    await this.listening
    this.offs.forEach((f) => f())
    this.disposables.forEach((d) => d.dispose())
    for (const d of this.docs.values()) {
      clearTimeout(d.timer)
      d.subs.forEach((x) => x.dispose())
    }
    this.docs.clear()
    for (const s of this.sessions.values()) {
      s.gen++
      s.client?.reset('chiuso')
      if (s.id) await this.deps.api.stop(s.id).catch(() => {})
    }
    this.sessions.clear()
    this.byId.clear()
  }

  // ---------- diagnostica ----------

  private diagnostics(p: any) {
    const path = typeof p?.uri === 'string' ? fileUriToPath(p.uri) : null
    const doc = path ? this.docs.get(path) : undefined
    if (!doc || !Array.isArray(p.diagnostics)) return
    const markers: M.editor.IMarkerData[] = p.diagnostics.map((d: any) => {
      const code = d.code && typeof d.code === 'object' ? d.code.value : d.code
      return {
        ...monRange(d.range),
        severity: SEVERITY[d.severity ?? 1] ?? 8,
        message: String(d.message ?? ''),
        source: typeof d.source === 'string' ? d.source : undefined,
        code: code === undefined || code === null ? undefined : String(code),
        tags: Array.isArray(d.tags) ? d.tags : undefined,
      }
    })
    this.deps.monaco.editor.setModelMarkers(doc.model, OWNER, markers)
  }

  // ---------- richieste dei provider ----------

  private inProject(path: string): boolean {
    if (!path.startsWith('/') || path.split('/').some((x) => x === '..')) return false
    return this.deps.folders().some((f) => {
      const base = f.replace(/\/+$/, '')
      return path === base || path.startsWith(base + '/')
    })
  }

  /** Richiesta su un documento aperto; `null` se il server non è pronto, annullata, scaduta o in errore. */
  private async call(model: M.editor.ITextModel, method: string, params: object, token?: M.CancellationToken, cap?: string): Promise<any> {
    const doc = this.docs.get(model.uri.path)
    const s = doc && this.sessions.get(doc.lang)
    if (!doc || !doc.opened || !s?.client || s.state !== 'pronto' || (cap && !s.caps?.[cap])) return null
    if (token?.isCancellationRequested) return null
    this.flush(doc)
    const ctl = new AbortController()
    const sub = token?.onCancellationRequested(() => ctl.abort())
    try {
      return await s.client.request(method, { textDocument: { uri: doc.uri }, ...params }, 5000, ctl.signal)
    } catch {
      return null
    } finally {
      sub?.dispose()
    }
  }

  private md(source: string): M.IMarkdownString {
    // HTML già sanificato da `renderMarkdown`; `isTrusted` resta spento (niente comandi nei link)
    return { value: this.deps.render(source), supportHtml: true }
  }

  private docText(d: any): string | M.IMarkdownString | undefined {
    if (!d) return undefined
    if (typeof d === 'string') return d
    if (d.kind === 'markdown') return this.md(String(d.value ?? ''))
    return typeof d.value === 'string' ? d.value : undefined
  }

  private suggestion(it: any, model: M.editor.ITextModel, pos: M.IPosition): M.languages.CompletionItem {
    const te = it.textEdit
    let range: M.languages.CompletionItem['range']
    if (te?.range) range = monRange(te.range)
    else if (te?.insert && te?.replace) range = { insert: monRange(te.insert), replace: monRange(te.replace) }
    else {
      const w = model.getWordUntilPosition(pos)
      range = { startLineNumber: pos.lineNumber, startColumn: w.startColumn, endLineNumber: pos.lineNumber, endColumn: w.endColumn }
    }
    const label = typeof it.label === 'string' ? it.label : String(it.label ?? '')
    const s: M.languages.CompletionItem = {
      label,
      kind: KIND[(it.kind ?? 1) - 1] ?? 18,
      insertText: te?.newText ?? it.insertText ?? label,
      insertTextRules: it.insertTextFormat === 2 ? 4 : undefined,
      range,
      detail: typeof it.detail === 'string' ? it.detail : undefined,
      documentation: this.docText(it.documentation),
      sortText: it.sortText,
      filterText: it.filterText,
      preselect: it.preselect,
      commitCharacters: it.commitCharacters,
      tags: it.deprecated || it.tags?.includes?.(1) ? [1] : undefined,
      additionalTextEdits: Array.isArray(it.additionalTextEdits) ? it.additionalTextEdits.map((e: any) => ({ range: monRange(e.range), text: e.newText })) : undefined,
    }
    originals.set(s, it)
    return s
  }

  private register() {
    const { languages, editor } = this.deps.monaco
    for (const lang of LSP_LANGS) {
      this.disposables.push(
        languages.registerCompletionItemProvider(lang, {
          triggerCharacters: ['.', ':', '/', '@', '<', '"', "'"],
          provideCompletionItems: async (model, position, context, token) => {
            const r = await this.call(
              model,
              'textDocument/completion',
              { position: lspPos(position), context: { triggerKind: context.triggerKind + 1, ...(context.triggerCharacter ? { triggerCharacter: context.triggerCharacter } : {}) } },
              token,
              'completionProvider',
            )
            const items: any[] = Array.isArray(r) ? r : (r?.items ?? [])
            return { suggestions: items.map((it) => this.suggestion(it, model, position)), incomplete: !!r?.isIncomplete }
          },
          resolveCompletionItem: async (item, token) => {
            const orig = originals.get(item)
            const s = this.sessions.get(lang)
            if (!orig || !s?.client || s.state !== 'pronto' || !s.caps?.completionProvider?.resolveProvider) return item
            try {
              const ctl = new AbortController()
              const sub = token.onCancellationRequested(() => ctl.abort())
              const full: any = await s.client.request('completionItem/resolve', orig, 5000, ctl.signal).finally(() => sub.dispose())
              if (!full) return item
              if (typeof full.detail === 'string') item.detail = full.detail
              const d = this.docText(full.documentation)
              if (d !== undefined) item.documentation = d
              if (Array.isArray(full.additionalTextEdits)) item.additionalTextEdits = full.additionalTextEdits.map((e: any) => ({ range: monRange(e.range), text: e.newText }))
            } catch {
              /* resta l'elemento non risolto */
            }
            return item
          },
        }),
        languages.registerHoverProvider(lang, {
          provideHover: async (model, position, token) => {
            const r = await this.call(model, 'textDocument/hover', { position: lspPos(position) }, token, 'hoverProvider')
            if (!r?.contents) return null
            const parts = (Array.isArray(r.contents) ? r.contents : [r.contents]).map(markdownSource).filter((x: string) => x.trim() !== '')
            if (parts.length === 0) return null
            return { contents: parts.map((p: string) => this.md(p)), range: r.range ? monRange(r.range) : undefined }
          },
        }),
        languages.registerSignatureHelpProvider(lang, {
          signatureHelpTriggerCharacters: ['(', ','],
          signatureHelpRetriggerCharacters: [')'],
          provideSignatureHelp: async (model, position, token) => {
            const r = await this.call(model, 'textDocument/signatureHelp', { position: lspPos(position) }, token, 'signatureHelpProvider')
            if (!r || !Array.isArray(r.signatures) || r.signatures.length === 0) return null
            const value: M.languages.SignatureHelp = {
              activeSignature: r.activeSignature ?? 0,
              activeParameter: r.activeParameter ?? 0,
              signatures: r.signatures.map((sg: any) => ({
                label: String(sg.label ?? ''),
                documentation: this.docText(sg.documentation),
                activeParameter: sg.activeParameter ?? undefined,
                parameters: (sg.parameters ?? []).map((p: any) => ({ label: p.label, documentation: this.docText(p.documentation) })),
              })),
            }
            return { value, dispose() {} }
          },
        }),
        languages.registerDefinitionProvider(lang, {
          provideDefinition: async (model, position, token) => {
            const r = await this.call(model, 'textDocument/definition', { position: lspPos(position) }, token, 'definitionProvider')
            const list: any[] = Array.isArray(r) ? r : r ? [r] : []
            const out: M.languages.Location[] = []
            for (const l of list) {
              const path = fileUriToPath(String(l.targetUri ?? l.uri ?? ''))
              const range = l.targetSelectionRange ?? l.targetRange ?? l.range
              // fuori dal progetto non si apre nulla (il core rifiuterebbe comunque la lettura)
              if (path && range && this.inProject(path)) out.push({ uri: this.deps.monaco.Uri.file(path), range: monRange(range) })
            }
            return out.length > 0 ? out : null
          },
        }),
      )
    }
    // Monaco apre da solo solo i modelli che esistono: i file del progetto si aprono nell'editor dell'app
    this.disposables.push(
      editor.registerEditorOpener({
        openCodeEditor: (_source, resource, sel) => {
          if (resource.scheme !== 'file') return false
          if (this.inProject(resource.path)) {
            const s = sel as any
            this.deps.openFile(resource.path, s?.startLineNumber ?? s?.lineNumber ?? 1, s?.startColumn ?? s?.column ?? 1)
          }
          return true
        },
      }),
    )
  }
}

/** Elemento LSP originale di ogni suggerimento, per `completionItem/resolve`. */
const originals = new WeakMap<object, any>()

export const createBridge = (deps: BridgeDeps) => new LspBridge(deps)
export { LANG_LABEL }
