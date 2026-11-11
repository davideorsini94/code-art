import { describe, expect, it } from 'vitest'
import { canCollapse } from './layout'
import { clampWidths, EDITOR_RESERVE, MIN_WIDTH } from './layout'

describe('canCollapse', () => {
  it('non permette di chiudere convlist se conv è già chiusa', () => {
    expect(canCollapse('convlist', new Set(['conv']))).toBe(false)
  })
  it('non permette di chiudere conv se convlist è già chiusa', () => {
    expect(canCollapse('conv', new Set(['convlist']))).toBe(false)
  })
  it('permette di chiudere convlist se conv è aperta', () => {
    expect(canCollapse('convlist', new Set())).toBe(true)
  })
  it('permette di chiudere conv se convlist è aperta', () => {
    expect(canCollapse('conv', new Set(['sidebar']))).toBe(true)
  })
  it('non limita le altre colonne', () => {
    expect(canCollapse('sidebar', new Set(['convlist', 'conv']))).toBe(true)
  })
})

describe('clampWidths', () => {
  it('leaves widths that fit the window alone', () => {
    const w = { sidebar: 218, convlist: 270, conv: 420 }
    expect(clampWidths(w, [], 1600)).toEqual(w)
  })
  it('shrinks a layout saved on a large display so the editor stays on screen', () => {
    const out = clampWidths({ sidebar: 600, convlist: 700, conv: 900 }, [], 1200)
    const used = out.sidebar + out.convlist + out.conv
    expect(used).toBeLessThanOrEqual(1200 - EDITOR_RESERVE)
    for (const v of Object.values(out)) expect(v).toBeGreaterThanOrEqual(MIN_WIDTH)
    expect(out.conv).toBeGreaterThan(out.sidebar) // proporzioni mantenute
  })
  it('ignores collapsed columns and never goes below the minimum', () => {
    const out = clampWidths({ sidebar: 900, convlist: 900, conv: 900 }, ['sidebar'], 900)
    expect(out.sidebar).toBe(900)
    expect(out.convlist).toBeGreaterThanOrEqual(MIN_WIDTH)
    expect(out.conv).toBeGreaterThanOrEqual(MIN_WIDTH)
  })
})
