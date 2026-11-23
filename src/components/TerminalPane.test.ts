// @vitest-environment jsdom
import { cleanup, createEvent, fireEvent, render, screen, waitFor } from '@testing-library/svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const ctl = vi.hoisted(() => ({ failTerm: false, writes: [] as unknown[] }))

vi.mock('@xterm/xterm', () => ({
  Terminal: class {
    constructor() {
      if (ctl.failTerm) {
        ctl.failTerm = false
        throw new Error('boom')
      }
    }
    cols = 80
    rows = 24
    loadAddon() {}
    attachCustomKeyEventHandler() {}
    open() {}
    write(d: unknown) {
      ctl.writes.push(d)
    }
    focus() {}
    dispose() {}
    onData() {}
    onResize() {}
  },
}))
vi.mock('@xterm/addon-fit', () => ({ FitAddon: class { fit() {} } }))
vi.mock('@xterm/addon-web-links', () => ({ WebLinksAddon: class {} }))
vi.mock('@xterm/xterm/css/xterm.css', () => ({}))
vi.mock('../lib/events', () => ({ listenProject: vi.fn(async () => () => {}) }))
vi.mock('../lib/external', () => ({ openExternal: vi.fn() }))
vi.mock('../lib/ipc', () => ({
  ptySpawn: vi.fn(async () => ({ id: 1, shell: 'zsh' })),
  ptySpawnSsh: vi.fn(async () => ({ id: 2, shell: 'ssh' })),
  ptyWrite: vi.fn(async () => {}),
  ptyResize: vi.fn(async () => {}),
  ptyKill: vi.fn(async () => {}),
  ptyPickCwd: vi.fn(async () => null),
  projectBranch: vi.fn(async () => ({ repo: '/Users/dev/app', name: 'main' })),
  gitRepos: vi.fn(async () => []),
  gitStatus: vi.fn(async () => ({ root: '/Users/dev/app', branch: 'main', upstream: null, ahead: 0, behind: 0, detached: false, merging: false, entries: [], truncated: false })),
}))

import TerminalPane from './TerminalPane.svelte'
import * as ipc from '../lib/ipc'
import { projectStore } from '../lib/stores/project.svelte'
import { terminals } from '../lib/stores/terminals.svelte'
import { git } from '../lib/stores/git.svelte'
import { ssh } from '../lib/stores/ssh.svelte'

beforeEach(() => {
  vi.stubGlobal('ResizeObserver', class { observe() {} disconnect() {} unobserve() {} })
  vi.mocked(ipc.ptySpawn).mockClear()
  vi.mocked(ipc.ptySpawnSsh).mockClear()
  vi.mocked(ipc.projectBranch).mockClear()
  projectStore.project = { id: 'p1', name: 'demo', folders: ['/Users/dev/app'] } as never
  terminals.requests = []
  terminals.visible = false
  git.status = {}
})
afterEach(() => {
  cleanup()
  projectStore.project = null
})

const tabs = () => screen.queryAllByRole('tab')

describe('TerminalPane', () => {
  it('opens a remote tab through ptySpawnSsh with the server name in the title', async () => {
    ssh.servers = [{ id: 's1', name: 'Dev', host: 'example.test', source: 'saved', hasPassword: false }] as never
    terminals.openRemote('s1', '/home/dev/app')
    render(TerminalPane, { visible: true })
    await waitFor(() => expect(ipc.ptySpawnSsh).toHaveBeenCalledTimes(1))
    expect(vi.mocked(ipc.ptySpawnSsh).mock.calls[0].slice(0, 2)).toEqual(['s1', '/home/dev/app'])
    expect(ipc.ptySpawn).not.toHaveBeenCalled()
    await waitFor(() => expect(tabs()).toHaveLength(1))
    expect(tabs()[0].textContent).toContain('Dev:app')
    expect(ipc.projectBranch).not.toHaveBeenCalled()
  })

  it('opens a default tab in the primary folder when first shown', async () => {
    render(TerminalPane, { visible: true })
    await waitFor(() => expect(ipc.ptySpawn).toHaveBeenCalledTimes(1))
    expect(vi.mocked(ipc.ptySpawn).mock.calls[0][0]).toBe('/Users/dev/app')
    await waitFor(() => expect(tabs()[0].textContent).toContain('app · main'))
  })

  it('opens one tab per store request, without an extra default tab', async () => {
    terminals.open('/Users/dev/app/api')
    terminals.open('/Users/dev/app/web')
    render(TerminalPane, { visible: true })
    await waitFor(() => expect(ipc.ptySpawn).toHaveBeenCalledTimes(2))
    expect(vi.mocked(ipc.ptySpawn).mock.calls.map((c) => c[0])).toEqual(['/Users/dev/app/api', '/Users/dev/app/web'])
    expect(terminals.requests).toHaveLength(0)
    await waitFor(() => expect(tabs()).toHaveLength(2))
  })

  it('renames on double click: Enter commits, Escape cancels, empty restores the automatic title', async () => {
    render(TerminalPane, { visible: true })
    await waitFor(() => expect(tabs()[0].textContent).toContain('app · main'))
    await fireEvent.dblClick(tabs()[0])
    let input = screen.getByLabelText('Nome del terminale') as HTMLInputElement
    await fireEvent.input(input, { target: { value: 'server' } })
    await fireEvent.keyDown(input, { key: 'Enter' })
    await waitFor(() => expect(tabs()[0].textContent).toContain('server'))

    await fireEvent.dblClick(tabs()[0])
    input = screen.getByLabelText('Nome del terminale') as HTMLInputElement
    await fireEvent.input(input, { target: { value: 'altro' } })
    await fireEvent.keyDown(input, { key: 'Escape' })
    await waitFor(() => expect(tabs()[0].textContent).toContain('server'))

    await fireEvent.dblClick(tabs()[0])
    input = screen.getByLabelText('Nome del terminale') as HTMLInputElement
    await fireEvent.input(input, { target: { value: '  ' } })
    await fireEvent.keyDown(input, { key: 'Enter' })
    await waitFor(() => expect(tabs()[0].textContent).toContain('app · main'))
  })

  it('the automatic title follows the branch in git.status; a renamed title stays', async () => {
    const st = (branch: string | null) => ({ branch, detached: branch === null, ahead: 0, behind: 0, entries: [] }) as never
    git.status = { '/Users/dev/app': st('main') }
    render(TerminalPane, { visible: true })
    await waitFor(() => expect(tabs()[0].textContent).toContain('app · main'))
    git.status = { '/Users/dev/app': st('dev') }
    await waitFor(() => expect(tabs()[0].textContent).toContain('app · dev'))
    await fireEvent.dblClick(tabs()[0])
    const input = screen.getByLabelText('Nome del terminale') as HTMLInputElement
    await fireEvent.input(input, { target: { value: 'server' } })
    await fireEvent.keyDown(input, { key: 'Enter' })
    git.status = { '/Users/dev/app': st('feat') }
    await waitFor(() => expect(tabs()[0].textContent).toContain('server'))
    expect(tabs()[0].textContent).not.toContain('feat')
  })

  it('a detached HEAD shows only the folder name', async () => {
    git.status = { '/Users/dev/app': { branch: null, detached: true, ahead: 0, behind: 0, entries: [] } as never }
    render(TerminalPane, { visible: true })
    await waitFor(() => expect(tabs()).toHaveLength(1))
    expect(tabs()[0].textContent).toContain('app')
    expect(tabs()[0].textContent).not.toContain('main')
    expect(tabs()[0].textContent).not.toContain('·')
  })

  it('the plus button opens straight into the primary folder when it is the only choice', async () => {
    render(TerminalPane, { visible: true })
    await waitFor(() => expect(tabs()).toHaveLength(1))
    await fireEvent.click(screen.getByLabelText('Nuovo terminale'))
    await waitFor(() => expect(tabs()).toHaveLength(2))
    expect(screen.queryByRole('menu')).toBeNull()
  })

  it('reorders tabs by drag and drop', async () => {
    terminals.open('/Users/dev/app/api')
    terminals.open('/Users/dev/app/web')
    render(TerminalPane, { visible: true })
    await waitFor(() => expect(tabs()).toHaveLength(2))
    const [a, b] = tabs()
    const rect = { left: 0, width: 100, right: 100, top: 0, bottom: 20, height: 20, x: 0, y: 0, toJSON() {} }
    b.getBoundingClientRect = () => rect as DOMRect
    await fireEvent.dragStart(a)
    const over = createEvent.dragOver(b)
    Object.defineProperty(over, 'clientX', { value: 90 }) // jsdom non conosce DragEvent
    await fireEvent(b, over)
    await fireEvent.drop(b)
    await waitFor(() => expect(tabs().map((t) => t.textContent)).toEqual([expect.stringContaining('web'), expect.stringContaining('api')]))
  })

  it('keeps serving later requests after one tab fails to open', async () => {
    ctl.failTerm = true
    terminals.open('/Users/dev/app/api')
    render(TerminalPane, { visible: true })
    await waitFor(() => expect(screen.getByRole('alert')).toBeTruthy())
    terminals.open('/Users/dev/app/web')
    await waitFor(() => expect(tabs()).toHaveLength(1))
    expect(vi.mocked(ipc.ptySpawn).mock.calls.map((c) => c[0])).toEqual(['/Users/dev/app/web'])
  })

  it('kills the PTY when the tab is closed while the shell is still starting', async () => {
    let resolve!: (v: { id: number; shell: string }) => void
    vi.mocked(ipc.ptySpawn).mockImplementationOnce(() => new Promise((r) => (resolve = r)))
    vi.mocked(ipc.ptyKill).mockClear()
    render(TerminalPane, { visible: true })
    await waitFor(() => expect(tabs()).toHaveLength(1))
    await waitFor(() => expect(ipc.ptySpawn).toHaveBeenCalled())
    ctl.writes.length = 0
    await fireEvent.click(screen.getByLabelText(/^Chiudi/))
    await waitFor(() => expect(tabs()).toHaveLength(0))
    resolve({ id: 42, shell: 'zsh' })
    await waitFor(() => expect(ipc.ptyKill).toHaveBeenCalledWith(42))
    expect(ctl.writes).toEqual([])
  })

  it('closing the last terminal hides the panel instead of opening a new one', async () => {
    terminals.visible = true
    render(TerminalPane, { visible: true })
    await waitFor(() => expect(tabs()).toHaveLength(1))
    await waitFor(() => expect(ipc.ptySpawn).toHaveBeenCalledTimes(1))
    await fireEvent.click(screen.getByLabelText(/^Chiudi/))
    await waitFor(() => expect(tabs()).toHaveLength(0))
    expect(terminals.visible).toBe(false)
    expect(ipc.ptySpawn).toHaveBeenCalledTimes(1)
  })

  it('closing one of two terminals keeps the panel open on the other', async () => {
    terminals.open('/Users/dev/app/api')
    terminals.open('/Users/dev/app/web')
    render(TerminalPane, { visible: true })
    await waitFor(() => expect(tabs()).toHaveLength(2))
    await fireEvent.click(screen.getAllByLabelText(/^Chiudi/)[0])
    await waitFor(() => expect(tabs()).toHaveLength(1))
    expect(terminals.visible).toBe(true)
  })
})
