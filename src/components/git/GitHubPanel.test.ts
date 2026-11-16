// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from '@testing-library/svelte'
import { afterEach, describe, expect, it, vi } from 'vitest'

const project = vi.hoisted(() => ({ project: { id: 'p1', name: 'app', folders: ['/Users/dev/app'] }, untrusted: [] as string[], refreshBranch: vi.fn() }))
vi.mock('../../lib/stores/project.svelte', () => ({ projectStore: project }))
vi.mock('../../lib/events', () => ({ listenProject: vi.fn(async () => () => {}) }))
const ipc = vi.hoisted(() => ({
  gitRepos: vi.fn(async () => []),
  gitStatus: vi.fn(),
  gitLog: vi.fn(async () => [{ hash: 'h', short: 'h', subject: 'Aggiunge il login', author: 'octo-user', dateIso: '' }]),
  ghAccounts: vi.fn(),
  ghRemote: vi.fn(),
  ghPrList: vi.fn(),
  ghPrChecks: vi.fn(),
  ghPrCreate: vi.fn(),
  ghPrMerge: vi.fn(),
  ghRepoCreate: vi.fn(),
  projectSettingGet: vi.fn(),
  projectSettingSet: vi.fn(),
}))
vi.mock('../../lib/ipc', () => ipc)
const external = vi.hoisted(() => ({ openExternal: vi.fn(() => true as const), isSafeLink: () => true }))
vi.mock('../../lib/external', () => external)

import GitHubPanel from './GitHubPanel.svelte'
import { github, GH_MISSING } from '../../lib/stores/github.svelte'
import { git } from '../../lib/stores/git.svelte'

const REPO = '/Users/dev/app'
const pr = (n: number, over = {}) => ({
  number: n, title: `PR ${n}`, author: 'octo-user', head: `feature/${n}`, base: 'main', state: 'open', isDraft: false,
  url: `https://github.com/octo-org/app/pull/${n}`, checks: { passing: 1, failing: 0, pending: 0 }, ...over,
})

function setup({ remote = 'octo-org/app' as string | null, prs = [pr(7), pr(8, { checks: { passing: 1, failing: 1, pending: 0 } }), pr(9, { checks: { passing: 0, failing: 0, pending: 2 } })] } = {}) {
  ipc.ghAccounts.mockResolvedValue([{ login: 'octo-user', active: true }, { login: 'octo-work', active: false }])
  ipc.projectSettingGet.mockResolvedValue(null)
  ipc.projectSettingSet.mockResolvedValue(undefined)
  ipc.ghRemote.mockResolvedValue(remote)
  ipc.ghPrList.mockResolvedValue(prs)
  render(GitHubPanel, { repo: REPO })
}

afterEach(() => {
  cleanup()
  github.accounts = []
  github.account = ''
  github.missing = false
  github.remotes = {}
  github.prs = {}
  github.checks = {}
  github.prState = 'open'
  github.creating = null
  github.busy = null
  ;(github as unknown as { accountsLoaded: boolean }).accountsLoaded = false
  git.error = null
  git.notice = null
  vi.clearAllMocks()
})

describe('GitHubPanel', () => {
  it('lists pull requests with a coloured check dot and opens links in the system browser', async () => {
    setup()
    await screen.findByText('PR 7')
    expect(ipc.ghPrList).toHaveBeenCalledWith(REPO, 'open')
    const dot = (n: number) => screen.getByRole('button', { name: `Controlli della pull request #${n}` })
    expect(dot(7).classList.contains('ok')).toBe(true)
    expect(dot(8).classList.contains('fail')).toBe(true)
    expect(dot(9).classList.contains('wait')).toBe(true)
    await fireEvent.click(screen.getByRole('button', { name: 'PR 7' }))
    expect(external.openExternal).toHaveBeenCalledWith('https://github.com/octo-org/app/pull/7')
  })

  it('shows the checks of a pull request on demand', async () => {
    ipc.ghPrChecks.mockResolvedValue([{ name: 'build', state: 'failing', url: 'https://github.com/octo-org/app/actions/runs/1' }])
    setup()
    await fireEvent.click(await screen.findByRole('button', { name: 'Controlli della pull request #8' }))
    expect(ipc.ghPrChecks).toHaveBeenCalledWith(REPO, 8)
    await fireEvent.click(await screen.findByRole('button', { name: 'build' }))
    expect(external.openExternal).toHaveBeenCalledWith('https://github.com/octo-org/app/actions/runs/1')
  })

  it('switching account saves the project setting and reloads', async () => {
    setup()
    await screen.findByText('PR 7')
    const select = screen.getByLabelText('Account GitHub') as HTMLSelectElement
    expect([...select.options].map((o) => o.textContent?.trim())).toEqual(['Account attivo di gh (octo-user)', 'octo-user', 'octo-work'])
    await fireEvent.change(select, { target: { value: 'octo-work' } })
    await vi.waitFor(() => expect(ipc.projectSettingSet).toHaveBeenCalledWith('github_account', 'octo-work'))
    await vi.waitFor(() => expect(ipc.ghPrList).toHaveBeenCalledTimes(2))
  })

  it('merges with the chosen method (the core asks for confirmation)', async () => {
    ipc.ghPrMerge.mockResolvedValue(true)
    setup()
    await screen.findByText('PR 7')
    await fireEvent.click(screen.getAllByRole('button', { name: 'Unisci…' })[0])
    await fireEvent.change(screen.getByLabelText('Metodo di unione'), { target: { value: 'squash' } })
    await fireEvent.click(screen.getByLabelText('Elimina il branch dopo l’unione'))
    await fireEvent.click(screen.getByRole('button', { name: 'Unisci #7' }))
    expect(ipc.ghPrMerge).toHaveBeenCalledWith(REPO, 7, 'squash', true)
    await vi.waitFor(() => expect(git.notice).toBe('Pull request #7 unita'))
  })

  it('without a GitHub remote offers "Pubblica su GitHub", private by default, named after the folder', async () => {
    ipc.ghRepoCreate.mockResolvedValue('https://github.com/octo-user/app')
    setup({ remote: null })
    await fireEvent.click(await screen.findByRole('button', { name: 'Pubblica su GitHub' }))
    expect((screen.getByLabelText('Nome del repository') as HTMLInputElement).value).toBe('app')
    expect((screen.getByLabelText('Privato') as HTMLInputElement).checked).toBe(true)
    await fireEvent.click(screen.getByRole('button', { name: 'Pubblica' }))
    expect(ipc.ghRepoCreate).toHaveBeenCalledWith(REPO, 'app', true, '')
    await vi.waitFor(() => expect(git.notice).toBe('Pubblicato su GitHub: https://github.com/octo-user/app'))
    expect(ipc.ghPrList).not.toHaveBeenCalled()
  })

  it('tells how to install gh when it is missing', async () => {
    ipc.ghAccounts.mockRejectedValue(GH_MISSING)
    ipc.ghRemote.mockResolvedValue('octo-org/app')
    ipc.ghPrList.mockRejectedValue(GH_MISSING)
    render(GitHubPanel, { repo: REPO })
    expect(await screen.findByText(GH_MISSING)).toBeTruthy()
    expect(git.error).toBeNull()
  })

  it('"Nuova pull request" opens the form with the title of the last commit', async () => {
    setup()
    await screen.findByText('PR 7')
    await fireEvent.click(screen.getByRole('button', { name: 'Nuova pull request' }))
    expect(await screen.findByDisplayValue('Aggiunge il login')).toBeTruthy()
    expect(ipc.gitLog).toHaveBeenCalledWith(REPO, 1)
  })
})
