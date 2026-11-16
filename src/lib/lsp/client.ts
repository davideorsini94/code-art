/** Client JSON-RPC 2.0 per un language server: correlazione risposte, timeout, annullamento, notifiche.
 *  Il trasporto (`send`) è un canale di messaggi JSON già serializzati: il core fa la cornice `Content-Length`. */
export type Send = (message: string) => Promise<void> | void

export class LspError extends Error {
  readonly code: number
  readonly data: unknown
  constructor(code: number, message: string, data?: unknown) {
    super(message)
    this.name = 'LspError'
    this.code = code
    this.data = data
  }
}

export class LspTimeout extends Error {
  constructor(method: string, ms: number) {
    super(`${method}: nessuna risposta in ${ms} ms`)
    this.name = 'LspTimeout'
  }
}

export class LspCancelled extends Error {
  constructor(method: string) {
    super(`${method}: annullata`)
    this.name = 'LspCancelled'
  }
}

type Pending = { method: string; resolve: (v: unknown) => void; reject: (e: unknown) => void; cleanup: () => void }

/** Richieste del server che il client accetta senza fare nulla (la risposta è `null`). */
const IGNORED_REQUESTS = new Set([
  'client/registerCapability',
  'client/unregisterCapability',
  'window/workDoneProgress/create',
  'window/showMessageRequest',
  'workspace/semanticTokens/refresh',
  'workspace/inlayHint/refresh',
  'workspace/diagnostic/refresh',
])

const errText = (e: unknown) => (e instanceof Error ? e.message : String(e))

export class LspClient {
  private nextId = 1
  private pending = new Map<number, Pending>()
  private handlers = new Map<string, Set<(params: any) => void>>()

  constructor(private readonly send: Send) {}

  request(method: string, params: unknown, timeoutMs = 5000, signal?: AbortSignal): Promise<unknown> {
    if (signal?.aborted) return Promise.reject(new LspCancelled(method))
    const id = this.nextId++
    return new Promise((resolve, reject) => {
      let timer: ReturnType<typeof setTimeout> | undefined
      const cleanup = () => {
        clearTimeout(timer)
        signal?.removeEventListener('abort', onAbort)
        this.pending.delete(id)
      }
      const cancel = (e: Error) => {
        if (!this.pending.has(id)) return
        cleanup()
        this.notify('$/cancelRequest', { id })
        reject(e)
      }
      const onAbort = () => cancel(new LspCancelled(method))
      this.pending.set(id, { method, resolve, reject, cleanup })
      timer = setTimeout(() => cancel(new LspTimeout(method, timeoutMs)), timeoutMs)
      signal?.addEventListener('abort', onAbort, { once: true })
      this.post({ jsonrpc: '2.0', id, method, params }).catch((e) => {
        if (!this.pending.has(id)) return
        cleanup()
        reject(new Error(errText(e)))
      })
    })
  }

  notify(method: string, params: unknown): void {
    void this.post({ jsonrpc: '2.0', method, params }).catch(() => {})
  }

  /** Iscrive `cb` alle notifiche `method` del server; restituisce la funzione che la toglie. */
  onNotification(method: string, cb: (params: any) => void): () => void {
    let set = this.handlers.get(method)
    if (!set) this.handlers.set(method, (set = new Set()))
    set.add(cb)
    return () => set.delete(cb)
  }

  /** Un messaggio arrivato dal server (stringa JSON). Quelli non validi si ignorano. */
  handle(raw: string): void {
    let m: any
    try {
      m = JSON.parse(raw)
    } catch {
      return
    }
    if (!m || typeof m !== 'object') return
    if (typeof m.method === 'string') {
      if (m.id === undefined || m.id === null) this.handlers.get(m.method)?.forEach((cb) => cb(m.params))
      else this.answer(m.id, m.method, m.params)
      return
    }
    const p = typeof m.id === 'number' ? this.pending.get(m.id) : undefined
    if (!p) return
    p.cleanup()
    if (m.error) p.reject(new LspError(Number(m.error.code ?? 0), String(m.error.message ?? ''), m.error.data))
    else p.resolve(m.result ?? null)
  }

  /** Il server è ripartito (o il canale è chiuso): le richieste in volo non avranno più risposta. */
  reset(reason: string): void {
    for (const p of [...this.pending.values()]) {
      p.cleanup()
      p.reject(new Error(reason))
    }
  }

  private answer(id: number | string, method: string, params: any) {
    if (method === 'workspace/configuration') {
      const n = Array.isArray(params?.items) ? params.items.length : 0
      void this.post({ jsonrpc: '2.0', id, result: Array.from({ length: n }, () => null) }).catch(() => {})
    } else if (IGNORED_REQUESTS.has(method)) {
      void this.post({ jsonrpc: '2.0', id, result: null }).catch(() => {})
    } else {
      void this.post({ jsonrpc: '2.0', id, error: { code: -32601, message: `metodo non supportato: ${method}` } }).catch(() => {})
    }
  }

  private async post(msg: object): Promise<void> {
    await this.send(JSON.stringify(msg))
  }
}
