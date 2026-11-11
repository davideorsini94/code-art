// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from '@testing-library/svelte'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { tick } from 'svelte'

const openUrl = vi.fn()
vi.mock('@tauri-apps/plugin-opener', () => ({ openUrl: (u: string) => openUrl(u) }))
const openDiff = vi.fn()
const openFile = vi.fn()
const convSend = vi.fn(async (..._a: unknown[]) => {})
vi.mock('../lib/ipc', () => ({ convSend: (...a: unknown[]) => convSend(...a) }))
const addUser = vi.fn()
vi.mock('../lib/stores/chat.svelte', () => ({ chat: { addUser: (...a: unknown[]) => addUser(...a) } }))
const project = vi.hoisted(() => ({ untrusted: [] as string[] }))
vi.mock('../lib/stores/project.svelte', () => ({ projectStore: project }))
const convs = vi.hoisted(() => ({ all: [] as { sessionId: string; status: string }[] }))
vi.mock('../lib/stores/conversations.svelte', () => ({ conversations: convs }))
vi.mock('../lib/stores/editor.svelte', () => ({ editor: { openDiff: (...a: unknown[]) => openDiff(...a), open: (...a: unknown[]) => openFile(...a) } }))

import ChatItem from './ChatItem.svelte'
import type { Row } from '../lib/conv/reduce'
import type { ChatItem as Item } from '../lib/types'

const row = (item: Item, children: Item[] = []): Row => ({ item, children })
const tool = (over: Partial<Extract<Item, { kind: 'tool' }>> = {}): Extract<Item, { kind: 'tool' }> => ({
  kind: 'tool', id: 't', name: 'Read', input: { file_path: '/Users/dev/p/a.txt' }, result: null, ok: null, parentId: null, filePath: '/Users/dev/p/a.txt', ...over,
})
afterEach(() => {
  cleanup()
  vi.clearAllMocks()
  vi.useRealTimers()
  convs.all = []
})

describe('ChatItem', () => {
  it('assistant markdown is sanitised and links go to the system browser', async () => {
    const { container } = render(ChatItem, { row: row({ kind: 'assistant', id: 'a', text: '<img src=x onerror=alert(1)>\n\n<script>x</script>\n\n[sito](https://example.com) [x](javascript:alert(1))' }), sessionId: 's' })
    expect(container.innerHTML).not.toContain('onerror')
    expect(container.innerHTML).not.toContain('<script')
    expect(container.innerHTML).not.toContain('javascript:')
    await fireEvent.click(screen.getByText('sito'))
    expect(openUrl).toHaveBeenCalledWith('https://example.com')
  })

  it('tool result and user text are plain text, never html', async () => {
    const evil = '<img src=x onerror=alert(1)><b>x</b>'
    const { container } = render(ChatItem, { row: row(tool({ result: evil, ok: true })), sessionId: 's' })
    await fireEvent.click(screen.getByRole('button', { name: /Read/ }))
    expect(container.querySelector('img')).toBeNull()
    expect(container.querySelector('b')).toBeNull()
    expect(container.textContent).toContain(evil)
    cleanup()
    const u = render(ChatItem, { row: row({ kind: 'user', id: 'u', text: evil, images: [] }), sessionId: 's' })
    expect(u.container.querySelector('img')).toBeNull()
    expect(u.container.textContent).toContain(evil)
  })

  it('long results are capped at 20000 chars until "mostra tutto"', async () => {
    const big = 'x'.repeat(25_000)
    const { container } = render(ChatItem, { row: row(tool({ result: big, ok: true })), sessionId: 's' })
    await fireEvent.click(screen.getByRole('button', { name: /Read/ }))
    const pres = [...container.querySelectorAll('pre')]
    expect(pres.some((p) => p.textContent!.length === 20_000)).toBe(true)
    await fireEvent.click(screen.getByText(/mostra tutto/))
    expect([...container.querySelectorAll('pre')].some((p) => p.textContent!.length === 25_000)).toBe(true)
  })

  it('Edit has "Apri diff" calling editor.openDiff; Write opens the file', async () => {
    render(ChatItem, { row: row(tool({ name: 'Edit', input: { file_path: '/Users/dev/p/a.txt', old_string: 'red', new_string: 'blue' } })), sessionId: 's' })
    await fireEvent.click(screen.getByText('Apri diff'))
    expect(openDiff).toHaveBeenCalledWith('/Users/dev/p/a.txt', 'red', 'blue')
    cleanup()
    render(ChatItem, { row: row(tool({ name: 'Write', input: { file_path: '/Users/dev/p/a.txt', content: 'hi' } })), sessionId: 's' })
    await fireEvent.click(screen.getByText('Apri file'))
    expect(openFile).toHaveBeenCalledWith('/Users/dev/p/a.txt')
  })

  it('subagent block is collapsed, shows the step count and nests children', async () => {
    const parent = tool({ id: 'p', name: 'Agent', input: { description: 'Leggi a.txt' }, result: 'color = blue', ok: true, filePath: null })
    const child = tool({ id: 'c', parentId: 'p' })
    render(ChatItem, { row: row(parent, [child]), sessionId: 's' })
    expect(screen.getByText('1 passo ✓')).toBeTruthy()
    expect(screen.queryByText('color = blue')).toBeNull()
    await fireEvent.click(screen.getByRole('button', { name: /Subagent/ }))
    expect(screen.getByText('color = blue')).toBeTruthy()
    expect(screen.getByRole('button', { name: /Read/ })).toBeTruthy()
  })

  it('"Errore di Claude" is an error notice with plain text', () => {
    const evil = '<img src=x onerror=alert(1)>API Error: 529'
    const { container } = render(ChatItem, { row: row({ kind: 'raw', id: 'e', label: 'Errore di Claude', json: evil }), sessionId: 's' })
    const note = screen.getByRole('alert')
    expect(note.className).toContain('errnote')
    expect(note.textContent).toContain(evil)
    expect(container.querySelector('img')).toBeNull()
  })

  it('"Turno interrotto": Riprendi sends "Continua" like the composer; disabled in untrusted projects', async () => {
    const item: Item = { kind: 'raw', id: 'i', label: 'Turno interrotto — Riprendi', json: '' }
    render(ChatItem, { row: row(item), sessionId: 's' })
    expect(screen.getByText('Il turno è stato interrotto.')).toBeTruthy()
    await fireEvent.click(screen.getByText('Riprendi'))
    expect(convSend).toHaveBeenCalledWith('s', 'Continua', [])
    await new Promise((r) => setTimeout(r, 0))
    expect(addUser).toHaveBeenCalledWith('s', 'Continua')
    cleanup()
    convSend.mockClear()
    project.untrusted = ['/Users/dev/x']
    render(ChatItem, { row: row(item), sessionId: 's' })
    expect(screen.getByText('Riprendi').getAttribute('aria-disabled')).toBe('true')
    await fireEvent.click(screen.getByText('Riprendi'))
    expect(convSend).not.toHaveBeenCalled()
    project.untrusted = []
  })

  it('a user message with images says how many were attached', () => {
    render(ChatItem, { row: row({ kind: 'user', id: 'u', text: 'guarda', images: ['/tmp/a.png', '/tmp/b.png'] }), sessionId: 's' })
    expect(screen.getByText('2 immagini allegate')).toBeTruthy()
    cleanup()
    render(ChatItem, { row: row({ kind: 'user', id: 'u', text: 'guarda', images: ['/tmp/a.png'] }), sessionId: 's' })
    expect(screen.getByText('1 immagine allegata')).toBeTruthy()
    cleanup()
    render(ChatItem, { row: row({ kind: 'user', id: 'u', text: 'guarda', images: [] }), sessionId: 's' })
    expect(screen.queryByText(/allegat/)).toBeNull()
  })

  it('while the conversation is working the streaming text is re-rendered at most every ~100 ms', async () => {
    vi.useFakeTimers()
    convs.all = [{ sessionId: 's', status: 'working' }]
    const a = (text: string) => row({ kind: 'assistant', id: 'a', text })
    const { container, rerender } = render(ChatItem, { row: a('uno'), sessionId: 's' })
    expect(container.textContent).toContain('uno')
    await rerender({ row: a('uno due'), sessionId: 's' })
    await rerender({ row: a('uno due tre'), sessionId: 's' })
    expect(container.textContent).not.toContain('tre')
    await vi.advanceTimersByTimeAsync(120)
    await tick()
    expect(container.textContent).toContain('uno due tre')
    // a turno finito il testo si aggiorna subito
    convs.all = [{ sessionId: 's', status: 'idle' }]
    await rerender({ row: a('fine'), sessionId: 's' })
    expect(container.textContent).toContain('fine')
  })
})
