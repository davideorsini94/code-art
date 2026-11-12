// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from '@testing-library/svelte'
import { afterEach, describe, expect, it, vi } from 'vitest'

const commit = vi.hoisted(() => vi.fn(async () => true))
vi.mock('../../lib/stores/git.svelte', () => ({ git: { busy: null, focusTick: 0, primary: '/r', commit } }))

import CommitBox from './CommitBox.svelte'
import { resetTooltipForTests } from '../../lib/tooltip'


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
  vi.clearAllMocks()
})

const btn = () => screen.getByRole('button', { name: 'Commit' }) as HTMLButtonElement
const field = () => screen.getByLabelText('Messaggio di commit') as HTMLTextAreaElement

describe('CommitBox', () => {
  it('is disabled with an empty message or nothing staged, with the reason as tooltip', async () => {
    render(CommitBox, { repo: '/r', stagedCount: 2 })
    expect(btn().disabled).toBe(true)
    expect(tipText(btn())).toContain('messaggio')
    await fireEvent.input(field(), { target: { value: '   ' } })
    expect(btn().disabled).toBe(true)
    await fireEvent.input(field(), { target: { value: 'fix' } })
    expect(btn().disabled).toBe(false)
    cleanup()
    render(CommitBox, { repo: '/r', stagedCount: 0 })
    await fireEvent.input(field(), { target: { value: 'fix' } })
    expect(btn().disabled).toBe(true)
    expect(tipText(btn())).toContain('stage')
  })

  it('⌘↵ commits and clears the field; plain Enter does not', async () => {
    render(CommitBox, { repo: '/r', stagedCount: 1 })
    await fireEvent.input(field(), { target: { value: 'feat: x' } })
    await fireEvent.keyDown(field(), { key: 'Enter' })
    expect(commit).not.toHaveBeenCalled()
    await fireEvent.keyDown(field(), { key: 'Enter', metaKey: true })
    expect(commit).toHaveBeenCalledWith('/r', 'feat: x', false)
    await vi.waitFor(() => expect(field().value).toBe(''))
  })

  it('"Modifica ultimo commit" passes amend and allows an empty stage', async () => {
    render(CommitBox, { repo: '/r', stagedCount: 0 })
    await fireEvent.click(screen.getByLabelText(/Modifica ultimo commit/))
    await fireEvent.input(field(), { target: { value: 'nuovo testo' } })
    await fireEvent.click(btn())
    expect(commit).toHaveBeenCalledWith('/r', 'nuovo testo', true)
  })

  it('keeps the message when the commit fails', async () => {
    commit.mockResolvedValueOnce(false)
    render(CommitBox, { repo: '/r', stagedCount: 1 })
    await fireEvent.input(field(), { target: { value: 'x' } })
    await fireEvent.click(btn())
    await Promise.resolve()
    expect(field().value).toBe('x')
  })
})
