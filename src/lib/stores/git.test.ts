// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const handlers = vi.hoisted(() => new Map<string, (e: { payload: unknown }) => void>())
vi.mock('../events', () => ({
  listenProject: vi.fn(async (name: string, _id: string, cb: (e: { payload: unknown }) => void) => {
    handlers.set(name, cb)
    return () => handlers.delete(name)
  }),
}))
const ipc = vi.hoisted(() => ({
  gitRepos: vi.fn(),
  gitStatus: vi.fn(),
  gitStage: vi.fn(),
  gitUnstage: vi.fn(),
  gitDiscard: vi.fn(),
  gitCommit: vi.fn(),
  gitCheckout: vi.fn(),
  gitCreateBranch: vi.fn(),
  gitMerge: vi.fn(),
  gitMergeAbort: vi.fn(),
  gitInit: vi.fn(),
  gitRemoteOp: vi.fn(),
  gitCancel: vi.fn(),
  gitDiff: vi.fn(),
}))
vi.mock('../ipc', () => ipc)
const project = vi.hoisted(() => ({ project: { id: 'p1', name: 'app', folders: ['/Users/dev/app'] }, untrusted: [] as string[], refreshBranch: vi.fn() }))
vi.mock('./project.svelte', () => ({ projectStore: project }))
const openDiff = vi.fn()
vi.mock('./editor.svelte', () => ({ editor: { openDiff: (...a: unknown[]) => openDiff(...a) } }))

import { branchNameError, git, stagedEntries, unstagedEntries } from './git.svelte'

const REPO = '/Users/dev/app'
const status = (over = {}) => ({ root: REPO, branch: 'main', upstream: null, ahead: 0, behind: 0, detached: false, merging: false, entries: [], truncated: false, ...over })
const emit = (name: string, payload: unknown) => handlers.get(name)!({ payload })

beforeEach(async () => {
  ipc.gitCancel.mockResolvedValue(undefined)
  ipc.gitRepos.mockResolvedValue([REPO])
  ipc.gitStatus.mockResolvedValue(status())
  await git.bind('p1')
})
afterEach(() => {
  git.unbind()
  git.error = null
  git.busy = null
  git.repos = []
  git.status = {}
  project.untrusted = []
  vi.clearAllMocks()
  handlers.clear()
})

describe('git store', () => {
  it('loads repos and status, and refreshes on git:changed', async () => {
    expect(git.repos).toEqual([REPO])
    expect(git.status[REPO].branch).toBe('main')
    ipc.gitStatus.mockResolvedValue(status({ branch: 'dev' }))
    emit('git:changed', { repo: REPO })
    await vi.waitFor(() => expect(git.status[REPO].branch).toBe('dev'))
  })

  it('does not run git in an untrusted project', async () => {
    project.untrusted = [REPO]
    ipc.gitRepos.mockClear()
    await git.refresh()
    expect(ipc.gitRepos).not.toHaveBeenCalled()
  })

  it('splits entries into staged and unstaged groups (conflicts stay in changes)', () => {
    const s = status({
      entries: [
        { path: 'a', origPath: null, staged: 'M', unstaged: null, conflict: false, untracked: false },
        { path: 'b', origPath: null, staged: 'A', unstaged: 'M', conflict: false, untracked: false },
        { path: 'c', origPath: null, staged: null, unstaged: null, conflict: false, untracked: true },
        { path: 'd', origPath: null, staged: 'U', unstaged: 'U', conflict: true, untracked: false },
      ],
    })
    expect(stagedEntries(s).map((e) => e.path)).toEqual(['a', 'b'])
    expect(unstagedEntries(s).map((e) => e.path)).toEqual(['b', 'c', 'd'])
  })

  it('remote op: busy with the last progress line, cleared on op_done; progress that beats the id is kept', async () => {
    let resolve!: (id: number) => void
    ipc.gitRemoteOp.mockReturnValue(new Promise<number>((r) => (resolve = r)))
    const p = git.remote(REPO, 'push')
    expect(git.busy).toMatchObject({ op: 'push', id: null })
    emit('git:progress', { id: 7, line: 'Writing objects: 50%' })
    resolve(7)
    await p
    expect(git.busy).toMatchObject({ id: 7, line: 'Writing objects: 50%' })
    emit('git:progress', { id: 99, line: 'altra operazione' })
    expect(git.busy?.line).toBe('Writing objects: 50%')
    emit('git:op_done', { id: 7, ok: true, message: '' })
    expect(git.busy).toBeNull()
    expect(git.error).toBeNull()
  })

  it('cancel calls git_cancel with the op id; a failed op shows its message', async () => {
    ipc.gitRemoteOp.mockResolvedValue(3)
    await git.remote(REPO, 'fetch')
    await git.cancel()
    expect(ipc.gitCancel).toHaveBeenCalledWith(3)
    emit('git:op_done', { id: 3, ok: false, message: 'Operazione annullata' })
    expect(git.error).toBe('Operazione annullata')
    expect(git.busy).toBeNull()
  })

  it('a done event arriving before the id is not lost', async () => {
    let resolve!: (id: number) => void
    ipc.gitRemoteOp.mockReturnValue(new Promise<number>((r) => (resolve = r)))
    const p = git.remote(REPO, 'pull')
    emit('git:op_done', { id: 5, ok: true, message: '' })
    resolve(5)
    await p
    expect(git.busy).toBeNull()
  })

  it('blocks a second operation while busy', async () => {
    ipc.gitRemoteOp.mockResolvedValue(1)
    await git.remote(REPO, 'fetch')
    await git.remote(REPO, 'push')
    await git.stage(REPO, ['a'])
    expect(ipc.gitRemoteOp).toHaveBeenCalledTimes(1)
    expect(ipc.gitStage).not.toHaveBeenCalled()
  })

  it('shows the core error text, and clears it on the next success', async () => {
    ipc.gitStage.mockRejectedValueOnce('percorso non valido')
    await git.stage(REPO, ['-x'])
    expect(git.error).toBe('percorso non valido')
    expect(git.busy).toBeNull()
    ipc.gitStage.mockResolvedValue(undefined)
    await git.stage(REPO, ['a'])
    expect(git.error).toBeNull()
  })

  it('commit reports success only when the core accepted it', async () => {
    ipc.gitCommit.mockRejectedValueOnce('niente da committare')
    expect(await git.commit(REPO, 'x', false)).toBe(false)
    ipc.gitCommit.mockResolvedValue('abc1234')
    expect(await git.commit(REPO, 'x', true)).toBe(true)
    expect(ipc.gitCommit).toHaveBeenLastCalledWith(REPO, 'x', true)
  })

  it('opens the diff in the editor with the absolute path; binary files only warn', async () => {
    const e = { path: 'src/a.ts', origPath: null, staged: 'M', unstaged: null, conflict: false, untracked: false }
    ipc.gitDiff.mockResolvedValueOnce({ before: 'a', after: 'b', binary: false })
    await git.openDiff(REPO, e, true)
    expect(ipc.gitDiff).toHaveBeenCalledWith(REPO, 'src/a.ts', null, true)
    expect(openDiff).toHaveBeenCalledWith('/Users/dev/app/src/a.ts', 'a', 'b')
    ipc.gitDiff.mockResolvedValueOnce({ before: '', after: '', binary: true })
    await git.openDiff(REPO, e, false)
    expect(openDiff).toHaveBeenCalledTimes(1)
    expect(git.error).toMatch(/binario/)
  })

  it('merge with conflicts leaves a notice', async () => {
    ipc.gitMerge.mockResolvedValue({ kind: 'conflicts', conflicts: ['a', 'b'] })
    await git.merge(REPO, 'dev')
    expect(git.notice).toBe('Unione con 2 conflitti')
  })
})

describe('branchNameError', () => {
  it.each(['', '-x', 'a b', 'a..b', 'a~1', 'x/', '.hidden', 'a.lock', 'a@{1}', 'a:b'])('rejects %j', (n) => {
    expect(branchNameError(n)).not.toBeNull()
  })
  it.each(['main', 'feature/login', 'fix-1_a', 'è-ok'])('accepts %j', (n) => {
    expect(branchNameError(n)).toBeNull()
  })
})
