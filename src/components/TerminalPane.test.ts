// @vitest-environment jsdom
import { cleanup, createEvent, fireEvent, render, screen, waitFor } from '@testing-library/svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

vi.mock('@xterm/xterm', () => ({
  Terminal: class {
    cols = 80
    rows = 24
    loadAddon() {}
    attachCustomKeyEventHandler() {}
    open() {}
    write() {}
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

beforeEach(() => {
  vi.stubGlobal('ResizeObserver', class { observe() {} disconnect() {} unobserve() {} })
  vi.mocked(ipc.ptySpawn).mockClear()
  projectStore.project = { id: 'p1', name: 'demo', folders: ['/Users/dev/app'] } as never
  terminals.requests = []
  terminals.visible = false
})
afterEach(() => {
  cleanup()
  projectStore.project = null
})

const tabs = () => screen.queryAllByRole('tab')

describe('TerminalPane', () => {
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
})
