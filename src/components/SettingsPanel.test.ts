// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const stored: Record<string, string | null> = {}
vi.mock('../lib/ipc', () => ({
  settingsGet: vi.fn(async (key: string) => stored[key] ?? null),
  settingsSet: vi.fn(async (key: string, value: string) => {
    stored[key] = value
  }),
}))

import SettingsPanel from './SettingsPanel.svelte'
import { settingsSet } from '../lib/ipc'

beforeEach(() => {
  for (const k of Object.keys(stored)) delete stored[k]
  vi.mocked(settingsSet).mockClear()
})
afterEach(cleanup)

const radio = (name: string) => screen.getByRole('radio', { name }) as HTMLInputElement

describe('SettingsPanel', () => {
  it('without saved values shows the defaults the app actually uses', async () => {
    render(SettingsPanel)
    await waitFor(() => expect(radio('Senza conferme').checked).toBe(true))
    expect(radio('Solo avanzamento rapido').checked).toBe(true)
  })

  it('shows the saved values', async () => {
    stored.default_permission_mode = 'plan'
    stored.git_pull_mode = 'rebase'
    render(SettingsPanel)
    await waitFor(() => expect(radio('Piano').checked).toBe(true))
    expect(radio('Rebase').checked).toBe(true)
  })

  it('saves the permission mode when one is chosen', async () => {
    render(SettingsPanel)
    await waitFor(() => expect(radio('Senza conferme').checked).toBe(true))
    await fireEvent.click(radio('Chiedi sempre'))
    expect(settingsSet).toHaveBeenCalledWith('default_permission_mode', 'default')
  })

  it('saves the pull mode when one is chosen', async () => {
    render(SettingsPanel)
    await waitFor(() => expect(radio('Solo avanzamento rapido').checked).toBe(true))
    await fireEvent.click(radio('Merge'))
    expect(settingsSet).toHaveBeenCalledWith('git_pull_mode', 'merge')
  })

  it('shows an error and keeps the previous choice when saving fails', async () => {
    vi.mocked(settingsSet).mockRejectedValueOnce('valore non valido per git_pull_mode')
    render(SettingsPanel)
    await waitFor(() => expect(radio('Solo avanzamento rapido').checked).toBe(true))
    await fireEvent.click(radio('Merge'))
    await waitFor(() => expect(screen.getByRole('alert').textContent).toContain('valore non valido'))
    expect(radio('Solo avanzamento rapido').checked).toBe(true)
  })
})
