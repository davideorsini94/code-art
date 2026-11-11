import { describe, expect, it, vi } from 'vitest'
import { ImageLoader, type Limits } from './image-budget'
import { previewPolicy } from './md-policy'

const L: Limits = { concurrency: 2, maxImages: 3, perImage: 100, total: 150 }
const deferred = () => {
  let res!: (v: { url: string; size: number }) => void
  const p = new Promise<{ url: string; size: number }>((r) => (res = r))
  return { p, res }
}

describe('ImageLoader', () => {
  it('runs at most `concurrency` loads at once', async () => {
    const ds = [deferred(), deferred(), deferred()]
    let i = 0
    let active = 0
    let peak = 0
    const load = vi.fn(async () => {
      active++
      peak = Math.max(peak, active)
      const r = await ds[i++].p
      active--
      return r
    })
    const l = new ImageLoader(load, () => {}, { ...L, maxImages: 10 })
    const ps = ['a', 'b', 'c'].map((k) => l.request(k))
    await Promise.resolve()
    expect(load).toHaveBeenCalledTimes(2)
    ds[0].res({ url: 'blob:1', size: 1 })
    await Promise.resolve()
    await Promise.resolve()
    await Promise.resolve()
    expect(load).toHaveBeenCalledTimes(3)
    ds[1].res({ url: 'blob:2', size: 1 })
    ds[2].res({ url: 'blob:3', size: 1 })
    expect(await Promise.all(ps)).toEqual(['blob:1', 'blob:2', 'blob:3'])
    expect(peak).toBe(2)
  })

  it('caps the number of distinct images and caches repeats', async () => {
    const load = vi.fn(async (k: string) => ({ url: `blob:${k}`, size: 1 }))
    const l = new ImageLoader(load, () => {}, L)
    expect(await l.request('a')).toBe('blob:a')
    expect(await l.request('a')).toBe('blob:a')
    await l.request('b')
    await l.request('c')
    expect(await l.request('d')).toBeNull()
    expect(load).toHaveBeenCalledTimes(3)
  })

  it('passes a per-image cap clamped to the remaining budget and stops when exhausted', async () => {
    const caps: number[] = []
    const load = vi.fn(async (k: string, max: number) => (caps.push(max), { url: `blob:${k}`, size: max }))
    const l = new ImageLoader(load, () => {}, { ...L, maxImages: 10 })
    expect(await l.request('a')).toBe('blob:a')
    expect(await l.request('b')).toBe('blob:b')
    expect(await l.request('c')).toBeNull()
    expect(caps).toEqual([100, 50])
  })

  it('returns null on load errors and revokes late/finished urls on dispose', async () => {
    const revoke = vi.fn()
    const d = deferred()
    const l = new ImageLoader(async (k) => (k === 'bad' ? Promise.reject(new Error('x')) : k === 'late' ? d.p : { url: 'blob:ok', size: 1 }), revoke, L)
    expect(await l.request('bad')).toBeNull()
    expect(await l.request('ok')).toBe('blob:ok')
    const late = l.request('late')
    l.dispose()
    expect(revoke).toHaveBeenCalledWith('blob:ok')
    d.res({ url: 'blob:late', size: 1 })
    expect(await late).toBeNull()
    expect(revoke).toHaveBeenCalledWith('blob:late')
    expect(await l.request('z')).toBeNull()
  })
})

describe('previewPolicy', () => {
  it('is live at 150ms for small docs, 600ms for medium, on request when large', () => {
    expect(previewPolicy(1000)).toEqual({ large: false, debounceMs: 150 })
    expect(previewPolicy(100 * 1024)).toEqual({ large: false, debounceMs: 150 })
    expect(previewPolicy(100 * 1024 + 1)).toEqual({ large: false, debounceMs: 600 })
    expect(previewPolicy(400 * 1024)).toEqual({ large: false, debounceMs: 600 })
    expect(previewPolicy(400 * 1024 + 1).large).toBe(true)
  })
})
