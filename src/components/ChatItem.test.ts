// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from '@testing-library/svelte'
import { afterEach, describe, expect, it, vi } from 'vitest'

const openUrl = vi.fn()
vi.mock('@tauri-apps/plugin-opener', () => ({ openUrl: (u: string) => openUrl(u) }))
const openDiff = vi.fn()
const openFile = vi.fn()
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
})
