// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from '@testing-library/svelte'
import { afterEach, describe, expect, it, vi } from 'vitest'

vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }))
vi.mock('../lib/ipc', () => ({ projectRename: vi.fn(async () => Promise.reject('il nome non può essere vuoto')) }))

import Titlebar from './Titlebar.svelte'
import { projectStore } from '../lib/stores/project.svelte'

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
})
