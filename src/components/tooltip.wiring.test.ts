// @vitest-environment jsdom
import { cleanup, render, screen } from '@testing-library/svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import Rail from './Rail.svelte'
import { resetTooltipForTests } from '../lib/tooltip'

beforeEach(() => vi.useFakeTimers())
afterEach(() => {
  cleanup()
  resetTooltipForTests()
  vi.useRealTimers()
})

describe('components wire the tooltip', () => {
  it('hovering a real button shows its text in the shared tooltip, keeping its own accessible name', () => {
    render(Rail, { collapsed: new Set<string>(), ontoggle: () => {} })
    const btn = screen.getByRole('button', { name: 'Progetto' })
    btn.dispatchEvent(new PointerEvent('pointerover', { bubbles: true }))
    vi.advanceTimersByTime(500)
    const tip = document.getElementById('codeart-tooltip')!
    expect(tip.style.display).toBe('block')
    expect(tip.textContent).toBe('Nascondi i file del progetto')
    expect(btn.hasAttribute('title')).toBe(false)
  })
})
