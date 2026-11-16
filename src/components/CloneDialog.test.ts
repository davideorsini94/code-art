// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from '@testing-library/svelte'
import { afterEach, describe, expect, it, vi } from 'vitest'

vi.mock('../lib/stores/project.svelte', () => ({ projectStore: { project: null, untrusted: [], refreshBranch: vi.fn() } }))
vi.mock('../lib/events', () => ({ listenProject: vi.fn(async () => () => {}) }))
const ipc = vi.hoisted(() => ({
  gitRepos: vi.fn(async () => []),
  ghAccounts: vi.fn(),
  ghRepoList: vi.fn(),
  ghClone: vi.fn(),
  projectSettingGet: vi.fn(async () => null),
}))
vi.mock('../lib/ipc', () => ipc)

import CloneDialog from './CloneDialog.svelte'
import { github, GH_MISSING } from '../lib/stores/github.svelte'

const repos = [
  { nameWithOwner: 'octo-user/demo', description: 'Progetto di prova', private: true, updatedAt: '2026-11-01T10:00:00Z' },
  { nameWithOwner: 'octo-user/altro', description: '', private: false, updatedAt: '2026-10-01T10:00:00Z' },
]

afterEach(() => {
  cleanup()
  github.cloneOpen = false
  github.accounts = []
  github.missing = false
  ;(github as unknown as { accountsLoaded: boolean }).accountsLoaded = false
  vi.clearAllMocks()
})

describe('CloneDialog', () => {
  it('lists the repositories of the chosen account, filters them and clones one', async () => {
    ipc.ghAccounts.mockResolvedValue([{ login: 'octo-user', active: true }, { login: 'octo-work', active: false }])
    ipc.ghRepoList.mockResolvedValue(repos)
    ipc.ghClone.mockResolvedValue(undefined)
    github.cloneOpen = true
    render(CloneDialog)
    expect(await screen.findByText('octo-user/demo')).toBeTruthy()
    expect(ipc.ghRepoList).toHaveBeenCalledWith('', 200)
    await fireEvent.input(screen.getByLabelText('Filtra i repository'), { target: { value: 'alt' } })
    expect(screen.queryByText('octo-user/demo')).toBeNull()
    await fireEvent.change(screen.getByLabelText('Account GitHub'), { target: { value: 'octo-work' } })
    await vi.waitFor(() => expect(ipc.ghRepoList).toHaveBeenLastCalledWith('octo-work', 200))
    await fireEvent.input(screen.getByLabelText('Filtra i repository'), { target: { value: '' } })
    await fireEvent.click(await screen.findByRole('button', { name: 'Clona octo-user/demo' }))
    expect(ipc.ghClone).toHaveBeenCalledWith('octo-user/demo', 'octo-work')
    await vi.waitFor(() => expect(github.cloneOpen).toBe(false))
  })

  it('shows the install hint when gh is missing', async () => {
    ipc.ghAccounts.mockRejectedValue(GH_MISSING)
    github.cloneOpen = true
    render(CloneDialog)
    expect(await screen.findByText(GH_MISSING)).toBeTruthy()
    expect(ipc.ghRepoList).not.toHaveBeenCalled()
  })

  it('renders nothing when closed and closes with Esc', async () => {
    ipc.ghAccounts.mockResolvedValue([])
    ipc.ghRepoList.mockResolvedValue([])
    render(CloneDialog)
    expect(screen.queryByRole('dialog')).toBeNull()
    github.cloneOpen = true
    const dlg = await screen.findByRole('dialog', { name: 'Clona da GitHub' })
    await fireEvent.keyDown(dlg, { key: 'Escape' })
    expect(github.cloneOpen).toBe(false)
  })
})
