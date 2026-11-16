// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte'
import { afterEach, describe, expect, it, vi } from 'vitest'

vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }))
vi.mock('../lib/events', () => ({ listenProject: vi.fn(async () => () => {}) }))
vi.mock('../lib/ipc', () => ({
  projectRename: vi.fn(async () => Promise.reject('il nome non può essere vuoto')),
  gitRepos: vi.fn(async () => ['/Users/dev/demo']),
  gitStatus: vi.fn(async () => ({ root: '/Users/dev/demo', branch: 'main', upstream: null, ahead: 0, behind: 0, detached: false, merging: false, entries: [], truncated: false })),
  gitBranches: vi.fn(async () => [{ name: 'main', remote: false, current: true, upstream: null }]),
}))

import Titlebar from './Titlebar.svelte'
import { projectStore } from '../lib/stores/project.svelte'
import { resetTooltipForTests } from '../lib/tooltip'


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

afterEach(() => {
  cleanup()
  projectStore.project = null
})

describe('Titlebar', () => {
  it('no ⌘K pill on the welcome window (no project)', () => {
    render(Titlebar)
    expect(screen.queryByText(/Cerca file, conversazioni, comandi/)).toBeNull()
    cleanup()
    projectStore.project = { id: 'p', name: 'demo', folders: ['/Users/dev/demo'] }
    render(Titlebar)
    expect(screen.getByText(/Cerca file, conversazioni, comandi/)).toBeTruthy()
  })

  it('a rejected rename shows the error and keeps the old name', async () => {
    projectStore.project = { id: 'p', name: 'demo', folders: ['/Users/dev/demo'] }
    render(Titlebar)
    await fireEvent.dblClick(screen.getByText('demo'))
    const input = screen.getByLabelText('Nome del progetto')
    await fireEvent.input(input, { target: { value: 'nuovo' } })
    await fireEvent.keyDown(input, { key: 'Enter' })
    expect((await screen.findByRole('alert')).textContent).toContain('il nome non può essere vuoto')
    expect(screen.getByText('demo')).toBeTruthy()
  })

  it('the branch is a button with the "Cambia branch" tooltip', () => {
    projectStore.project = { id: 'p', name: 'demo', folders: ['/Users/dev/demo'] }
    projectStore.branch = 'main'
    render(Titlebar)
    const b = screen.getByRole('button', { name: /main/ })
    expect(tipText(b)).toBe('Cambia branch')
    projectStore.branch = null
  })

  it('clicking the branch opens the branch picker, Escape closes it', async () => {
    projectStore.project = { id: 'p', name: 'demo', folders: ['/Users/dev/demo'] }
    projectStore.branch = 'main'
    render(Titlebar)
    await fireEvent.click(screen.getByRole('button', { name: /main/ }))
    const dlg = await screen.findByRole('dialog', { name: 'Branch' })
    expect(await screen.findByText('Crea branch da qui…')).toBeTruthy()
    await fireEvent.keyDown(dlg, { key: 'Escape' })
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull())
    projectStore.branch = null
  })
})
