// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from '@testing-library/svelte'
import { afterEach, describe, expect, it, vi } from 'vitest'

vi.mock('../lib/events', () => ({ listenProject: vi.fn(async () => () => {}) }))
vi.mock('../lib/ipc', () => ({ searchStart: vi.fn(async () => 1), searchCancel: vi.fn(async () => {}), searchReplace: vi.fn() }))
const project = vi.hoisted(() => ({ project: { id: 'p1', name: 'app', folders: ['/Users/dev/app'] }, untrusted: [] as string[] }))
vi.mock('../lib/stores/project.svelte', () => ({ projectStore: project }))
const openAt = vi.fn()
vi.mock('../lib/stores/editor.svelte', () => ({ editor: { tabs: [], openAt: (...a: unknown[]) => openAt(...a) } }))

import SearchPanel from './SearchPanel.svelte'
import { search } from '../lib/stores/search.svelte'

afterEach(() => {
  cleanup()
  search.unbind()
  search.groups = []
  search.truncated = false
  search.status = 'idle'
  project.untrusted = []
})

describe('SearchPanel', () => {
  it('shows hits grouped by file, highlights with <mark> text, and opens the hit on click', async () => {
    search.status = 'done'
    search.hitCount = 1
    search.groups = [{ path: '/Users/dev/app/src/a.ts', hits: [{ path: '/Users/dev/app/src/a.ts', line: 3, col: 8, len: 6, preview: '  foo <b>needle</b>', previewCol: 9, previewLen: 6 }] }]
    const { container } = render(SearchPanel)
    expect(screen.getByText('a.ts')).toBeTruthy()
    const mark = container.querySelector('mark')!
    expect(mark.textContent).toBe('needle')
    expect(container.querySelector('.pv b')).toBeNull() // il markup dei dati resta testo
    await fireEvent.click(mark.closest('button')!)
    expect(openAt).toHaveBeenCalledWith('/Users/dev/app/src/a.ts', 3, 8)
  })

  it('toggles Aa / ab / .* have tooltips and flip the option', async () => {
    render(SearchPanel)
    for (const [label, key] of [['Distingui maiuscole e minuscole', 'caseSensitive'], ['Solo parole intere', 'wholeWord'], ['Espressione regolare', 'regex']] as const) {
      const b = screen.getByLabelText(label)
      expect(b.getAttribute('data-tt')).not.toBeNull()
      expect(b.getAttribute('aria-pressed')).toBe('false')
      await fireEvent.click(b)
      expect(search[key]).toBe(true)
      expect(b.getAttribute('aria-pressed')).toBe('true')
      search[key] = false
    }
  })

  it('hides the replace field in an untrusted project', () => {
    project.untrusted = ['/Users/dev/app']
    render(SearchPanel)
    expect(screen.queryByPlaceholderText('Sostituisci')).toBeNull()
  })

  it('shows the replace field when trusted, and the truncation notice', () => {
    search.truncated = true
    search.status = 'done'
    render(SearchPanel)
    expect(screen.getByPlaceholderText('Sostituisci')).toBeTruthy()
    expect(screen.getByText('Mostrati i primi 10 000 risultati')).toBeTruthy()
  })
})
