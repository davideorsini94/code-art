// @vitest-environment jsdom
import { cleanup, createEvent, fireEvent, render, screen, waitFor } from '@testing-library/svelte'
import { afterEach, describe, expect, it, vi } from 'vitest'

vi.mock('../../lib/events', () => ({ listenSshState: vi.fn(async () => () => {}), listenProject: vi.fn(async () => () => {}) }))
vi.mock('../../lib/ipc', () => ({
  sshList: vi.fn(async (_id: string, dir: string) => ({
    entries:
      dir === '/home/dev'
        ? [
            { name: 'src', path: '/home/dev/src', isDir: true, isLink: false },
            { name: 'a.ts', path: '/home/dev/a.ts', isDir: false, isLink: false },
          ]
        : [],
    truncated: false,
  })),
  sshUpload: vi.fn(async () => 't1'),
  sshWrite: vi.fn(),
  sshMkdir: vi.fn(),
  sshRename: vi.fn(),
  sshDelete: vi.fn(),
  sshDownload: vi.fn(),
  asFsError: (e: unknown) => (e && typeof e === 'object' && 'code' in e ? e : { code: 'io', message: String(e) }),
}))
vi.mock('../../lib/stores/project.svelte', () => ({ projectStore: { project: { id: 'p1', folders: ['/Users/dev/app'] } } }))
const editorMock = vi.hoisted(() => ({ openRemote: vi.fn(), repathRemote: vi.fn(), markRemoteRemoved: vi.fn() }))
vi.mock('../../lib/stores/editor.svelte', () => ({ editor: editorMock }))

const termMock = vi.hoisted(() => ({ openRemote: vi.fn() }))
vi.mock('../../lib/stores/terminals.svelte', () => ({ terminals: termMock }))

import * as ipc from '../../lib/ipc'
import RemoteTree from './RemoteTree.svelte'

afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

describe('RemoteTree', () => {
  it('opens a remote terminal in the folder, or in the parent of a file', async () => {
    render(RemoteTree, { serverId: 's1', root: '/home/dev' })
    const dir = (await screen.findByText('src')).closest('[role="treeitem"]')!
    await fireEvent.contextMenu(dir)
    await fireEvent.click(await screen.findByText('Apri terminale qui'))
    expect(termMock.openRemote).toHaveBeenLastCalledWith('s1', '/home/dev/src')
    await fireEvent.contextMenu(screen.getByText('a.ts').closest('[role="treeitem"]')!)
    await fireEvent.click(await screen.findByText('Apri terminale qui'))
    expect(termMock.openRemote).toHaveBeenLastCalledWith('s1', '/home/dev')
  })

  it('opens a file in the editor on double click, not a folder', async () => {
    render(RemoteTree, { serverId: 's1', root: '/home/dev' })
    const file = (await screen.findByText('a.ts')).closest('[role="treeitem"]')!
    await fireEvent.dblClick(file)
    expect(editorMock.openRemote).toHaveBeenCalledWith('s1', '/home/dev/a.ts')
    await fireEvent.dblClick(screen.getByText('src').closest('[role="treeitem"]')!)
    expect(editorMock.openRemote).toHaveBeenCalledTimes(1)
  })

  it('uploads local paths dropped on a remote folder', async () => {
    render(RemoteTree, { serverId: 's1', root: '/home/dev' })
    const dir = (await screen.findByText('src')).closest('[role="treeitem"]')!
    const drop = createEvent.drop(dir)
    Object.defineProperty(drop, 'dataTransfer', { value: { getData: (t: string) => (t === 'application/x-codeart-local' ? JSON.stringify(['/Users/dev/app/x.ts']) : '') } })
    await fireEvent(dir, drop)
    await waitFor(() => expect(ipc.sshUpload).toHaveBeenCalledWith('s1', ['/Users/dev/app/x.ts'], '/home/dev/src'))
  })

  const dropRemote = async (el: Element, path: string) => {
    const drop = createEvent.drop(el)
    Object.defineProperty(drop, 'dataTransfer', {
      value: { getData: (t: string) => (t === 'application/x-codeart-remote' ? JSON.stringify({ serverId: 's1', path }) : '') },
    })
    await fireEvent(el, drop)
  }

  it('moving a file onto a folder renames it on the server and repaths its open tabs', async () => {
    render(RemoteTree, { serverId: 's1', root: '/home/dev' })
    const dir = (await screen.findByText('src')).closest('[role="treeitem"]')!
    await dropRemote(dir, '/home/dev/a.ts')
    await waitFor(() => expect(ipc.sshRename).toHaveBeenCalledWith('s1', '/home/dev/a.ts', '/home/dev/src/a.ts'))
    await waitFor(() => expect(editorMock.repathRemote).toHaveBeenCalledWith('s1', '/home/dev/a.ts', '/home/dev/src/a.ts'))
  })

  it('a failed move leaves the tabs alone', async () => {
    vi.mocked(ipc.sshRename).mockRejectedValueOnce({ code: 'io', message: 'esiste già' })
    render(RemoteTree, { serverId: 's1', root: '/home/dev' })
    const dir = (await screen.findByText('src')).closest('[role="treeitem"]')!
    await dropRemote(dir, '/home/dev/a.ts')
    await waitFor(() => expect(ipc.sshRename).toHaveBeenCalled())
    await screen.findByText(/esiste già/)
    expect(editorMock.repathRemote).not.toHaveBeenCalled()
  })

  it('dropping a child of the root onto the root is not a self-rename', async () => {
    render(RemoteTree, { serverId: 's1', root: '/home/dev' })
    await screen.findByText('a.ts')
    const rootRow = screen.getAllByRole('treeitem')[0]
    await dropRemote(rootRow, '/home/dev/a.ts')
    await dropRemote(rootRow, '/home/dev')
    expect(ipc.sshRename).not.toHaveBeenCalled()
  })

  it('renaming from the menu repaths open tabs; deleting marks them removed', async () => {
    render(RemoteTree, { serverId: 's1', root: '/home/dev' })
    const file = (await screen.findByText('a.ts')).closest('[role="treeitem"]')!
    await fireEvent.contextMenu(file)
    await fireEvent.click(await screen.findByText('Rinomina'))
    const input = screen.getByLabelText('Nome') as HTMLInputElement
    await fireEvent.input(input, { target: { value: 'b.ts' } })
    await fireEvent.keyDown(input, { key: 'Enter' })
    await waitFor(() => expect(editorMock.repathRemote).toHaveBeenCalledWith('s1', '/home/dev/a.ts', '/home/dev/b.ts'))
    await fireEvent.contextMenu(screen.getByText('a.ts').closest('[role="treeitem"]')!)
    await fireEvent.click(await screen.findByText('Elimina'))
    await waitFor(() => expect(editorMock.markRemoteRemoved).toHaveBeenCalledWith('s1', '/home/dev/a.ts'))
  })
})
