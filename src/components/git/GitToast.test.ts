// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from '@testing-library/svelte'
import { afterEach, describe, expect, it, vi } from 'vitest'

vi.mock('../../lib/ipc', () => ({ gitCancel: vi.fn(async () => {}) }))
vi.mock('../../lib/events', () => ({ listenProject: vi.fn(async () => () => {}) }))
vi.mock('../../lib/stores/project.svelte', () => ({ projectStore: { project: null, untrusted: [] } }))
vi.mock('../../lib/stores/editor.svelte', () => ({ editor: {} }))

import GitToast from './GitToast.svelte'
import { git } from '../../lib/stores/git.svelte'

afterEach(() => {
  cleanup()
  git.error = null
  git.notice = null
  git.busy = null
  vi.useRealTimers()
})

describe('GitToast', () => {
  it('shows an error outside the Git view and dismisses it', async () => {
    git.error = 'Le modifiche locali verrebbero sovrascritte'
    render(GitToast)
    expect(screen.getByText(/sovrascritte/)).toBeTruthy()
    await fireEvent.click(screen.getByRole('button', { name: 'Chiudi il messaggio' }))
    expect(git.error).toBeNull()
  })

  it('stays hidden while the Git panel is in view (it shows the messages itself)', () => {
    git.error = 'x'
    render(GitToast, { panelVisible: true })
    expect(screen.queryByText('x')).toBeNull()
  })

  it('shows progress with Annulla for a remote op started elsewhere (palette)', async () => {
    git.busy = { repo: '/r', op: 'push', id: 3, line: 'Writing objects: 40%' }
    render(GitToast)
    expect(screen.getByText(/Push · Writing objects: 40%/)).toBeTruthy()
    await fireEvent.click(screen.getByRole('button', { name: 'Annulla' }))
  })

  it('an outcome notice disappears by itself', async () => {
    vi.useFakeTimers()
    git.notice = 'Pull completato'
    render(GitToast)
    expect(screen.getByText('Pull completato')).toBeTruthy()
    await vi.advanceTimersByTimeAsync(6100)
    expect(git.notice).toBeNull()
  })
})
