import { describe, expect, it } from 'vitest'
import { clampWidths, EDITOR_RESERVE, editorColumnShown, gridTemplate, loadLayout, MIN_WIDTH, STORAGE_KEY } from './layout'

describe('clampWidths', () => {
  it('leaves widths that fit the window alone', () => {
    const w = { sidebar: 218, conv: 420 }
    expect(clampWidths(w, [], 1600)).toEqual(w)
  })
  it('shrinks a layout saved on a large display so the editor stays on screen', () => {
    const out = clampWidths({ sidebar: 600, conv: 900 }, [], 1000)
    const used = out.sidebar + out.conv
    expect(used).toBeLessThanOrEqual(1000 - EDITOR_RESERVE)
    for (const v of Object.values(out)) expect(v).toBeGreaterThanOrEqual(MIN_WIDTH)
    expect(out.conv).toBeGreaterThan(out.sidebar) // proporzioni mantenute
  })
  it('ignores collapsed columns and never goes below the minimum', () => {
    const out = clampWidths({ sidebar: 900, conv: 900 }, ['sidebar'], 700)
    expect(out.sidebar).toBe(900)
    expect(out.conv).toBeGreaterThanOrEqual(MIN_WIDTH)
  })
})

describe('loadLayout', () => {
  it('ignora la vecchia colonna convlist e lascia richiudere conv', () => {
    const raw = JSON.stringify({ widths: { sidebar: 300, convlist: 270, conv: 500 }, collapsed: ['convlist', 'conv'] })
    const out = loadLayout({ getItem: (k: string) => (k === STORAGE_KEY ? raw : null) })
    expect(out).toEqual({ widths: { sidebar: 300, conv: 500 }, collapsed: ['conv'] })
  })
})

describe('editor column', () => {
  const w = { sidebar: 260, conv: 460 }
  it('is shown with the editor as the flexible column when there are files or terminals', () => {
    expect(editorColumnShown([], true)).toBe(true)
    expect(gridTemplate(w, [], true)).toBe('46px 260px 6px 460px 6px minmax(240px, 1fr)')
  })
  it('is closed without files and terminals: the conversation takes the free space', () => {
    expect(editorColumnShown([], false)).toBe(false)
    expect(gridTemplate(w, [], false)).toBe('46px 260px 6px minmax(240px, 1fr) 0px')
  })
  it('closed with the conversation hidden: the side panel takes the free space', () => {
    expect(gridTemplate(w, ['conv'], false)).toBe('46px minmax(240px, 1fr) 0px 0px 0px')
  })
  it('stays shown (empty) when every other column is closed, so the window is never blank', () => {
    expect(editorColumnShown(['sidebar', 'conv'], false)).toBe(true)
  })
})
