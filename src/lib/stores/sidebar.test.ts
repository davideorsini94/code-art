// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from 'vitest'
import { SidebarStore } from './sidebar.svelte'

afterEach(() => {
  vi.restoreAllMocks()
  localStorage.clear()
})

describe('sidebar store', () => {
  it('defaults to files', () => {
    const s = new SidebarStore()
    s.bind('p1')
    expect(s.view).toBe('files')
  })

  it('persists the view per project', () => {
    const a = new SidebarStore()
    a.bind('p1')
    a.setView('git')
    expect(localStorage.getItem('codeart.sidebar.p1')).toBe('git')
    const b = new SidebarStore()
    b.bind('p1')
    expect(b.view).toBe('git')
    b.bind('p2')
    expect(b.view).toBe('files')
  })

  it('ignores an invalid saved value', () => {
    localStorage.setItem('codeart.sidebar.p1', 'boh')
    const s = new SidebarStore()
    s.bind('p1')
    expect(s.view).toBe('files')
  })

  it('does not break when localStorage throws', () => {
    vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => {
      throw new Error('negato')
    })
    vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
      throw new Error('negato')
    })
    const s = new SidebarStore()
    expect(() => s.bind('p1')).not.toThrow()
    expect(() => s.setView('search')).not.toThrow()
    expect(s.view).toBe('search')
  })
})
