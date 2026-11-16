// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from '@testing-library/svelte'
import { afterEach, describe, expect, it, vi } from 'vitest'

vi.mock('../../lib/stores/project.svelte', () => ({ projectStore: { project: { id: 'p1', name: 'app', folders: [] }, untrusted: [], refreshBranch: vi.fn() } }))
vi.mock('../../lib/events', () => ({ listenProject: vi.fn(async () => () => {}) }))
const ipc = vi.hoisted(() => ({
  gitRepos: vi.fn(async () => []),
  gitStatus: vi.fn(),
  gitLog: vi.fn(async () => [{ hash: 'h', short: 'h', subject: 'Corregge il menu', author: 'octo-user', dateIso: '' }]),
  ghPrCreate: vi.fn(),
  ghPrList: vi.fn(async () => []),
}))
vi.mock('../../lib/ipc', () => ipc)

import PrCreate from './PrCreate.svelte'
import { github } from '../../lib/stores/github.svelte'
import { git } from '../../lib/stores/git.svelte'

const REPO = '/Users/dev/app'

afterEach(() => {
  cleanup()
  github.creating = null
  git.notice = null
  git.error = null
  vi.clearAllMocks()
})

describe('PrCreate', () => {
  it('prefills the title, leaves the base to the default branch, and creates the pull request', async () => {
    ipc.ghPrCreate.mockResolvedValue({ number: 12, title: 'Corregge il menu', author: 'octo-user', head: 'fix', base: 'main', state: 'open', isDraft: true, url: 'https://github.com/octo-org/app/pull/12', checks: { passing: 0, failing: 0, pending: 0 } })
    github.creating = REPO
    render(PrCreate, { repo: REPO })
    const title = (await screen.findByDisplayValue('Corregge il menu')) as HTMLInputElement
    expect(title.value).toBe('Corregge il menu')
    const base = screen.getByLabelText('Branch di destinazione') as HTMLInputElement
    expect(base.value).toBe('')
    expect(base.placeholder).toContain('predefinito')
    await fireEvent.input(screen.getByLabelText('Descrizione (markdown)'), { target: { value: 'Chiude #3' } })
    await fireEvent.click(screen.getByLabelText('Bozza'))
    await fireEvent.click(screen.getByRole('button', { name: 'Crea pull request' }))
    expect(ipc.ghPrCreate).toHaveBeenCalledWith(REPO, 'Corregge il menu', 'Chiude #3', '', true)
    await vi.waitFor(() => expect(git.notice).toBe('Pull request #12 creata'))
    expect(github.creating).toBeNull()
  })

  it('previews the body as sanitized markdown', async () => {
    render(PrCreate, { repo: REPO })
    await screen.findByDisplayValue('Corregge il menu')
    await fireEvent.input(screen.getByLabelText('Descrizione (markdown)'), { target: { value: '**forte** <img src=x onerror=alert(1)> [link](javascript:alert(1))' } })
    await fireEvent.click(screen.getByRole('tab', { name: 'Anteprima' }))
    const preview = screen.getByRole('region', { name: 'Anteprima della descrizione' })
    expect(preview.querySelector('strong')?.textContent).toBe('forte')
    expect(preview.querySelector('img')).toBeNull()
    expect(preview.innerHTML).not.toContain('javascript:')
    expect(preview.innerHTML).not.toContain('onerror')
  })

  it('needs a title', async () => {
    ipc.gitLog.mockResolvedValueOnce([])
    render(PrCreate, { repo: REPO })
    const btn = (await screen.findByRole('button', { name: 'Crea pull request' })) as HTMLButtonElement
    expect(btn.disabled).toBe(true)
  })
})
