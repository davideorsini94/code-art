// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from '@testing-library/svelte'
import { afterEach, describe, expect, it, vi } from 'vitest'

vi.mock('../lib/ipc', () => ({ convMarkViewed: vi.fn(async () => {}), convStart: vi.fn(async () => 'n') }))

import ConvTabs from './ConvTabs.svelte'
import { conversations } from '../lib/stores/conversations.svelte'
import type { Conversation } from '../lib/types'

const mk = (sessionId: string, title: string, status: Conversation['status'] = 'idle'): Conversation => ({
  sessionId, projectId: 'p', cwd: '/Users/dev/app', title, prevTitle: null, titleChangedAt: null, titleLocked: false,
  summary: '', status, owned: false, archived: false, lastActivity: 1, turnCount: 1, model: null, permissionMode: 'default',
  contextUsedPct: null, maybeOpenElsewhere: false,
})

afterEach(() => {
  cleanup()
  conversations.all = []
  conversations.tabs = []
  conversations.activeId = null
})

describe('ConvTabs', () => {
  it('una scheda per conversazione aperta, attiva marcata, clic seleziona, il pulsante chiude', async () => {
    conversations.all = [mk('a', 'Fatture', 'waiting'), mk('b', 'Login'), mk('c', 'Non aperta')]
    conversations.tabs = ['a', 'b']
    conversations.activeId = 'a'
    render(ConvTabs)
    const tabs = screen.getAllByRole('tab')
    expect(tabs.map((t) => t.textContent?.trim())).toEqual(['Fatture', 'Login'])
    expect(tabs[0].getAttribute('aria-selected')).toBe('true')
    expect(screen.queryByText('Non aperta')).toBeNull()
    await fireEvent.click(tabs[1])
    expect(conversations.activeId).toBe('b')
    await fireEvent.click(screen.getByLabelText('Chiudi Login'))
    expect(conversations.tabs).toEqual(['a'])
    expect(conversations.activeId).toBe('a')
  })

  it('il clic centrale chiude la scheda', async () => {
    conversations.all = [mk('a', 'Fatture')]
    conversations.tabs = ['a']
    conversations.activeId = 'a'
    render(ConvTabs)
    await fireEvent(screen.getByRole('tab'), new MouseEvent('auxclick', { button: 1, bubbles: true }))
    expect(conversations.tabs).toEqual([])
  })
})
