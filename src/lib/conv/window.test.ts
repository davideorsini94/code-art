import { describe, expect, it } from 'vitest'
import { variableRange } from './window'

describe('variableRange', () => {
  const h = Array.from({ length: 100 }, (_, i) => (i % 2 ? 100 : 50)) // totale 7500
  it('starts at 0 with spacer only below', () => {
    const r = variableRange(h, 0, 300, 0)
    expect(r.start).toBe(0)
    expect(r.top).toBe(0)
    expect(r.bottom + h.slice(0, r.end).reduce((a, b) => a + b, 0)).toBe(7500)
    expect(r.end).toBeLessThan(10)
  })
  it('spacers plus rendered rows always sum to the total', () => {
    for (const st of [0, 333, 2000, 7000, 99999]) {
      const r = variableRange(h, st, 400)
      const mid = h.slice(r.start, r.end).reduce((a, b) => a + b, 0)
      expect(r.top + mid + r.bottom).toBe(7500)
    }
  })
  it('covers the viewport', () => {
    const r = variableRange(h, 1000, 400, 0)
    const top = h.slice(0, r.start).reduce((a, b) => a + b, 0)
    const bottom = top + h.slice(r.start, r.end).reduce((a, b) => a + b, 0)
    expect(top).toBeLessThanOrEqual(1000)
    expect(bottom).toBeGreaterThanOrEqual(1400)
  })
  it('handles empty', () => {
    expect(variableRange([], 0, 100)).toEqual({ start: 0, end: 0, top: 0, bottom: 0 })
  })
})
