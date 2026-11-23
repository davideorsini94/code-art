// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from '@testing-library/svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import Rail from './Rail.svelte'
import { sidebar } from '../lib/stores/sidebar.svelte'
import { resetTooltipForTests } from '../lib/tooltip'

beforeEach(() => {
  sidebar.bind('rail-test')
  sidebar.setView('files')
})
afterEach(() => {
  cleanup()
  resetTooltipForTests()
  localStorage.clear()
})

const pressed = (name: string) => screen.getByRole('button', { name }).getAttribute('aria-pressed')

describe('Rail views', () => {
  it('has three view buttons with their shortcut in the tooltip', () => {
    vi.useFakeTimers()
    try {
      render(Rail, { collapsed: new Set<string>(), ontoggle: () => {} })
      const expected = { Esplora: '⌘⇧E', Cerca: '⌘⇧F', Git: '⌘⇧G' }
      for (const [name, key] of Object.entries(expected)) {
        const btn = screen.getByRole('button', { name })
        btn.dispatchEvent(new PointerEvent('pointerover', { bubbles: true }))
        vi.advanceTimersByTime(500)
        expect(document.getElementById('codeart-tooltip')!.textContent).toContain(key)
        btn.dispatchEvent(new PointerEvent('pointerout', { bubbles: true }))
      }
    } finally {
      vi.useRealTimers()
    }
  })

  it('marks only the active view as pressed', () => {
    render(Rail, { collapsed: new Set<string>(), ontoggle: () => {} })
    expect(pressed('Esplora')).toBe('true')
    expect(pressed('Cerca')).toBe('false')
    expect(pressed('Git')).toBe('false')
  })

  it('no view is pressed while the panel is closed', () => {
    render(Rail, { collapsed: new Set(['sidebar']), ontoggle: () => {} })
    expect(pressed('Esplora')).toBe('false')
  })

  it('clicking the active view with the panel open closes it', async () => {
    const ontoggle = vi.fn()
    render(Rail, { collapsed: new Set<string>(), ontoggle })
    await fireEvent.click(screen.getByRole('button', { name: 'Esplora' }))
    expect(ontoggle).toHaveBeenCalledWith('sidebar')
    expect(sidebar.view).toBe('files')
  })

  it('clicking another view switches to it without closing the panel', async () => {
    const ontoggle = vi.fn()
    render(Rail, { collapsed: new Set<string>(), ontoggle })
    await fireEvent.click(screen.getByRole('button', { name: 'Git' }))
    expect(sidebar.view).toBe('git')
    expect(ontoggle).not.toHaveBeenCalled()
    expect(pressed('Git')).toBe('true')
  })

  it('clicking a view with the panel closed opens it on that view', async () => {
    const ontoggle = vi.fn()
    render(Rail, { collapsed: new Set(['sidebar']), ontoggle })
    await fireEvent.click(screen.getByRole('button', { name: 'Cerca' }))
    expect(sidebar.view).toBe('search')
    expect(ontoggle).toHaveBeenCalledWith('sidebar')
  })

  it('Conversazioni è una vista laterale; la colonna della conversazione resta un interruttore', async () => {
    const ontoggle = vi.fn()
    render(Rail, { collapsed: new Set(['sidebar']), ontoggle })
    await fireEvent.click(screen.getByRole('button', { name: 'Conversazioni' }))
    expect(sidebar.view).toBe('convs')
    expect(ontoggle).toHaveBeenCalledWith('sidebar')
    await fireEvent.click(screen.getByRole('button', { name: 'Conversazione' }))
    expect(ontoggle).toHaveBeenCalledWith('conv')
  })

  it('Impostazioni apre la sua vista nel pannello laterale', async () => {
    const ontoggle = vi.fn()
    render(Rail, { collapsed: new Set(['sidebar']), ontoggle })
    const btn = screen.getByRole('button', { name: 'Impostazioni' })
    expect(btn.getAttribute('aria-disabled')).toBeNull()
    await fireEvent.click(btn)
    expect(sidebar.view).toBe('settings')
    expect(ontoggle).toHaveBeenCalledWith('sidebar')
  })
})
