// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte'
import { afterEach, describe, expect, it, vi } from 'vitest'

vi.mock('../lib/events', () => ({ listenProject: vi.fn(async () => () => {}) }))
vi.mock('../lib/ipc', () => ({
  fsList: vi.fn(async (dir: string) => (dir === '/Users/dev/app' ? [{ name: 'src', path: '/Users/dev/app/src', isDir: true, ignored: false }] : [])),
  fsCreate: vi.fn(),
  fsRename: vi.fn(),
  fsTrash: vi.fn(),
  asFsError: (e: unknown) => ({ code: 'io', message: String(e) }),
}))
const project = vi.hoisted(() => ({ project: { id: 'p1', name: 'app', folders: ['/Users/dev/app'] }, untrusted: [] as string[] }))
vi.mock('../lib/stores/project.svelte', () => ({ projectStore: project }))
const terminalsMock = vi.hoisted(() => ({ open: vi.fn() }))
vi.mock('../lib/stores/terminals.svelte', () => ({ terminals: terminalsMock }))
vi.mock('../lib/stores/editor.svelte', () => ({ editor: { active: null, open: vi.fn(), repath: vi.fn(), closeUnder: vi.fn() } }))

import FileTree from './FileTree.svelte'

afterEach(() => {
  cleanup()
  project.untrusted = []
  vi.clearAllMocks()
})

describe('FileTree project root', () => {
  it('cannot be renamed, trashed or dragged; a subfolder can', async () => {
    render(FileTree)
    const root = screen.getByText('app').closest('[role="treeitem"]')!
    expect(root.getAttribute('draggable')).toBe('false')
    await fireEvent.contextMenu(root)
    expect(screen.getByText('Nuovo file')).toBeTruthy()
    expect(screen.queryByText('Rinomina')).toBeNull()
    expect(screen.queryByText('Sposta nel Cestino')).toBeNull()
    await fireEvent.keyDown(window, { key: 'Escape' })
    expect(screen.queryByRole('menu')).toBeNull()

    await waitFor(() => expect(screen.getByText('src')).toBeTruthy())
    const sub = screen.getByText('src').closest('[role="treeitem"]')!
    expect(sub.getAttribute('draggable')).toBe('true')
    await fireEvent.contextMenu(sub)
    expect(screen.getByText('Rinomina')).toBeTruthy()
    expect(screen.getByText('Sposta nel Cestino')).toBeTruthy()
  })
})

describe('FileTree: Apri terminale qui', () => {
  it('opens a terminal in the root folder and in a subfolder', async () => {
    render(FileTree)
    await fireEvent.contextMenu(screen.getByText('app').closest('[role="treeitem"]')!)
    await fireEvent.click(screen.getByText('Apri terminale qui'))
    expect(terminalsMock.open).toHaveBeenCalledWith('/Users/dev/app')

    await waitFor(() => expect(screen.getByText('src')).toBeTruthy())
    await fireEvent.contextMenu(screen.getByText('src').closest('[role="treeitem"]')!)
    await fireEvent.click(screen.getByText('Apri terminale qui'))
    expect(terminalsMock.open).toHaveBeenLastCalledWith('/Users/dev/app/src')
  })

  it('opens the containing folder for a file', async () => {
    const ipc = await import('../lib/ipc')
    vi.mocked(ipc.fsList).mockImplementation(async (dir: string) =>
      dir === '/Users/dev/app' ? [{ name: 'a.ts', path: '/Users/dev/app/a.ts', isDir: false, ignored: false }] : [])
    render(FileTree)
    await waitFor(() => expect(screen.getByText('a.ts')).toBeTruthy())
    await fireEvent.contextMenu(screen.getByText('a.ts').closest('[role="treeitem"]')!)
    await fireEvent.click(screen.getByText('Apri terminale qui'))
    expect(terminalsMock.open).toHaveBeenCalledWith('/Users/dev/app')
  })

  it('is disabled for an untrusted project', async () => {
    project.untrusted = ['/Users/dev/app']
    render(FileTree)
    await fireEvent.contextMenu(screen.getByText('app').closest('[role="treeitem"]')!)
    const item = screen.getByText('Apri terminale qui')
    expect(item.getAttribute('aria-disabled')).toBe('true')
    await fireEvent.click(item)
    expect(terminalsMock.open).not.toHaveBeenCalled()
  })
})
