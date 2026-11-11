// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const handlers: Record<string, (e: { payload: unknown }) => void> = {}
vi.mock('../events', () => ({
  listenProject: vi.fn(async (name: string, _id: string, cb: (e: { payload: unknown }) => void) => {
    handlers[name] = cb
    return () => {}
  }),
}))
let nextId = 1
const searchStart = vi.fn(async (_q: unknown) => nextId++)
const searchCancel = vi.fn(async (_id: number) => {})
const searchReplace = vi.fn(async (..._a: unknown[]) => ({ files: 1, replacements: 2, skipped: [], cancelled: false }))
vi.mock('../ipc', () => ({
  searchStart: (q: unknown) => searchStart(q),
  searchCancel: (id: number) => searchCancel(id),
  searchReplace: (...a: unknown[]) => searchReplace(...a),
}))
const tabs: { path: string; dirty: boolean; diff: null }[] = []
vi.mock('./editor.svelte', () => ({ editor: { get tabs() { return tabs } } }))

import { SearchStore, DEBOUNCE_MS } from './search.svelte'

const hit = (path: string, line = 1) => ({ path, line, col: 1, len: 1, preview: 'x', previewCol: 0, previewLen: 1 })

beforeEach(() => {
  vi.useFakeTimers()
  nextId = 1
  tabs.length = 0
  vi.clearAllMocks()
})
afterEach(() => vi.useRealTimers())

async function bound() {
  const s = new SearchStore()
  await s.bind('p1')
  return s
}

describe('search store', () => {
  it('debounces typing by 250 ms into a single search', async () => {
    const s = await bound()
    s.setText('a')
    s.setText('ab')
    await vi.advanceTimersByTimeAsync(DEBOUNCE_MS - 1)
    expect(searchStart).not.toHaveBeenCalled()
    await vi.advanceTimersByTimeAsync(1)
    expect(searchStart).toHaveBeenCalledTimes(1)
    expect(searchStart.mock.calls[0][0]).toMatchObject({ text: 'ab', regex: false, caseSensitive: false, wholeWord: false })
  })

  it('a new query cancels the old one and ignores its late events', async () => {
    const s = await bound()
    s.setText('a', true)
    await vi.advanceTimersByTimeAsync(0)
    s.setText('b', true)
    await vi.advanceTimersByTimeAsync(0)
    expect(searchCancel).toHaveBeenCalledWith(1)
    handlers['search:hits']({ payload: { id: 1, hits: [hit('/p/old')] } })
    handlers['search:hits']({ payload: { id: 2, hits: [hit('/p/new')] } })
    expect(s.groups.map((g) => g.path)).toEqual(['/p/new'])
  })

  it('keeps events that arrive before search_start resolves', async () => {
    const s = await bound()
    searchStart.mockImplementationOnce(async () => {
      handlers['search:hits']({ payload: { id: 7, hits: [hit('/p/a')] } })
      return 7
    })
    s.setText('a', true)
    await vi.advanceTimersByTimeAsync(0)
    expect(s.hitCount).toBe(1)
  })

  it('groups hits by file in arrival order with counts, then reports done and truncation', async () => {
    const s = await bound()
    s.setText('a', true)
    await vi.advanceTimersByTimeAsync(0)
    handlers['search:hits']({ payload: { id: 1, hits: [hit('/p/b', 1), hit('/p/a', 1), hit('/p/b', 2)] } })
    handlers['search:hits']({ payload: { id: 1, hits: [hit('/p/a', 5)] } })
    expect(s.groups.map((g) => [g.path, g.hits.length])).toEqual([['/p/b', 2], ['/p/a', 2]])
    expect(s.hitCount).toBe(4)
    expect(s.status).toBe('running')
    handlers['search:done']({ payload: { id: 1, summary: { files: 2, hits: 4, truncated: true, cancelled: false, error: null } } })
    expect(s.status).toBe('done')
    expect(s.truncated).toBe(true)
  })

  it('empty text clears results without searching; an invalid pattern shows the error', async () => {
    const s = await bound()
    s.setText('a', true)
    await vi.advanceTimersByTimeAsync(0)
    handlers['search:hits']({ payload: { id: 1, hits: [hit('/p/a')] } })
    s.setText('', true)
    await vi.advanceTimersByTimeAsync(0)
    expect(s.groups).toEqual([])
    expect(s.status).toBe('idle')
    expect(searchStart).toHaveBeenCalledTimes(1)
    s.setText('(', true)
    await vi.advanceTimersByTimeAsync(0)
    handlers['search:done']({ payload: { id: 2, summary: { files: 0, hits: 0, truncated: false, cancelled: false, error: 'Espressione non valida' } } })
    expect(s.status).toBe('error')
    expect(s.error).toBe('Espressione non valida')
  })

  it('splits include/exclude globs on commas', async () => {
    const s = await bound()
    s.include = '*.ts, src/**'
    s.exclude = ' '
    s.text = 'x'
    expect(s.query()).toMatchObject({ include: ['*.ts', 'src/**'], exclude: [] })
  })

  it('replaceAll skips files with unsaved edits, then searches again', async () => {
    const s = await bound()
    s.setText('a', true)
    await vi.advanceTimersByTimeAsync(0)
    handlers['search:hits']({ payload: { id: 1, hits: [hit('/p/a'), hit('/p/b')] } })
    tabs.push({ path: '/p/b', dirty: true, diff: null })
    s.replacement = 'z'
    await s.replaceAll()
    expect(searchReplace).toHaveBeenCalledWith(expect.objectContaining({ text: 'a' }), 'z', ['/p/a'])
    expect(s.outcome).toMatchObject({ replacements: 2, dirtySkipped: 1 })
    expect(searchStart).toHaveBeenCalledTimes(2)
  })
})

describe('search store robustness', () => {
  it('overlapping runs: the newer run keeps its events and finishes', async () => {
    const s = await bound()
    let release1!: (id: number) => void
    searchStart.mockImplementationOnce(() => new Promise<number>((r) => (release1 = r)))
    s.setText('a', true)
    s.setText('ab', true)
    await vi.advanceTimersByTimeAsync(0) // la seconda risponde con id 1 (default), la prima è ancora in attesa
    handlers['search:hits']({ payload: { id: 1, hits: [hit('/p/new')] } })
    release1(9) // la prima risponde dopo: non deve rubare lo stato
    await vi.advanceTimersByTimeAsync(0)
    handlers['search:done']({ payload: { id: 1, summary: { files: 1, hits: 1, truncated: false, cancelled: false, error: null } } })
    expect(s.status).toBe('done')
    expect(s.groups.map((g) => g.path)).toEqual(['/p/new'])
  })

  it('listeners survive hiding the view: bind is idempotent and the debounce still fires', async () => {
    const s = await bound()
    await s.bind('p1')
    s.setText('a')
    await vi.advanceTimersByTimeAsync(DEBOUNCE_MS)
    expect(searchStart).toHaveBeenCalledTimes(1)
  })

  it('auto-collapses groups past 1 000 hits and caps visible rows per file', async () => {
    const s = await bound()
    s.setText('a', true)
    await vi.advanceTimersByTimeAsync(0)
    handlers['search:hits']({ payload: { id: 1, hits: Array.from({ length: 150 }, (_, i) => hit('/p/a', i + 1)) } })
    const g = s.groups[0]
    expect(s.visible(g)).toMatchObject({ hidden: 50 })
    expect(s.visible(g).hits).toHaveLength(100)
    s.showMore('/p/a')
    expect(s.visible(s.groups[0]).hidden).toBe(0)
    expect(s.collapsed['/p/a']).toBeUndefined()
    handlers['search:hits']({ payload: { id: 1, hits: Array.from({ length: 900 }, (_, i) => hit('/p/b', i + 1)) } })
    handlers['search:hits']({ payload: { id: 1, hits: [hit('/p/c')] } })
    expect(s.collapsed).toMatchObject({ '/p/a': true, '/p/b': true, '/p/c': true })
  })
})

describe('empty text', () => {
  it('does not leave the store waiting for a superseded search_start', async () => {
    const s = await bound()
    let release!: (id: number) => void
    searchStart.mockImplementationOnce(() => new Promise<number>((r) => (release = r)))
    s.setText('a', true)
    s.setText('', true)
    await vi.advanceTimersByTimeAsync(0)
    release(5)
    await vi.advanceTimersByTimeAsync(0)
    expect(searchCancel).toHaveBeenCalledWith(5)
    s.setText('b', true)
    await vi.advanceTimersByTimeAsync(0)
    handlers['search:hits']({ payload: { id: 1, hits: [hit('/p/x')] } })
    expect(s.groups).toHaveLength(1)
  })
})
