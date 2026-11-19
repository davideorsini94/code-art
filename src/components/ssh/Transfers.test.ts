// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from '@testing-library/svelte'
import { flushSync } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

vi.mock('../../lib/events', () => ({ listenSshState: vi.fn(async () => () => {}), listenProject: vi.fn(async () => () => {}) }))
vi.mock('../../lib/ipc', () => ({ sshTransferCancel: vi.fn(async () => {}) }))
vi.mock('../../lib/stores/project.svelte', () => ({ projectStore: { project: { id: 'p1', folders: ['/Users/dev/app'] } } }))

import * as ipc from '../../lib/ipc'
import { ssh } from '../../lib/stores/ssh.svelte'
import Transfers from './Transfers.svelte'

const ev = (o: object) => ({ transferId: 't1', done: 0, total: 100, file: '/home/dev/a.txt', state: 'running', ...o }) as never

beforeEach(() => {
  vi.useFakeTimers()
  ssh.transfers = []
})
afterEach(() => {
  cleanup()
  vi.useRealTimers()
})

describe('Transfers', () => {
  it('shows progress from events and cancels a running transfer', async () => {
    render(Transfers)
    ssh.applyTransfer(ev({ done: 40 }))
    flushSync()
    expect(screen.getByText('a.txt')).toBeTruthy()
    expect(screen.getByRole('progressbar').getAttribute('aria-valuenow')).toBe('40')
    await fireEvent.click(screen.getByText('Annulla'))
    expect(ipc.sshTransferCancel).toHaveBeenCalledWith('t1')
  })

  it('hides a clean finished transfer after a few seconds', () => {
    render(Transfers)
    ssh.applyTransfer(ev({ done: 100, state: 'done' }))
    flushSync()
    expect(screen.getByText('Completato')).toBeTruthy()
    vi.advanceTimersByTime(8100)
    flushSync()
    expect(screen.queryByText('Completato')).toBeNull()
  })

  it('keeps a transfer with errors until dismissed and lists them on demand', async () => {
    render(Transfers)
    ssh.applyTransfer(ev({ state: 'done', errors: [{ path: '/home/dev/b.txt', message: 'permesso negato' }] }))
    flushSync()
    vi.advanceTimersByTime(20000)
    flushSync()
    await fireEvent.click(screen.getByText('1 errore'))
    expect(screen.getByText(/permesso negato/)).toBeTruthy()
    await fireEvent.click(screen.getByLabelText('Chiudi'))
    expect(screen.queryByText('1 errore')).toBeNull()
  })
})
