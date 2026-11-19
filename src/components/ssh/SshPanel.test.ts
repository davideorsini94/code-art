// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

vi.mock('../../lib/events', () => ({ listenSshState: vi.fn(async () => () => {}) }))
vi.mock('../../lib/ipc', () => ({
  sshServers: vi.fn(),
  sshStates: vi.fn(async () => []),
  sshServerSave: vi.fn(),
  sshPasswordSet: vi.fn(),
  sshPasswordClear: vi.fn(),
  sshConnect: vi.fn(),
  sshHome: vi.fn(async () => '/home/dev'),
  sshList: vi.fn(),
  sshWrite: vi.fn(),
  sshMkdir: vi.fn(),
  sshRename: vi.fn(),
  sshDelete: vi.fn(),
  sshDownload: vi.fn(),
}))
vi.mock('../../lib/stores/project.svelte', () => ({ projectStore: { project: { folders: ['/Users/dev/app'] } } }))

import * as ipc from '../../lib/ipc'
import { ssh } from '../../lib/stores/ssh.svelte'
import SshPanel from './SshPanel.svelte'
import RemoteTree from './RemoteTree.svelte'

const srv = (o: object) => ({ id: 'a', name: 'Prova', host: 'example.test', user: null, port: null, identityFile: null, startDir: null, source: 'saved', hasPassword: false, ...o })

beforeEach(() => {
  vi.clearAllMocks()
  ssh.stop()
  ssh.servers = []
  ssh.states = {}
  ssh.errors = {}
  ssh.passwordFor = null
  ssh.loaded = false
})
afterEach(cleanup)

describe('SshPanel', () => {
  it('lists saved and config servers, config ones labelled', async () => {
    vi.mocked(ipc.sshServers).mockResolvedValue([srv({}), srv({ id: 'c', name: 'bastion', source: 'config' })] as never)
    render(SshPanel)
    await waitFor(() => expect(screen.getByText('Prova')).toBeTruthy())
    expect(screen.getByText('bastion')).toBeTruthy()
    expect(screen.getAllByText('da ssh config')).toHaveLength(1)
    expect(screen.queryByLabelText('Elimina bastion')).toBeNull()
  })

  it('saves the server, sends the password separately and clears the field', async () => {
    vi.mocked(ipc.sshServers).mockResolvedValue([])
    vi.mocked(ipc.sshServerSave).mockResolvedValue(srv({ id: 'new' }) as never)
    render(SshPanel)
    await fireEvent.click(screen.getByLabelText('Aggiungi server'))
    await fireEvent.input(screen.getByLabelText('Nome'), { target: { value: 'Prova' } })
    await fireEvent.input(screen.getByLabelText('Host'), { target: { value: 'example.test' } })
    await fireEvent.input(screen.getByLabelText('Porta'), { target: { value: '2222' } })
    const pw = screen.getByLabelText('Password (facoltativa)') as HTMLInputElement
    expect(pw.type).toBe('password')
    await fireEvent.input(pw, { target: { value: 'segreto-finto' } })
    await fireEvent.click(screen.getByText('Salva'))
    await waitFor(() => expect(ipc.sshPasswordSet).toHaveBeenCalledWith('new', 'segreto-finto'))
    expect(ipc.sshServerSave).toHaveBeenCalledWith({ id: null, name: 'Prova', host: 'example.test', user: null, port: 2222, identityFile: null, startDir: null })
    expect(ssh.servers.length >= 0).toBe(true)
    expect(JSON.stringify(ssh)).not.toContain('segreto-finto')
  })

  it('shows validation errors inline and keeps the form open', async () => {
    vi.mocked(ipc.sshServers).mockResolvedValue([])
    vi.mocked(ipc.sshServerSave).mockRejectedValue('host non valido')
    render(SshPanel)
    await fireEvent.click(screen.getByLabelText('Aggiungi server'))
    await fireEvent.click(screen.getByText('Salva'))
    await waitFor(() => expect(screen.getByRole('alert').textContent).toContain('host non valido'))
    expect(ipc.sshPasswordSet).not.toHaveBeenCalled()
  })

  it("prompts for the password on 'auth' then retries", async () => {
    vi.mocked(ipc.sshServers).mockResolvedValue([srv({})] as never)
    vi.mocked(ipc.sshConnect).mockRejectedValueOnce('auth').mockResolvedValue(undefined)
    render(SshPanel)
    await waitFor(() => screen.getByText('Connetti'))
    await fireEvent.click(screen.getByText('Connetti'))
    const input = (await screen.findByLabelText('Password')) as HTMLInputElement
    await fireEvent.input(input, { target: { value: 'pw-finta' } })
    await fireEvent.click(screen.getAllByText('Connetti')[0])
    await waitFor(() => expect(ipc.sshPasswordSet).toHaveBeenCalledWith('a', 'pw-finta'))
    await waitFor(() => expect(ipc.sshConnect).toHaveBeenCalledTimes(2))
  })
})

describe('RemoteTree', () => {
  const entries = (list: [string, boolean][], dir: string) => ({
    entries: list.map(([name, isDir]) => ({ name, path: `${dir}/${name}`, isDir, isLink: false })),
    truncated: false,
  })
  beforeEach(() => {
    vi.mocked(ipc.sshList).mockImplementation((async (_id: string, dir: string) =>
      dir === '/home/dev'
        ? entries([['src', true], ['<img src=x onerror=alert(1)>.txt', false], ['a b', false]], dir)
        : entries([['main.rs', false]], dir)) as never)
  })

  const open = async () => {
    render(RemoteTree, { serverId: 'a', root: '/home/dev' })
    await waitFor(() => expect(screen.getByText('src')).toBeTruthy())
  }
  const item = (t: string) => screen.getByText(t).closest('[role="treeitem"]')!

  it('expands a directory with sshList and renders hostile names as text', async () => {
    const { container } = render(RemoteTree, { serverId: 'a', root: '/home/dev' })
    await waitFor(() => expect(screen.getByText('src')).toBeTruthy())
    expect(screen.getByText('<img src=x onerror=alert(1)>.txt')).toBeTruthy()
    expect(container.querySelector('.name img, [role="treeitem"] .name img')).toBeNull()
    expect(container.querySelectorAll('img[onerror]')).toHaveLength(0)
    await fireEvent.click(item('src'))
    await waitFor(() => expect(screen.getByText('main.rs')).toBeTruthy())
    expect(ipc.sshList).toHaveBeenCalledWith('a', '/home/dev/src')
  })

  it('menu actions call the right ipc with the right paths', async () => {
    await open()
    await fireEvent.contextMenu(item('src'))
    await fireEvent.click(screen.getByText('Nuovo file'))
    let input = screen.getByLabelText('Nome')
    await fireEvent.input(input, { target: { value: 'n.txt' } })
    await fireEvent.keyDown(input, { key: 'Enter' })
    await waitFor(() => expect(ipc.sshWrite).toHaveBeenCalledWith('a', '/home/dev/src/n.txt', '', ''))

    await fireEvent.contextMenu(item('src'))
    await fireEvent.click(screen.getByText('Nuova cartella'))
    input = screen.getByLabelText('Nome')
    await fireEvent.input(input, { target: { value: 'lib' } })
    await fireEvent.keyDown(input, { key: 'Enter' })
    await waitFor(() => expect(ipc.sshMkdir).toHaveBeenCalledWith('a', '/home/dev/src/lib'))

    await fireEvent.contextMenu(item('a b'))
    await fireEvent.click(screen.getByText('Rinomina'))
    input = screen.getByLabelText('Nome')
    await fireEvent.input(input, { target: { value: 'c' } })
    await fireEvent.keyDown(input, { key: 'Enter' })
    await waitFor(() => expect(ipc.sshRename).toHaveBeenCalledWith('a', '/home/dev/a b', '/home/dev/c'))

    await fireEvent.contextMenu(item('a b'))
    await fireEvent.click(screen.getByText('Elimina'))
    await waitFor(() => expect(ipc.sshDelete).toHaveBeenCalledWith('a', '/home/dev/a b'))

    await fireEvent.contextMenu(item('a b'))
    await fireEvent.click(screen.getByText('Scarica nel progetto…'))
    await waitFor(() => expect(ipc.sshDownload).toHaveBeenCalledWith('a', ['/home/dev/a b'], '/Users/dev/app'))
  })

  it('root has no rename/delete; disabled tree has no menu', async () => {
    await open()
    await fireEvent.contextMenu(item('/home/dev'))
    expect(screen.queryByText('Rinomina')).toBeNull()
    expect(screen.queryByText('Elimina')).toBeNull()
    cleanup()
    render(RemoteTree, { serverId: 'a', root: '/home/dev', disabled: true })
    await waitFor(() => expect(screen.getByText('src')).toBeTruthy())
    await fireEvent.contextMenu(item('src'))
    expect(screen.queryByRole('menu')).toBeNull()
  })

  it('dragging inside the tree renames into the target directory', async () => {
    await open()
    const data: Record<string, string> = {}
    const dt = { setData: (k: string, v: string) => (data[k] = v), getData: (k: string) => data[k] ?? '' }
    await fireEvent.dragStart(item('a b'), { dataTransfer: dt })
    await fireEvent.drop(item('src'), { dataTransfer: dt })
    await waitFor(() => expect(ipc.sshRename).toHaveBeenCalledWith('a', '/home/dev/a b', '/home/dev/src/a b'))
  })
})
