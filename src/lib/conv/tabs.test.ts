// @vitest-environment jsdom
import { afterEach, describe, expect, it } from 'vitest'
import { closeTab, loadTabs, openTab, pruneTabs, saveTabs } from './tabs'

afterEach(() => localStorage.clear())

describe('schede delle conversazioni', () => {
  it('apre in coda e non duplica', () => {
    expect(openTab([], 'a')).toEqual(['a'])
    expect(openTab(['a', 'b'], 'c')).toEqual(['a', 'b', 'c'])
    const t = ['a', 'b']
    expect(openTab(t, 'a')).toBe(t)
  })

  it("chiudendo l'attiva al centro passa a quella a destra", () => {
    expect(closeTab(['a', 'b', 'c'], 'b', 'b')).toEqual({ tabs: ['a', 'c'], activeId: 'c' })
  })

  it("chiudendo l'ultima attiva passa a quella a sinistra", () => {
    expect(closeTab(['a', 'b', 'c'], 'c', 'c')).toEqual({ tabs: ['a', 'b'], activeId: 'b' })
  })

  it("chiudendo l'unica non resta nessuna attiva", () => {
    expect(closeTab(['a'], 'a', 'a')).toEqual({ tabs: [], activeId: null })
  })

  it("chiudendo una non attiva l'attiva resta", () => {
    expect(closeTab(['a', 'b', 'c'], 'c', 'a')).toEqual({ tabs: ['b', 'c'], activeId: 'c' })
  })

  it('chiudere una scheda che non c’è non cambia nulla', () => {
    expect(closeTab(['a'], 'a', 'z')).toEqual({ tabs: ['a'], activeId: 'a' })
  })

  it('scarta le schede di conversazioni sparite', () => {
    expect(pruneTabs(['a', 'x', 'b'], new Set(['a', 'b']))).toEqual(['a', 'b'])
  })

  it('salva e rilegge per progetto', () => {
    saveTabs('p1', { tabs: ['a', 'b'], activeId: 'b' })
    expect(loadTabs('p1')).toEqual({ tabs: ['a', 'b'], activeId: 'b' })
    expect(loadTabs('p2')).toEqual({ tabs: [], activeId: null })
  })

  it('un valore salvato rotto vale come vuoto', () => {
    localStorage.setItem('codeart.convtabs.p1', '{rotto')
    expect(loadTabs('p1')).toEqual({ tabs: [], activeId: null })
    localStorage.setItem('codeart.convtabs.p1', JSON.stringify({ tabs: [1, 'a', null], activeId: 7 }))
    expect(loadTabs('p1')).toEqual({ tabs: ['a'], activeId: null })
  })
})
