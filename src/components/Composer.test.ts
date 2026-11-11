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
import { projectStore } from '../lib/stores/project.svelte'
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
  projectStore.untrusted = []
  vi.stubGlobal('URL', Object.assign(URL, { createObjectURL: vi.fn(() => 'blob:x'), revokeObjectURL: vi.fn() }))
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
    expect((screen.getByRole('button', { name: 'Invia' }) as HTMLButtonElement).disabled).toBe(true)
    await fireEvent.click(screen.getByRole('button', { name: 'Interrompi' }))
    await fireEvent.keyDown(ta, { key: 'Escape' })
    expect(ipc.convInterrupt).toHaveBeenCalledTimes(2)
  })

  it('bypassPermissions chip is amber; modes and models call the right commands', async () => {
    const { container } = render(Composer, { conv: { ...base, sessionId: 'sm', permissionMode: 'bypassPermissions' } })
    expect(container.querySelector('.chip.mode')?.classList.contains('bypass')).toBe(true)
    await fireEvent.change(screen.getByLabelText('Modalità permessi'), { target: { value: 'plan' } })
    expect(ipc.convSetPermissionMode).toHaveBeenCalledWith('sm', 'plan')
    await fireEvent.change(screen.getByLabelText('Modello'), { target: { value: 'opus' } })
    expect(ipc.convSetModel).toHaveBeenCalledWith('sm', 'opus')
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

  it('while working the placeholder says that typing is possible and sending waits for the turn end', () => {
    const { rerender } = render(Composer, { conv: { ...base, sessionId: 'sw', status: 'idle' } })
    const ta = screen.getByLabelText('Messaggio') as HTMLTextAreaElement
    expect(ta.placeholder).toBe('Rispondi a Claude…')
    return rerender({ conv: { ...base, sessionId: 'sw', status: 'working' } }).then(() => {
      expect(ta.placeholder).toBe('Claude sta lavorando — puoi scrivere, invierai a fine turno')
    })
  })

  it('model picker lists fable, opus, sonnet, haiku in order with friendly labels', () => {
    render(Composer, { conv: { ...base, sessionId: 'sf', model: 'sonnet' } })
    const opts = [...(screen.getByLabelText('Modello') as HTMLSelectElement).options]
    expect(opts.map((o) => o.value)).toEqual(['fable', 'opus', 'sonnet', 'haiku'])
    expect(opts.map((o) => o.textContent)).toEqual(['Fable', 'Opus', 'Sonnet', 'Haiku'])
  })

  it('a model reported by the process that is not an alias is still listed and selected', async () => {
    conversations.setInit('sg', [], 'claude-opus-4')
    render(Composer, { conv: { ...base, sessionId: 'sg', model: null } })
    const sel = screen.getByLabelText('Modello') as HTMLSelectElement
    expect([...sel.options].map((o) => o.value)).toEqual(['fable', 'opus', 'sonnet', 'haiku', 'claude-opus-4'])
    expect(sel.value).toBe('claude-opus-4')
  })

  it('choosing Fable sets the model to the fable alias', async () => {
    render(Composer, { conv: { ...base, sessionId: 'sh', model: 'sonnet' } })
    await fireEvent.change(screen.getByLabelText('Modello'), { target: { value: 'fable' } })
    expect(ipc.convSetModel).toHaveBeenCalledWith('sh', 'fable')
  })

  it('shows the context ring only when known', () => {
    render(Composer, { conv: base })
    expect(screen.queryByRole('img', { name: /Contesto usato/ })).toBeNull()
    cleanup()
    render(Composer, { conv: { ...base, contextUsedPct: 42.4 } })
    expect(screen.getByRole('img', { name: 'Contesto usato: 42%' })).toBeTruthy()
  })

  it('restores the draft for the conversation', () => {
    localStorage.setItem('codeart.draft.s1', 'bozza')
    render(Composer, { conv: base })
    expect((screen.getByLabelText('Messaggio') as HTMLTextAreaElement).value).toBe('bozza')
  })

  it('after a successful model choice the picker shows conv.model, not the older init model', async () => {
    conversations.setInit('s5', [], 'claude-opus-4')
    const { rerender } = render(Composer, { conv: { ...base, sessionId: 's5', model: 'sonnet' } })
    const sel = screen.getByLabelText('Modello') as HTMLSelectElement
    expect(sel.value).toBe('claude-opus-4')
    await new Promise((r) => setTimeout(r, 2))
    await fireEvent.change(sel, { target: { value: 'haiku' } })
    await waitFor(() => expect(ipc.convSetModel).toHaveBeenCalledWith('s5', 'haiku'))
    await rerender({ conv: { ...base, sessionId: 's5', model: 'haiku' } })
    await waitFor(() => expect(sel.value).toBe('haiku'))
  })

  it('a failed model/mode change puts the select back to the real value', async () => {
    vi.mocked(ipc.convSetPermissionMode).mockRejectedValueOnce('errore')
    render(Composer, { conv: { ...base, sessionId: 's6' } })
    const sel = screen.getByLabelText('Modalità permessi') as HTMLSelectElement
    await fireEvent.change(sel, { target: { value: 'plan' } })
    await screen.findByText('errore')
    expect(sel.value).toBe('default')
  })

  const png = (n = 8, name = 'a.png') => new File([new Uint8Array(n)], name, { type: 'image/png' })
  const paste = (ta: HTMLElement, files: File[]) => fireEvent.paste(ta, { clipboardData: { files } })

  it('pasting an image shows a removable thumbnail; sending saves it once and passes the path', async () => {
    render(Composer, { conv: base })
    const ta = screen.getByLabelText('Messaggio')
    await paste(ta, [png()])
    await screen.findByAltText('a.png')
    await fireEvent.input(ta, { target: { value: 'guarda' } })
    vi.mocked(ipc.convSend).mockRejectedValueOnce('boom')
    await fireEvent.keyDown(ta, { key: 'Enter', metaKey: true })
    await screen.findByText('boom')
    await fireEvent.keyDown(ta, { key: 'Enter', metaKey: true })
    await waitFor(() => expect(ipc.convSend).toHaveBeenLastCalledWith('s1', 'guarda', ['/tmp/CodeArt/images/x.png']))
    expect(ipc.convSaveImage).toHaveBeenCalledTimes(1)
    await waitFor(() => expect(screen.queryByAltText('a.png')).toBeNull())
  })

  it('removing a thumbnail drops it', async () => {
    render(Composer, { conv: base })
    await paste(screen.getByLabelText('Messaggio'), [png()])
    await fireEvent.click(await screen.findByLabelText('Rimuovi a.png'))
    expect(screen.queryByAltText('a.png')).toBeNull()
  })

  it('accepts at most 10 images per message', async () => {
    render(Composer, { conv: base })
    const ta = screen.getByLabelText('Messaggio')
    for (let i = 0; i < 11; i++) await paste(ta, [png(8, `i${i}.png`)])
    await screen.findByText('al massimo 10 immagini per messaggio')
    expect(screen.getAllByRole('img').filter((e) => e.tagName === 'IMG')).toHaveLength(10)
  })

  it('refuses images beyond 30 MB in total', async () => {
    render(Composer, { conv: base })
    const ta = screen.getByLabelText('Messaggio')
    for (const n of [1, 2]) await paste(ta, [png(14 * 1024 * 1024, `g${n}.png`)])
    await paste(ta, [png(14 * 1024 * 1024, 'g3.png')])
    await screen.findByText('immagini troppo grandi: al massimo 30 MB per messaggio')
    expect(screen.queryByAltText('g3.png')).toBeNull()
  })

  it("does not wipe text typed while the send is in flight", async () => {
    let done!: () => void
    vi.mocked(ipc.convSend).mockImplementationOnce(() => new Promise<void>((r) => (done = r)))
    render(Composer, { conv: base })
    const ta = screen.getByLabelText('Messaggio') as HTMLTextAreaElement
    await fireEvent.input(ta, { target: { value: 'uno' } })
    await fireEvent.keyDown(ta, { key: 'Enter', metaKey: true })
    await fireEvent.input(ta, { target: { value: 'uno due' } })
    done()
    await waitFor(() => expect(ipc.convSend).toHaveBeenCalled())
    await new Promise((r) => setTimeout(r, 5))
    expect(ta.value).toBe('uno due')
  })

  it('archiving a conversation removes its draft', async () => {
    localStorage.setItem('codeart.draft.sz', 'x')
    await conversations.archive('sz', true)
    expect(localStorage.getItem('codeart.draft.sz')).toBeNull()
  })

  it('untrusted folder: send is disabled with the reason, before any click', async () => {
    projectStore.untrusted = ['/Users/dev/app']
    render(Composer, { conv: { ...base, sessionId: 'su' } })
    const ta = screen.getByLabelText('Messaggio')
    await fireEvent.input(ta, { target: { value: 'ciao' } })
    expect((screen.getByRole('button', { name: 'Invia' }) as HTMLButtonElement).disabled).toBe(true)
    await fireEvent.keyDown(ta, { key: 'Enter', metaKey: true })
    expect(ipc.convSend).not.toHaveBeenCalled()
    expect(screen.getByText('Cartella non fidata — sola lettura')).toBeTruthy()
  })

  it('command errors use the readable text, never the raw code', async () => {
    vi.mocked(ipc.convSetModel).mockRejectedValueOnce('untrusted_folder')
    render(Composer, { conv: { ...base, sessionId: 'se' } })
    await fireEvent.change(screen.getByLabelText('Modello') as HTMLSelectElement, { target: { value: 'opus' } })
    await screen.findByText('Cartella non fidata: sola lettura, Claude e terminale disattivati')
    expect(screen.queryByText('untrusted_folder')).toBeNull()
  })

  it('warns above the composer when the conversation may be open elsewhere', () => {
    render(Composer, { conv: { ...base, sessionId: 'sx', maybeOpenElsewhere: true } })
    expect(screen.getByRole('note').textContent).toMatch(/aperta anche in un altro programma/)
  })
})
