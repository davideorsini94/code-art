// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, within } from '@testing-library/svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const project = vi.hoisted(() => ({ project: { id: 'p1', name: 'app', folders: ['/Users/dev/app'] }, untrusted: [] as string[], acceptTrust: vi.fn() }))
vi.mock('../../lib/stores/project.svelte', () => ({ projectStore: project }))
vi.mock('../../lib/events', () => ({ listenProject: vi.fn(async () => () => {}) }))
const ipc = vi.hoisted(() => ({ gitRepos: vi.fn(), gitStatus: vi.fn(), gitStage: vi.fn(), gitUnstage: vi.fn(), gitDiscard: vi.fn(), gitDiff: vi.fn(), gitInit: vi.fn(), gitRemoteOp: vi.fn(), gitCancel: vi.fn(), gitMergeAbort: vi.fn(), gitBranches: vi.fn(), fsRead: vi.fn(), ghAccounts: vi.fn(async () => []), ghRemote: vi.fn(async () => null), ghPrList: vi.fn(async () => []), projectSettingGet: vi.fn(async () => null) }))
vi.mock('../../lib/ipc', () => ipc)
const editor = vi.hoisted(() => ({ open: vi.fn(), openDiff: vi.fn(), tabs: [] as { key: string; dirty: boolean }[] }))
vi.mock('../../lib/stores/editor.svelte', () => ({ editor }))

import GitPanel from './GitPanel.svelte'
import { git } from '../../lib/stores/git.svelte'
import { resetTooltipForTests } from '../../lib/tooltip'


const REPO = '/Users/dev/app'
const e = (path: string, over = {}) => ({ path, origPath: null, staged: null, unstaged: null, conflict: false, untracked: false, ...over })
const status = (over = {}) => ({
  root: REPO, branch: 'main', upstream: 'origin/main', ahead: 2, behind: 1, detached: false, merging: false, truncated: false,
  entries: [e('src/a.ts', { staged: 'M' }), e('src/b.ts', { unstaged: 'M' }), e('new.txt', { untracked: true })],
  ...over,
})

async function mount(st = status(), repos = [REPO]) {
  ipc.gitRepos.mockResolvedValue(repos)
  ipc.gitStatus.mockResolvedValue(st)
  render(GitPanel)
  if (repos.length) await screen.findByText('Modifiche')
}

/** Testo del tooltip di un elemento (passa il ritardo di comparsa con timer finti). */
function tipText(el: Element) {
  vi.useFakeTimers()
  el.dispatchEvent(new PointerEvent('pointerover', { bubbles: true }))
  vi.advanceTimersByTime(600)
  const text = document.getElementById('codeart-tooltip')?.textContent ?? null
  el.dispatchEvent(new PointerEvent('pointerout', { bubbles: true }))
  vi.useRealTimers()
  resetTooltipForTests()
  return text
}

beforeEach(() => ipc.gitCancel.mockResolvedValue(undefined))

afterEach(() => {
  cleanup()
  git.unbind()
  git.repos = []
  git.status = {}
  git.loaded = false
  git.error = null
  git.busy = null
  project.untrusted = []
  vi.clearAllMocks()
})

describe('GitPanel', () => {
  it('each repository has a GitHub section; without a GitHub remote it offers to publish', async () => {
    await mount()
    const gh = screen.getByRole('region', { name: 'GitHub' })
    expect(await within(gh).findByRole('button', { name: 'Pubblica su GitHub' })).toBeTruthy()
    expect(ipc.ghRemote).toHaveBeenCalledWith(REPO)
  })

  it('groups changes into staged and unstaged, and shows branch, ahead and behind', async () => {
    await mount()
    const staged = screen.getByRole('region', { name: 'Modifiche in stage' })
    const changes = screen.getByRole('region', { name: 'Modifiche' })
    expect(staged.textContent).toContain('a.ts')
    expect(changes.textContent).toContain('b.ts')
    expect(changes.textContent).toContain('new.txt')
    expect(screen.getByText(/↑2 ↓1/)).toBeTruthy()
    expect(tipText(screen.getByRole('button', { name: /main/ }))).toBe('Cambia branch')
  })

  it('row actions call the right commands; discard exists only in "Modifiche"', async () => {
    await mount()
    const staged = screen.getByRole('region', { name: 'Modifiche in stage' })
    const changes = screen.getByRole('region', { name: 'Modifiche' })
    expect(staged.querySelector('[aria-label^="Scarta"]')).toBeNull()
    await fireEvent.click(screen.getByLabelText('Togli a.ts dallo stage'))
    expect(ipc.gitUnstage).toHaveBeenCalledWith(REPO, ['src/a.ts'])
    await vi.waitFor(() => expect(git.busy).toBeNull())
    await fireEvent.click(screen.getByLabelText('Aggiungi b.ts allo stage'))
    expect(ipc.gitStage).toHaveBeenCalledWith(REPO, ['src/b.ts'])
    await vi.waitFor(() => expect(git.busy).toBeNull())
    await fireEvent.click(screen.getByLabelText('Scarta le modifiche a new.txt'))
    expect(ipc.gitDiscard).toHaveBeenCalledWith(REPO, ['new.txt'])
    await vi.waitFor(() => expect(git.busy).toBeNull())
    await fireEvent.click(screen.getByLabelText('Aggiungi tutto allo stage'))
    expect(ipc.gitStage).toHaveBeenLastCalledWith(REPO, ['src/b.ts', 'new.txt'])
    await vi.waitFor(() => expect(git.busy).toBeNull())
    await fireEvent.click(screen.getByLabelText('Apri il file b.ts'))
    expect(editor.open).toHaveBeenCalledWith('/Users/dev/app/src/b.ts')
    expect(changes).toBeTruthy()
  })

  it('clicking a row opens its diff in the editor', async () => {
    ipc.gitDiff.mockResolvedValue({ before: 'x', after: 'y', binary: false })
    await mount()
    await fireEvent.click(screen.getByText('b.ts'))
    await vi.waitFor(() => expect(editor.openDiff).toHaveBeenCalledWith('/Users/dev/app/src/b.ts', 'x', 'y', { editable: true }))
    expect(ipc.gitDiff).toHaveBeenCalledWith(REPO, 'src/b.ts', null, false)
  })

  it('shows the core error text (e.g. a refused path)', async () => {
    ipc.gitStage.mockRejectedValue('percorso non valido')
    await mount()
    await fireEvent.click(screen.getByLabelText('Aggiungi b.ts allo stage'))
    expect((await screen.findByRole('alert')).textContent).toContain('percorso non valido')
  })

  it('Fetch/Pull/Push start the remote op; progress shows with Annulla, which cancels', async () => {
    ipc.gitRemoteOp.mockResolvedValue(4)
    await mount()
    await fireEvent.click(screen.getByRole('button', { name: 'Push' }))
    expect(ipc.gitRemoteOp).toHaveBeenCalledWith(REPO, 'push')
    await vi.waitFor(() => expect(git.busy?.id).toBe(4))
    git.busy = { ...git.busy!, line: 'Writing objects: 10%' }
    expect((await screen.findByRole('status')).textContent).toContain('Writing objects: 10%')
    expect((screen.getByRole('button', { name: 'Fetch' }) as HTMLButtonElement).disabled).toBe(true)
    await fireEvent.click(screen.getByRole('button', { name: 'Annulla' }))
    expect(ipc.gitCancel).toHaveBeenCalledWith(4)
  })

  it('merge in progress: banner with the conflict count and "Annulla unione"', async () => {
    await mount(status({ merging: true, entries: [e('a', { conflict: true, staged: 'U', unstaged: 'U' }), e('b', { conflict: true })] }))
    expect(screen.getByText('Unione in corso: 2 conflitti')).toBeTruthy()
    await fireEvent.click(screen.getByRole('button', { name: 'Annulla unione' }))
    expect(ipc.gitMergeAbort).toHaveBeenCalledWith(REPO)
  })

  it('not a repository: message and "Inizializza"', async () => {
    ipc.gitRepos.mockResolvedValue([])
    render(GitPanel)
    expect(await screen.findByText(/non è un repository/)).toBeTruthy()
    await fireEvent.click(screen.getByRole('button', { name: 'Inizializza' }))
    expect(ipc.gitInit).toHaveBeenCalledWith(REPO)
  })

  it('staged diffs open read-only', async () => {
    ipc.gitDiff.mockResolvedValue({ before: 'x', after: 'y', binary: false })
    await mount()
    await fireEvent.click(screen.getByText('a.ts'))
    await vi.waitFor(() => expect(editor.openDiff).toHaveBeenCalledWith('/Users/dev/app/src/a.ts', 'x', 'y', { editable: false }))
  })

  it('multi-folder project: a non-repo folder gets its own "Inizializza" next to the repositories', async () => {
    project.project = { id: 'p1', name: 'app', folders: [REPO, '/Users/dev/docs'] }
    await mount()
    await fireEvent.click(await screen.findByRole('button', { name: 'Inizializza docs' }))
    expect(ipc.gitInit).toHaveBeenCalledWith('/Users/dev/docs')
    project.project = { id: 'p1', name: 'app', folders: [REPO] }
  })

  it('shows the outcome notice in the panel', async () => {
    await mount()
    git.notice = 'Fetch completato'
    expect((await screen.findByText('Fetch completato')).textContent).toBeTruthy()
    git.notice = null
  })

  it('untrusted project: message and "Mi fido", no git calls', async () => {
    project.untrusted = [REPO]
    render(GitPanel)
    await fireEvent.click(screen.getByRole('button', { name: 'Mi fido' }))
    expect(project.acceptTrust).toHaveBeenCalledWith(true)
    expect(ipc.gitRepos).not.toHaveBeenCalled()
  })

  it('with several repositories each section is titled by its folder', async () => {
    const B = '/Users/dev/lib'
    ipc.gitRepos.mockResolvedValue([REPO, B])
    ipc.gitStatus.mockImplementation(async (r: string) => status({ root: r }))
    render(GitPanel)
    expect(await screen.findByRole('region', { name: 'lib' })).toBeTruthy()
    expect(screen.getByRole('region', { name: 'app' })).toBeTruthy()
  })
})

describe('GitPanel conflicts', () => {
  const conf = () => status({ merging: true, entries: [e('a.ts', { conflict: true, staged: 'U', unstaged: 'U' })] })

  const mark = () => screen.queryByRole('button', { name: 'Segna a.ts come risolto' })
  const settle = () => new Promise((r) => setTimeout(r, 0))
  const MARKED = 'a\n<<<<<<< HEAD\nx\n=======\ny\n>>>>>>> f\n'

  afterEach(() => { editor.tabs = [] })

  it('no "Segna come risolto" while the file still has markers', async () => {
    ipc.fsRead.mockResolvedValue({ text: MARKED, mtimeMs: 1 })
    await mount(conf())
    await settle()
    expect(mark()).toBeNull()
  })

  it('no "Segna come risolto" while the resolved file has unsaved edits', async () => {
    ipc.fsRead.mockResolvedValue({ text: 'risolto\n', mtimeMs: 2 })
    editor.tabs = [{ key: `${REPO}/a.ts`, dirty: true }]
    await mount(conf())
    await settle()
    expect(mark()).toBeNull()
  })

  it('"Segna come risolto" stages the file once saved and without markers', async () => {
    ipc.gitStage.mockResolvedValue(undefined)
    ipc.fsRead.mockResolvedValue({ text: 'risolto\n', mtimeMs: 2 })
    await mount(conf())
    await fireEvent.click(await screen.findByRole('button', { name: 'Segna a.ts come risolto' }))
    expect(ipc.gitStage).toHaveBeenCalledWith(REPO, ['a.ts'])
  })
})
