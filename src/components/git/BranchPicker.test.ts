// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from '@testing-library/svelte'
import { afterEach, describe, expect, it, vi } from 'vitest'

const gitBranches = vi.hoisted(() => vi.fn())
vi.mock('../../lib/ipc', () => ({ gitBranches }))
const store = vi.hoisted(() => ({ busy: null, checkout: vi.fn(async () => true), createBranch: vi.fn(async () => true), merge: vi.fn(async () => undefined) }))
vi.mock('../../lib/stores/git.svelte', async (orig) => ({ ...(await orig<object>()), git: store }))

import BranchPicker from './BranchPicker.svelte'

const BRANCHES = [
  { name: 'main', remote: false, current: true, upstream: 'origin/main' },
  { name: 'feature/login', remote: false, current: false, upstream: null },
  { name: 'origin/main', remote: true, current: false, upstream: null },
  { name: 'origin/HEAD', remote: true, current: false, upstream: null },
]

afterEach(() => {
  cleanup()
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

  it('Escape closes', async () => {
    const onclose = await mount()
    await fireEvent.keyDown(screen.getByRole('dialog'), { key: 'Escape' })
    expect(onclose).toHaveBeenCalled()
  })
})
