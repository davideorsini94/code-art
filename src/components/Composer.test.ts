// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }))
vi.mock('../lib/ipc', () => ({
  convSend: vi.fn(async () => {}),
  convInterrupt: vi.fn(async () => {}),
  convSetModel: vi.fn(async () => {}),
  convSetPermissionMode: vi.fn(async () => {}),
  convSaveImage: vi.fn(async () => '/tmp/CodeArt/images/x.png'),
  convHistory: vi.fn(async () => []),
  convPending: vi.fn(async () => []),
  convArchive: vi.fn(async () => {}),
  convMarkViewed: vi.fn(),
  convList: vi.fn(),
  convStart: vi.fn(),
  convRename: vi.fn(),
  convUnlockTitle: vi.fn(),
}))

import Composer from './Composer.svelte'
import { chat } from '../lib/stores/chat.svelte'
import { conversations } from '../lib/stores/conversations.svelte'
import * as ipc from '../lib/ipc'
import type { Conversation } from '../lib/types'

const base: Conversation = {
  sessionId: 's1', projectId: 'p', cwd: '/Users/dev/app', title: 'T', prevTitle: null, titleChangedAt: null, titleLocked: false,
  summary: '', status: 'idle', owned: false, archived: false, lastActivity: 0, turnCount: 0, model: 'sonnet', permissionMode: 'default',
  contextUsedPct: null, maybeOpenElsewhere: false,
}

beforeEach(() => {
  localStorage.clear()
  vi.clearAllMocks()
  conversations.inits = {}
})
afterEach(cleanup)

describe('Composer', () => {
  it('sends on ⌘↵ with the typed text as data, shows it locally and clears the draft', async () => {
    const add = vi.spyOn(chat, 'addUser')
    render(Composer, { conv: base })
    const ta = screen.getByLabelText('Messaggio') as HTMLTextAreaElement
    await fireEvent.input(ta, { target: { value: '<img src=x onerror=1> `rm -rf`' } })
    await fireEvent.keyDown(ta, { key: 'Enter', metaKey: true })
    await waitFor(() => expect(ipc.convSend).toHaveBeenCalledWith('s1', '<img src=x onerror=1> `rm -rf`', []))
    await waitFor(() => expect(add).toHaveBeenCalledWith('s1', '<img src=x onerror=1> `rm -rf`', []))
    expect(ta.value).toBe('')
    expect(localStorage.getItem('codeart.draft.s1')).toBeNull()
  })

  it('plain Enter does not send', async () => {
    render(Composer, { conv: base })
    const ta = screen.getByLabelText('Messaggio')
    await fireEvent.input(ta, { target: { value: 'ciao' } })
    await fireEvent.keyDown(ta, { key: 'Enter' })
    expect(ipc.convSend).not.toHaveBeenCalled()
  })

  it('keeps the text and shows the error when sending fails', async () => {
    vi.mocked(ipc.convSend).mockRejectedValueOnce('claude non trovato')
    render(Composer, { conv: base })
    const ta = screen.getByLabelText('Messaggio') as HTMLTextAreaElement
    await fireEvent.input(ta, { target: { value: 'ciao' } })
    await fireEvent.keyDown(ta, { key: 'Enter', metaKey: true })
    await screen.findByText('claude non trovato')
    expect(ta.value).toBe('ciao')
  })

  it('while working: Interrompi (button and Esc), send disabled with tooltip', async () => {
    render(Composer, { conv: { ...base, status: 'working' } })
    const ta = screen.getByLabelText('Messaggio')
    await fireEvent.input(ta, { target: { value: 'altro' } })
    await fireEvent.keyDown(ta, { key: 'Enter', metaKey: true })
    expect(ipc.convSend).not.toHaveBeenCalled()
    expect((screen.getByTitle('Attendi la fine del turno per inviare') as HTMLButtonElement).disabled).toBe(true)
    await fireEvent.click(screen.getByRole('button', { name: 'Interrompi' }))
    await fireEvent.keyDown(ta, { key: 'Escape' })
    expect(ipc.convInterrupt).toHaveBeenCalledTimes(2)
  })

  it('bypassPermissions chip is amber; modes and models call the right commands', async () => {
    const { container } = render(Composer, { conv: { ...base, permissionMode: 'bypassPermissions' } })
    expect(container.querySelector('.chip.mode')?.classList.contains('bypass')).toBe(true)
    await fireEvent.change(screen.getByLabelText('Modalità permessi'), { target: { value: 'plan' } })
    expect(ipc.convSetPermissionMode).toHaveBeenCalledWith('s1', 'plan')
    await fireEvent.change(screen.getByLabelText('Modello'), { target: { value: 'opus' } })
    expect(ipc.convSetModel).toHaveBeenCalledWith('s1', 'opus')
  })

  it('offers built-in commands, then the ones from init; selecting inserts the command', async () => {
    const { rerender } = render(Composer, { conv: base })
    const ta = screen.getByLabelText('Messaggio') as HTMLTextAreaElement
    await fireEvent.input(ta, { target: { value: '/co' } })
    expect(screen.getByRole('option', { name: '/compact' })).toBeTruthy()
    conversations.setInit('s1', ['review', 'commit'], 'claude-opus-4')
    await rerender({ conv: base })
    await fireEvent.input(ta, { target: { value: '/c' } })
    expect(screen.queryByRole('option', { name: '/compact' })).toBeNull()
    await fireEvent.click(screen.getByRole('option', { name: '/commit' }).querySelector('button')!)
    expect(ta.value).toBe('/commit ')
    expect((screen.getByLabelText('Modello') as HTMLSelectElement).value).toBe('claude-opus-4')
  })

  it('shows the context ring only when known', () => {
    render(Composer, { conv: base })
    expect(screen.queryByTitle(/Contesto usato/)).toBeNull()
    cleanup()
    render(Composer, { conv: { ...base, contextUsedPct: 42.4 } })
    expect(screen.getByTitle('Contesto usato: 42%')).toBeTruthy()
  })

  it('restores the draft for the conversation', () => {
    localStorage.setItem('codeart.draft.s1', 'bozza')
    render(Composer, { conv: base })
    expect((screen.getByLabelText('Messaggio') as HTMLTextAreaElement).value).toBe('bozza')
  })
})
