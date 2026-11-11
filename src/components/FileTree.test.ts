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
vi.mock('../lib/stores/project.svelte', () => ({
  projectStore: { project: { id: 'p1', name: 'app', folders: ['/Users/dev/app'] }, untrusted: [] },
}))
vi.mock('../lib/stores/editor.svelte', () => ({ editor: { active: null, open: vi.fn(), repath: vi.fn(), closeUnder: vi.fn() } }))

import FileTree from './FileTree.svelte'

afterEach(cleanup)

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
