// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from '@testing-library/svelte'
import { afterEach, describe, expect, it, vi } from 'vitest'
import ConvCard from './ConvCard.svelte'
import type { Conversation } from '../lib/types'

const now = new Date(2026, 10, 11, 15, 0, 0).getTime()
const base: Conversation = {
  sessionId: 's1', projectId: 'p', cwd: '/Users/dev/app', title: 'Numerazione fatture', prevTitle: null, titleChangedAt: null,
  titleLocked: false, summary: 'Trovata la race condition.', status: 'idle', owned: false, archived: false, lastActivity: now - 2 * 60_000,
  turnCount: 3, model: null, permissionMode: 'default', contextUsedPct: null, maybeOpenElsewhere: false,
}
afterEach(cleanup)

describe('ConvCard', () => {
  it('rende le tre etichette di stato; idle senza etichetta', () => {
    const { unmount } = render(ConvCard, { conv: { ...base, status: 'waiting' }, now })
    expect(screen.getByText('aspetta te')).toBeTruthy()
    unmount()
    const r2 = render(ConvCard, { conv: { ...base, status: 'working' }, now })
    expect(screen.getByText('al lavoro')).toBeTruthy()
    r2.unmount()
    render(ConvCard, { conv: base, now })
    expect(screen.queryByText('aspetta te')).toBeNull()
    expect(screen.queryByText('al lavoro')).toBeNull()
  })

  it('la classe pulse è solo su working', () => {
    for (const status of ['waiting', 'working', 'idle'] as const) {
      const { unmount } = render(ConvCard, { conv: { ...base, status }, now })
      expect(screen.getByTestId('dot').classList.contains('pulse')).toBe(status === 'working')
      unmount()
    }
  })

  it('mostra titolo, riassunto e tempo relativo', () => {
    render(ConvCard, { conv: base, now })
    expect(screen.getByText('Numerazione fatture')).toBeTruthy()
    expect(screen.getByText('Trovata la race condition.')).toBeTruthy()
    expect(screen.getByText('2 min fa')).toBeTruthy()
  })

  it('il titolo precedente compare solo se prevTitle è impostato', () => {
    const { unmount } = render(ConvCard, { conv: base, now })
    expect(screen.queryByText(/^era:/)).toBeNull()
    unmount()
    render(ConvCard, { conv: { ...base, prevTitle: 'Vecchio titolo' }, now })
    expect(screen.getByText('era: Vecchio titolo')).toBeTruthy()
  })

  it('tratta titolo e riassunto come testo, non come HTML', () => {
    const evil = '<img src=x onerror=alert(1)><b>x</b>'
    const { container } = render(ConvCard, { conv: { ...base, title: evil, summary: evil }, now })
    expect(container.querySelector('img')).toBeNull()
    expect(container.querySelector('b')).toBeNull()
    expect(container.textContent).toContain(evil)
  })

  it('lucchetto solo se titleLocked; il clic sblocca senza selezionare', async () => {
    const onunlock = vi.fn()
    const onselect = vi.fn()
    const { unmount } = render(ConvCard, { conv: base, now })
    expect(screen.queryByLabelText('Sblocca il titolo')).toBeNull()
    unmount()
    render(ConvCard, { conv: { ...base, titleLocked: true }, now, onunlock, onselect })
    await fireEvent.click(screen.getByLabelText('Sblocca il titolo'))
    expect(onunlock).toHaveBeenCalledOnce()
    expect(onselect).not.toHaveBeenCalled()
  })

  it('icona "altrove" con suggerimento solo se maybeOpenElsewhere', () => {
    const { unmount } = render(ConvCard, { conv: base, now })
    expect(screen.queryByLabelText('Forse aperta in un altro programma')).toBeNull()
    unmount()
    render(ConvCard, { conv: { ...base, maybeOpenElsewhere: true }, now })
    expect(screen.getByLabelText('Forse aperta in un altro programma')).toBeTruthy()
  })

  it('doppio click sul titolo avvia la rinomina; Invio conferma il testo nuovo', async () => {
    const onstartrename = vi.fn()
    const onrename = vi.fn()
    const { rerender } = render(ConvCard, { conv: base, now, onstartrename, onrename })
    await fireEvent.dblClick(screen.getByText('Numerazione fatture'))
    expect(onstartrename).toHaveBeenCalledOnce()
    await rerender({ conv: base, now, renaming: true, onstartrename, onrename })
    const input = screen.getByLabelText('Titolo della conversazione') as HTMLInputElement
    expect(input.value).toBe('Numerazione fatture')
    await fireEvent.input(input, { target: { value: '  Nuovo titolo ' } })
    await fireEvent.keyDown(input, { key: 'Enter' })
    expect(onrename).toHaveBeenCalledExactlyOnceWith('Nuovo titolo')
  })

  it('Escape annulla la rinomina senza chiamare onrename', async () => {
    const onrename = vi.fn()
    const oncancelrename = vi.fn()
    render(ConvCard, { conv: base, now, renaming: true, onrename, oncancelrename })
    const input = screen.getByLabelText('Titolo della conversazione')
    await fireEvent.input(input, { target: { value: 'Altro' } })
    await fireEvent.keyDown(input, { key: 'Escape' })
    await fireEvent.blur(input)
    expect(onrename).not.toHaveBeenCalled()
    expect(oncancelrename).toHaveBeenCalledOnce()
  })

  it('la bozza di rinomina sopravvive a un nuovo oggetto conv (upsert) durante la digitazione', async () => {
    const { rerender } = render(ConvCard, { conv: base, now, renaming: true })
    const input = screen.getByLabelText('Titolo della conversazione') as HTMLInputElement
    await fireEvent.input(input, { target: { value: 'Sto scrivendo' } })
    await rerender({ conv: { ...base, title: 'Titolo rigenerato', summary: 'Altro riassunto', lastActivity: now }, now, renaming: true })
    expect((screen.getByLabelText('Titolo della conversazione') as HTMLInputElement).value).toBe('Sto scrivendo')
  })
})
