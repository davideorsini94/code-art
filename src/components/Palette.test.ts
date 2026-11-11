// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte'
import { tick } from 'svelte'
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest'

vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }))
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => []) }))

import Palette from './Palette.svelte'
import { palette } from '../lib/palette/state.svelte'
import { hooks } from '../lib/palette/search'

beforeAll(() => {
  Element.prototype.scrollIntoView = vi.fn()
})
afterEach(() => {
  cleanup()
  palette.close()
  document.body.innerHTML = ''
})

describe('Palette focus', () => {
  it('a command that moves focus keeps it after the palette closes', async () => {
    const opener = document.createElement('button')
    const target = document.createElement('input')
    document.body.append(opener, target)
    opener.focus()
    hooks.renameProject = () => target.focus()
    render(Palette)
    palette.show('commands')
    const box = (await screen.findByRole('combobox')) as HTMLInputElement
    await fireEvent.input(box, { target: { value: 'Rinomina' } })
    await waitFor(() => expect(screen.getByRole('option', { name: /Rinomina progetto/ })).toBeTruthy())
    await fireEvent.keyDown(box, { key: 'Enter' })
    await waitFor(() => expect(document.activeElement).toBe(target))
    await tick()
    expect(screen.queryByRole('dialog')).toBeNull()
    expect(document.activeElement).toBe(target)
  })

  it('Esc closes and gives focus back to the previous element', async () => {
    const opener = document.createElement('button')
    document.body.append(opener)
    opener.focus()
    render(Palette)
    palette.show('all')
    const box = await screen.findByRole('combobox')
    await fireEvent.keyDown(box, { key: 'Escape' })
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull())
    expect(document.activeElement).toBe(opener)
  })
})
