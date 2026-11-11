import { afterEach, describe, expect, it, vi } from 'vitest'

vi.mock('./ipc', () => ({ fsReadBytes: vi.fn(async () => new Uint8Array([1, 2, 3, 4])) }))
import { formatSize, loadBlobUrl } from './blob'

afterEach(() => vi.restoreAllMocks())

describe('loadBlobUrl', () => {
  it('builds a blob url with the MIME type from the fixed table', async () => {
    let seen: Blob | undefined
    URL.createObjectURL = vi.fn((b: Blob | MediaSource) => ((seen = b as Blob), 'blob:x/1'))
    const r = await loadBlobUrl('/p/a.PNG', 'a.PNG')
    expect(r).toEqual({ url: 'blob:x/1', size: 4 })
    expect(seen?.type).toBe('image/png')
    await loadBlobUrl('/p/a.unknown', 'a.unknown')
    expect(seen?.type).toBe('application/octet-stream')
  })
})

describe('formatSize', () => {
  it('formats bytes, KB and MB', () => {
    expect(formatSize(12)).toBe('12 B')
    expect(formatSize(2048)).toBe('2.0 KB')
    expect(formatSize(5 * 1024 * 1024)).toBe('5.0 MB')
  })
})
