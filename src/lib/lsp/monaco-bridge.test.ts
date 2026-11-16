// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { createBridge, type LspApi } from './monaco-bridge'
import { renderMarkdown } from '../safe-markdown'

type Msg = { jsonrpc: '2.0'; id?: number; method?: string; params?: any; result?: any }

/** Server finto: risponde alle richieste con `replies[method]` e registra tutto ciò che il client invia. */
function setup(opts: { startError?: string; replies?: Record<string, (p: any) => unknown> } = {}) {
  const sent: Msg[] = []
  const messageCbs: ((id: number, m: string) => void)[] = []
  const exitCbs: ((id: number, code: number) => void)[] = []
  const state = { starts: 0, failSend: null as string | null }
  const replies: Record<string, (p: any) => unknown> = {
    initialize: () => ({ capabilities: { completionProvider: { resolveProvider: true }, hoverProvider: true, signatureHelpProvider: {}, definitionProvider: true } }),
    ...opts.replies,
  }
  const reply = (id: number, result: unknown) => queueMicrotask(() => messageCbs.forEach((cb) => cb(7, JSON.stringify({ jsonrpc: '2.0', id, result }))))
  const api: LspApi = {
    start: vi.fn(async () => {
      state.starts++
      if (opts.startError) throw opts.startError
      return { id: 7, rootUri: 'file:///Users/dev/proj' }
    }),
    send: vi.fn(async (_id: number, message: string) => {
      if (state.failSend) throw state.failSend
      const m = JSON.parse(message) as Msg
      sent.push(m)
      if (m.id !== undefined && m.method && replies[m.method]) reply(m.id, replies[m.method](m.params))
    }),
    stop: vi.fn(async () => {}),
    onMessage: async (cb) => {
      messageCbs.push(cb)
      return () => {}
    },
    onExit: async (cb) => {
      exitCbs.push(cb)
      return () => {}
    },
  }
  const push = (method: string, params: unknown) => messageCbs.forEach((cb) => cb(7, JSON.stringify({ jsonrpc: '2.0', method, params })))
  const exit = (code = 1) => exitCbs.forEach((cb) => cb(7, code))
  return { api, sent, push, exit, state, methods: () => sent.map((m) => m.method) }
}

function fakeMonaco() {
  const providers: Record<string, Record<string, any>> = {}
  const markers: { model: unknown; owner: string; list: any[] }[] = []
  const reg = (kind: string) => (lang: string, p: unknown) => {
    ;(providers[kind] ??= {})[lang] = p
    return { dispose() {} }
  }
  let opener: any
  const monaco = {
    Uri: { file: (path: string) => ({ scheme: 'file', path, toString: () => `file://${path}` }) },
    languages: {
      registerCompletionItemProvider: reg('completion'),
      registerHoverProvider: reg('hover'),
      registerSignatureHelpProvider: reg('signature'),
      registerDefinitionProvider: reg('definition'),
    },
    editor: {
      setModelMarkers: (model: unknown, owner: string, list: any[]) => markers.push({ model, owner, list }),
      registerEditorOpener: (o: unknown) => {
        opener = o
        return { dispose() {} }
      },
    },
  }
  return { monaco: monaco as never, providers, markers, opener: () => opener }
}

function fakeModel(path: string, text: string) {
  let value = text
  const changeSubs: (() => void)[] = []
  const disposeSubs: (() => void)[] = []
  const uri = { scheme: 'file', path, toString: () => `file://${path}` }
  return {
    uri,
    getValue: () => value,
    edit(t: string) {
      value = t
      changeSubs.forEach((f) => f())
    },
    onDidChangeContent: (f: () => void) => {
      changeSubs.push(f)
      return { dispose() { changeSubs.splice(changeSubs.indexOf(f), 1) } }
    },
    onWillDispose: (f: () => void) => {
      disposeSubs.push(f)
      return { dispose() {} }
    },
    dispose: () => disposeSubs.forEach((f) => f()),
    getWordUntilPosition: (p: { column: number }) => ({ startColumn: p.column - 2, endColumn: p.column, word: 'pr' }),
  }
}

const FOLDER = '/Users/dev/proj'
const tick = () => new Promise((r) => setTimeout(r, 0))
const token = () => ({ isCancellationRequested: false, onCancellationRequested: () => ({ dispose() {} }) })

function make(opts: Parameters<typeof setup>[0] = {}) {
  const s = setup(opts)
  const m = fakeMonaco()
  const opened: [string, number, number][] = []
  const notices: [string, string][] = []
  const status: [string, string, string | undefined][] = []
  const bridge = createBridge({
    monaco: m.monaco,
    api: s.api,
    folders: () => [FOLDER],
    openFile: (p, l, c) => opened.push([p, l, c]),
    render: renderMarkdown,
    onStatus: (lang, st, detail) => status.push([lang, st, detail]),
    notify: (lang, text) => notices.push([lang, text]),
  })
  return { ...s, ...m, bridge, opened, notices, status }
}

async function ready(t: ReturnType<typeof make>, path = `${FOLDER}/a.py`, text = 'print(1)\n') {
  const model = fakeModel(path, text)
  t.bridge.attach(model as never, path)
  await vi.waitFor(() => expect(t.methods()).toContain('textDocument/didOpen'))
  return model
}

beforeEach(() => vi.useRealTimers())
afterEach(() => vi.useRealTimers())

describe('lifecycle', () => {
  it('starts the server on the first file and sends initialize, initialized and didOpen', async () => {
    const t = make()
    await ready(t)
    expect(t.api.start).toHaveBeenCalledWith('python')
    expect(t.methods()).toEqual(['initialize', 'initialized', 'textDocument/didOpen'])
    const init = t.sent[0].params
    expect(init.rootUri).toBe('file:///Users/dev/proj')
    expect(init.workspaceFolders).toEqual([{ uri: 'file:///Users/dev/proj', name: 'proj' }])
    expect(init.capabilities.textDocument.synchronization).toBeTruthy()
    const open = t.sent[2].params.textDocument
    expect(open).toEqual({ uri: `file://${FOLDER}/a.py`, languageId: 'python', version: 1, text: 'print(1)\n' })
    expect(t.status.map((s) => s[1])).toEqual(['avvio', 'pronto'])
  })

  it('reuses one server for several files of the same language', async () => {
    const t = make()
    await ready(t)
    const b = fakeModel(`${FOLDER}/b.py`, 'x')
    t.bridge.attach(b as never, `${FOLDER}/b.py`)
    await vi.waitFor(() => expect(t.methods().filter((m) => m === 'textDocument/didOpen')).toHaveLength(2))
    expect(t.api.start).toHaveBeenCalledTimes(1)
    expect(t.methods().filter((m) => m === 'initialize')).toHaveLength(1)
  })

  it('ignores files that are not in a language with a server', async () => {
    const t = make()
    t.bridge.attach(fakeModel('/p/a.txt', '') as never, '/p/a.txt')
    await tick()
    expect(t.api.start).not.toHaveBeenCalled()
  })

  it('sends one didChange with the full text, 150 ms after the last edit', async () => {
    const t = make()
    const model = await ready(t)
    vi.useFakeTimers()
    model.edit('a')
    await vi.advanceTimersByTimeAsync(100)
    model.edit('ab')
    await vi.advanceTimersByTimeAsync(149)
    expect(t.methods()).not.toContain('textDocument/didChange')
    await vi.advanceTimersByTimeAsync(1)
    const changes = t.sent.filter((m) => m.method === 'textDocument/didChange')
    expect(changes).toHaveLength(1)
    expect(changes[0].params).toEqual({ textDocument: { uri: `file://${FOLDER}/a.py`, version: 2 }, contentChanges: [{ text: 'ab' }] })
  })

  it('sends didSave with the text and didClose when the model is disposed', async () => {
    const t = make()
    const model = await ready(t)
    t.bridge.didSave(model as never)
    model.dispose()
    await vi.waitFor(() => expect(t.methods()).toContain('textDocument/didClose'))
    const save = t.sent.find((m) => m.method === 'textDocument/didSave')!
    expect(save.params).toEqual({ textDocument: { uri: `file://${FOLDER}/a.py` }, text: 'print(1)\n' })
    expect(t.markers.at(-1)).toMatchObject({ owner: 'codeart-lsp', list: [] })
  })

  it('on server exit it redoes the handshake on the same id and reopens every document', async () => {
    const t = make()
    await ready(t)
    const b = fakeModel(`${FOLDER}/b.py`, 'bb')
    t.bridge.attach(b as never, `${FOLDER}/b.py`)
    await vi.waitFor(() => expect(t.methods().filter((m) => m === 'textDocument/didOpen')).toHaveLength(2))
    t.sent.length = 0
    t.exit()
    await vi.waitFor(() => expect(t.methods().filter((m) => m === 'textDocument/didOpen')).toHaveLength(2))
    expect(t.methods().slice(0, 2)).toEqual(['initialize', 'initialized'])
    expect(t.api.start).toHaveBeenCalledTimes(1)
    expect(t.api.send).toHaveBeenLastCalledWith(7, expect.any(String))
    expect(t.sent.filter((m) => m.method === 'textDocument/didOpen').map((m) => m.params.textDocument.text).sort()).toEqual(['bb', 'print(1)\n'])
    expect(t.status.at(-1)![1]).toBe('pronto')
  })

  it('rejects in-flight requests on exit, before redoing the handshake', async () => {
    const t = make({ replies: { 'textDocument/completion': () => { throw new Error('x') } } })
    const model = await ready(t)
    t.api.send = vi.fn(async (_i: number, m: string) => { t.sent.push(JSON.parse(m)) }) as never
    const p = t.providers.completion.python.provideCompletionItems(model, { lineNumber: 1, column: 1 }, { triggerKind: 0 }, token())
    await vi.waitFor(() => expect(t.methods()).toContain('textDocument/completion'))
    t.exit()
    expect(await p).toMatchObject({ suggestions: [] })
    expect(t.methods()).not.toContain('shutdown')
    await vi.waitFor(() => expect(t.methods().filter((m) => m === 'initialize')).toHaveLength(2))
  })

  it('shows "fermo" when the server stays stopped after the exit', async () => {
    const t = make()
    await ready(t)
    t.state.failSend = 'Language server fermo dopo troppi riavvii: riapri il progetto per riprovare'
    t.exit()
    await vi.waitFor(() => expect(t.status.at(-1)![1]).toBe('fermo'))
    expect(t.status.at(-1)![2]).toContain('troppi riavvii')
  })

  it('a missing server gives one informational line per session and status "fermo"', async () => {
    const t = make({ startError: 'language server per python non trovato: installa con `pip install python-lsp-server`' })
    t.bridge.attach(fakeModel(`${FOLDER}/a.py`, '') as never, `${FOLDER}/a.py`)
    t.bridge.attach(fakeModel(`${FOLDER}/b.py`, '') as never, `${FOLDER}/b.py`)
    await vi.waitFor(() => expect(t.status.at(-1)![1]).toBe('fermo'))
    await tick()
    expect(t.notices).toHaveLength(1)
    expect(t.notices[0][1]).toContain('pip install python-lsp-server')
    expect(t.methods()).toEqual([])
  })
})

describe('diagnostics', () => {
  it('turns publishDiagnostics into Monaco markers with severity and 1-based ranges', async () => {
    const t = make()
    const model = await ready(t)
    t.push('textDocument/publishDiagnostics', {
      uri: `file://${FOLDER}/a.py`,
      diagnostics: [
        { range: { start: { line: 0, character: 2 }, end: { line: 0, character: 5 } }, severity: 1, message: 'boom', source: 'pyright', code: 'E1' },
        { range: { start: { line: 2, character: 0 }, end: { line: 2, character: 1 } }, severity: 2, message: 'hmm' },
        { range: { start: { line: 3, character: 0 }, end: { line: 3, character: 1 } }, severity: 3, message: 'fyi' },
        { range: { start: { line: 4, character: 0 }, end: { line: 4, character: 1 } }, severity: 4, message: 'hint', tags: [1] },
      ],
    })
    const last = t.markers.at(-1)!
    expect(last.model).toBe(model)
    expect(last.owner).toBe('codeart-lsp')
    expect(last.list.map((x) => x.severity)).toEqual([8, 4, 2, 1])
    expect(last.list[0]).toMatchObject({ startLineNumber: 1, startColumn: 3, endLineNumber: 1, endColumn: 6, message: 'boom', source: 'pyright', code: 'E1' })
    expect(last.list[3].tags).toEqual([1])
  })

  it('ignores diagnostics for documents that are not open', async () => {
    const t = make()
    await ready(t)
    const n = t.markers.length
    t.push('textDocument/publishDiagnostics', { uri: `file://${FOLDER}/zzz.py`, diagnostics: [] })
    expect(t.markers.length).toBe(n)
  })
})

describe('completion', () => {
  it('converts items to suggestions with range, kind and snippets, and resolves on demand', async () => {
    const t = make({
      replies: {
        'textDocument/completion': () => ({
          isIncomplete: false,
          items: [
            { label: 'print', kind: 3, detail: 'fn', insertText: 'print($0)', insertTextFormat: 2, data: { k: 1 }, sortText: 'a' },
            { label: 'printf', kind: 2, textEdit: { range: { start: { line: 0, character: 0 }, end: { line: 0, character: 2 } }, newText: 'printf' } },
          ],
        }),
        'completionItem/resolve': (item: any) => ({ ...item, detail: 'resolved', documentation: { kind: 'markdown', value: '**doc**' }, additionalTextEdits: [{ range: { start: { line: 0, character: 0 }, end: { line: 0, character: 0 } }, newText: 'import x\n' }] }),
      },
    })
    const model = await ready(t)
    const p = t.providers.completion.python
    const res = await p.provideCompletionItems(model, { lineNumber: 1, column: 3 }, { triggerKind: 0 }, token())
    expect(res.suggestions).toHaveLength(2)
    const [a, b] = res.suggestions
    expect(a).toMatchObject({ label: 'print', kind: 1, insertText: 'print($0)', insertTextRules: 4, detail: 'fn', sortText: 'a' })
    expect(a.range).toEqual({ startLineNumber: 1, startColumn: 1, endLineNumber: 1, endColumn: 3 })
    expect(b).toMatchObject({ label: 'printf', kind: 0, insertText: 'printf', range: { startLineNumber: 1, startColumn: 1, endLineNumber: 1, endColumn: 3 } })
    const done = await p.resolveCompletionItem(a, token())
    expect(done.detail).toBe('resolved')
    expect(done.documentation.value).toContain('doc')
    expect(done.additionalTextEdits).toEqual([{ range: { startLineNumber: 1, startColumn: 1, endLineNumber: 1, endColumn: 1 }, text: 'import x\n' }])
    const req = t.sent.find((m) => m.method === 'textDocument/completion')!
    expect(req.params).toMatchObject({ textDocument: { uri: `file://${FOLDER}/a.py` }, position: { line: 0, character: 2 }, context: { triggerKind: 1 } })
  })

  it('sends pending edits before asking, so the server sees the current text', async () => {
    const t = make({ replies: { 'textDocument/completion': () => [] } })
    const model = await ready(t)
    model.edit('print(2)')
    await t.providers.completion.python.provideCompletionItems(model, { lineNumber: 1, column: 1 }, { triggerKind: 0 }, token())
    const ms = t.methods()
    expect(ms.indexOf('textDocument/didChange')).toBeGreaterThan(-1)
    expect(ms.indexOf('textDocument/didChange')).toBeLessThan(ms.indexOf('textDocument/completion'))
  })

  it('cancels the request with $/cancelRequest when Monaco cancels', async () => {
    const t = make() // nessuna risposta configurata per completion: la richiesta resta in sospeso
    const model = await ready(t)
    let cancel = () => {}
    const tok = { isCancellationRequested: false, onCancellationRequested: (f: () => void) => { cancel = f; return { dispose() {} } } }
    const p = t.providers.completion.python.provideCompletionItems(model, { lineNumber: 1, column: 1 }, { triggerKind: 0 }, tok)
    await vi.waitFor(() => expect(t.methods()).toContain('textDocument/completion'))
    const id = t.sent.find((m) => m.method === 'textDocument/completion')!.id
    cancel()
    expect(await p).toMatchObject({ suggestions: [] })
    expect(t.sent.at(-1)).toMatchObject({ method: '$/cancelRequest', params: { id } })
  })
})

describe('hover and signature help', () => {
  it('renders hover markdown through the sanitiser: no raw HTML survives', async () => {
    const t = make({
      replies: {
        'textDocument/hover': () => ({
          contents: { kind: 'markdown', value: '**bold** <script>alert(1)</script><img src=x onerror=alert(1)> [x](javascript:alert(1))' },
          range: { start: { line: 0, character: 0 }, end: { line: 0, character: 5 } },
        }),
      },
    })
    const model = await ready(t)
    const h = await t.providers.hover.python.provideHover(model, { lineNumber: 1, column: 2 }, token())
    expect(h.range).toEqual({ startLineNumber: 1, startColumn: 1, endLineNumber: 1, endColumn: 6 })
    const html = h.contents[0].value as string
    expect(html).toContain('<strong>bold</strong>')
    expect(html).not.toMatch(/<script|onerror|javascript:|<img/i)
    expect(h.contents[0].supportHtml).toBe(true)
    expect(h.contents[0].isTrusted).toBeFalsy()
  })

  it('accepts legacy MarkedString forms and returns null for empty hovers', async () => {
    let r: unknown = { contents: [{ language: 'python', value: 'def f()' }, 'testo'] }
    const t = make({ replies: { 'textDocument/hover': () => r } })
    const model = await ready(t)
    const h = await t.providers.hover.python.provideHover(model, { lineNumber: 1, column: 1 }, token())
    expect(h.contents.map((c: any) => c.value).join('')).toContain('def f()')
    r = null
    expect(await t.providers.hover.python.provideHover(model, { lineNumber: 1, column: 1 }, token())).toBeNull()
  })

  it('converts signature help', async () => {
    const t = make({
      replies: {
        'textDocument/signatureHelp': () => ({ signatures: [{ label: 'f(a, b)', documentation: 'doc', parameters: [{ label: 'a' }, { label: [3, 4], documentation: { kind: 'markdown', value: 'second' } }] }], activeSignature: 0, activeParameter: 1 }),
      },
    })
    const model = await ready(t)
    const r = await t.providers.signature.python.provideSignatureHelp(model, { lineNumber: 1, column: 3 }, token(), { triggerKind: 1, isRetrigger: false })
    expect(r.value.activeParameter).toBe(1)
    expect(r.value.signatures[0].label).toBe('f(a, b)')
    expect(r.value.signatures[0].parameters.map((p: any) => p.label)).toEqual(['a', [3, 4]])
    expect(typeof r.dispose).toBe('function')
  })
})

describe('definition', () => {
  const loc = (path: string) => ({ uri: `file://${path}`, range: { start: { line: 4, character: 1 }, end: { line: 4, character: 3 } } })

  it('returns in-project targets and drops those outside the project', async () => {
    const t = make({ replies: { 'textDocument/definition': () => [loc(`${FOLDER}/lib/x.py`), loc('/Users/dev/other/y.py'), loc(`${FOLDER}/../escape.py`), loc('/usr/lib/python3/os.py')] } })
    const model = await ready(t)
    const r = await t.providers.definition.python.provideDefinition(model, { lineNumber: 1, column: 1 }, token())
    expect(r).toHaveLength(1)
    expect(r[0].uri.path).toBe(`${FOLDER}/lib/x.py`)
    expect(r[0].range).toEqual({ startLineNumber: 5, startColumn: 2, endLineNumber: 5, endColumn: 4 })
  })

  it('returns null when every target is outside the project', async () => {
    const t = make({ replies: { 'textDocument/definition': () => loc('/usr/lib/python3/os.py') } })
    const model = await ready(t)
    expect(await t.providers.definition.python.provideDefinition(model, { lineNumber: 1, column: 1 }, token())).toBeNull()
  })

  it('accepts LocationLink targets', async () => {
    const t = make({ replies: { 'textDocument/definition': () => [{ targetUri: `file://${FOLDER}/z.py`, targetRange: loc('').range, targetSelectionRange: { start: { line: 1, character: 0 }, end: { line: 1, character: 2 } } }] } })
    const model = await ready(t)
    const r = await t.providers.definition.python.provideDefinition(model, { lineNumber: 1, column: 1 }, token())
    expect(r[0].range.startLineNumber).toBe(2)
  })

  it('the Monaco opener opens in-project files in the editor and swallows the rest', async () => {
    const t = make()
    const open = t.opener()
    const res = (path: string, scheme = 'file') => ({ scheme, path })
    expect(open.openCodeEditor(null, res(`${FOLDER}/lib/x.py`), { startLineNumber: 5, startColumn: 2 })).toBe(true)
    expect(t.opened).toEqual([[`${FOLDER}/lib/x.py`, 5, 2]])
    expect(open.openCodeEditor(null, res('/etc/passwd'), { startLineNumber: 1, startColumn: 1 })).toBe(true)
    expect(open.openCodeEditor(null, res('/etc/passwd', 'http'), undefined)).toBe(false)
    expect(t.opened).toHaveLength(1)
  })
})
