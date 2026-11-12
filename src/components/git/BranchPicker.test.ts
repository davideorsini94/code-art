// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from '@testing-library/svelte'
import { afterEach, describe, expect, it, vi } from 'vitest'

const gitBranches = vi.hoisted(() => vi.fn())
vi.mock('../../lib/ipc', () => ({ gitBranches }))
vi.mock('../../lib/events', () => ({ listenProject: vi.fn(async () => () => {}) }))
vi.mock('../../lib/stores/project.svelte', () => ({ projectStore: { project: null, untrusted: [], refreshBranch: vi.fn() } }))
vi.mock('../../lib/stores/editor.svelte', () => ({ editor: {} }))
// store reale (reattivo), con le operazioni del core sostituite
import { git as store } from '../../lib/stores/git.svelte'
store.checkout = vi.fn(async () => true)
store.createBranch = vi.fn(async () => true)
store.merge = vi.fn(async () => undefined)

import BranchPicker from './BranchPicker.svelte'

const BRANCHES = [
  { name: 'main', remote: false, current: true, upstream: 'origin/main' },
  { name: 'feature/login', remote: false, current: false, upstream: null },
  { name: 'origin/main', remote: true, current: false, upstream: null },
  { name: 'origin/HEAD', remote: true, current: false, upstream: null },
]

afterEach(() => {
  cleanup()
  store.error = null
  store.notice = null
  store.busy = null
  vi.clearAllMocks()
})

async function mount(mode: 'switch' | 'create' | 'merge' = 'switch') {
  gitBranches.mockResolvedValue(BRANCHES)
  const onclose = vi.fn()
  render(BranchPicker, { repo: '/r', mode, onclose })
  await screen.findByText('feature/login')
  return onclose
}

describe('BranchPicker', () => {
  it('lists local and remote branches (not origin/HEAD) and filters them', async () => {
    await mount()
    expect(screen.getByText('origin/main')).toBeTruthy()
    expect(screen.queryByText('origin/HEAD')).toBeNull()
    await fireEvent.input(screen.getByLabelText('Filtra i branch'), { target: { value: 'LOGIN' } })
    expect(screen.queryByText('origin/main')).toBeNull()
    expect(screen.getByText('feature/login')).toBeTruthy()
  })

  it('switching to a branch calls checkout and closes; the current one is inert', async () => {
    const onclose = await mount()
    await fireEvent.click(screen.getByText('main'))
    expect(store.checkout).not.toHaveBeenCalled()
    await fireEvent.click(screen.getByText('feature/login'))
    expect(store.checkout).toHaveBeenCalledWith('/r', 'feature/login')
    await vi.waitFor(() => expect(onclose).toHaveBeenCalled())
  })

  it('an invalid new name shows an error and disables "Crea"; a valid one creates from the current branch', async () => {
    const onclose = await mount('create')
    const input = screen.getByLabelText('Nome del nuovo branch')
    const crea = screen.getByRole('button', { name: 'Crea' }) as HTMLButtonElement
    expect(crea.disabled).toBe(true)
    await fireEvent.input(input, { target: { value: 'a b' } })
    expect(screen.getByRole('alert').textContent).toMatch(/non sono ammessi/)
    expect(crea.disabled).toBe(true)
    await fireEvent.input(input, { target: { value: 'main' } })
    expect(screen.getByRole('alert').textContent).toMatch(/Esiste già/)
    await fireEvent.input(input, { target: { value: 'feature/new' } })
    expect(screen.queryByRole('alert')).toBeNull()
    expect(crea.disabled).toBe(false)
    await fireEvent.click(crea)
    expect(store.createBranch).toHaveBeenCalledWith('/r', 'feature/new', 'main')
    await vi.waitFor(() => expect(onclose).toHaveBeenCalled())
  })

  it('merge mode merges the picked branch into the current one', async () => {
    await mount('merge')
    await fireEvent.click(screen.getByText('feature/login'))
    expect(store.merge).toHaveBeenCalledWith('/r', 'feature/login')
    expect(store.checkout).not.toHaveBeenCalled()
  })

  it('a failed checkout keeps the picker open and shows the error inside it', async () => {
    const onclose = await mount()
    vi.mocked(store.checkout).mockImplementationOnce(async () => {
      store.error = 'Le modifiche locali verrebbero sovrascritte'
      return false
    })
    await fireEvent.click(screen.getByText('feature/login'))
    await vi.waitFor(() => expect(store.checkout).toHaveBeenCalled())
    await Promise.resolve()
    expect(onclose).not.toHaveBeenCalled()
    // il messaggio compare tra le righe del selettore dopo il rendering
    await vi.waitFor(() => expect(screen.getByRole('alert').textContent).toContain('sovrascritte'))
  })

  it('merge mode closes only on success; with conflicts or an error it stays open with the message', async () => {
    const onclose = await mount('merge')
    vi.mocked(store.merge).mockImplementationOnce(async () => {
      store.notice = 'Unione con 1 conflitto: risolvili o annulla l’unione'
      return { kind: 'conflicts' as const, conflicts: ['a'] }
    })
    await fireEvent.click(screen.getByText('feature/login'))
    await vi.waitFor(() => expect(screen.getByRole('status').textContent).toContain('1 conflitto'))
    expect(onclose).not.toHaveBeenCalled()
    vi.mocked(store.merge).mockImplementationOnce(async () => undefined) // errore del core
    await fireEvent.click(screen.getByText('feature/login'))
    await Promise.resolve()
    await Promise.resolve()
    expect(onclose).not.toHaveBeenCalled()
    vi.mocked(store.merge).mockImplementationOnce(async () => ({ kind: 'merged' as const, conflicts: [] }))
    await fireEvent.click(screen.getByText('feature/login'))
    await vi.waitFor(() => expect(onclose).toHaveBeenCalled())
  })

  it('Escape closes', async () => {
    const onclose = await mount()
    await fireEvent.keyDown(screen.getByRole('dialog'), { key: 'Escape' })
    expect(onclose).toHaveBeenCalled()
  })
})
