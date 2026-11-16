import { describe, expect, it, vi } from 'vitest'
import { LspClient, LspError } from './client'

function setup() {
  const sent: any[] = []
  const client = new LspClient(async (m) => {
    sent.push(JSON.parse(m))
  })
  return { client, sent }
}

describe('LspClient', () => {
  it('correlates responses with requests by id', async () => {
    const { client, sent } = setup()
    const a = client.request('a', { x: 1 })
    const b = client.request('b', {})
    expect(sent.map((m) => [m.jsonrpc, m.method, m.id])).toEqual([['2.0', 'a', 1], ['2.0', 'b', 2]])
    client.handle(JSON.stringify({ jsonrpc: '2.0', id: 2, result: 'second' }))
    client.handle(JSON.stringify({ jsonrpc: '2.0', id: 1, result: 'first' }))
    expect(await a).toBe('first')
    expect(await b).toBe('second')
  })

  it('rejects with the server error', async () => {
    const { client } = setup()
    const p = client.request('a', {})
    client.handle(JSON.stringify({ jsonrpc: '2.0', id: 1, error: { code: -32601, message: 'no' } }))
    await expect(p).rejects.toMatchObject({ name: 'LspError', code: -32601, message: 'no' })
    await expect(p).rejects.toBeInstanceOf(LspError)
  })

  it('times out and tells the server to cancel', async () => {
    vi.useFakeTimers()
    const { client, sent } = setup()
    const p = client.request('slow', {}, 5000)
    const assertion = expect(p).rejects.toMatchObject({ name: 'LspTimeout' })
    await vi.advanceTimersByTimeAsync(5000)
    await assertion
    expect(sent.at(-1)).toMatchObject({ method: '$/cancelRequest', params: { id: 1 } })
    // una risposta tardiva non fa danni
    client.handle(JSON.stringify({ jsonrpc: '2.0', id: 1, result: 1 }))
    vi.useRealTimers()
  })

  it('uses 5 seconds as the default timeout', async () => {
    vi.useFakeTimers()
    const { client } = setup()
    const p = client.request('slow', {})
    const assertion = expect(p).rejects.toMatchObject({ name: 'LspTimeout' })
    await vi.advanceTimersByTimeAsync(4999)
    await vi.advanceTimersByTimeAsync(1)
    await assertion
    vi.useRealTimers()
  })

  it('cancels with $/cancelRequest when the signal aborts', async () => {
    const { client, sent } = setup()
    const ctl = new AbortController()
    const p = client.request('c', {}, 5000, ctl.signal)
    ctl.abort()
    await expect(p).rejects.toMatchObject({ name: 'LspCancelled' })
    expect(sent.at(-1)).toMatchObject({ method: '$/cancelRequest', params: { id: 1 } })
  })

  it('rejects at once when the signal is already aborted, without sending', async () => {
    const { client, sent } = setup()
    const ctl = new AbortController()
    ctl.abort()
    await expect(client.request('c', {}, 5000, ctl.signal)).rejects.toMatchObject({ name: 'LspCancelled' })
    expect(sent).toEqual([])
  })

  it('rejects when the transport fails to send', async () => {
    const client = new LspClient(async () => {
      throw 'Language server fermo'
    })
    await expect(client.request('a', {})).rejects.toThrow('Language server fermo')
  })

  it('dispatches notifications and lets handlers unsubscribe', () => {
    const { client } = setup()
    const cb = vi.fn()
    const off = client.onNotification('textDocument/publishDiagnostics', cb)
    client.handle(JSON.stringify({ jsonrpc: '2.0', method: 'textDocument/publishDiagnostics', params: { uri: 'u' } }))
    expect(cb).toHaveBeenCalledWith({ uri: 'u' })
    off()
    client.handle(JSON.stringify({ jsonrpc: '2.0', method: 'textDocument/publishDiagnostics', params: {} }))
    expect(cb).toHaveBeenCalledTimes(1)
  })

  it('sends notifications without an id', () => {
    const { client, sent } = setup()
    client.notify('initialized', {})
    expect(sent).toEqual([{ jsonrpc: '2.0', method: 'initialized', params: {} }])
  })

  it('answers server requests: known ones with null, unknown with MethodNotFound', () => {
    const { client, sent } = setup()
    client.handle(JSON.stringify({ jsonrpc: '2.0', id: 'a', method: 'workspace/configuration', params: { items: [{}, {}] } }))
    client.handle(JSON.stringify({ jsonrpc: '2.0', id: 7, method: 'client/registerCapability', params: {} }))
    client.handle(JSON.stringify({ jsonrpc: '2.0', id: 8, method: 'wat/ever', params: {} }))
    expect(sent[0]).toEqual({ jsonrpc: '2.0', id: 'a', result: [null, null] })
    expect(sent[1]).toEqual({ jsonrpc: '2.0', id: 7, result: null })
    expect(sent[2]).toMatchObject({ jsonrpc: '2.0', id: 8, error: { code: -32601 } })
  })

  it('ignores malformed messages', () => {
    const { client } = setup()
    expect(() => client.handle('not json')).not.toThrow()
    expect(() => client.handle('42')).not.toThrow()
  })

  it('reset rejects every pending request', async () => {
    const { client } = setup()
    const p = client.request('a', {})
    client.reset('server riavviato')
    await expect(p).rejects.toThrow('server riavviato')
  })
})
