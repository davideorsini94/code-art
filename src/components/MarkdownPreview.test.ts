// @vitest-environment jsdom
import { cleanup, render, screen, waitFor } from '@testing-library/svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }))
vi.mock('@tauri-apps/plugin-opener', () => ({ openUrl: vi.fn() }))
vi.mock('../lib/ipc', () => ({ fsReadBytes: vi.fn() }))

import MarkdownPreview from './MarkdownPreview.svelte'
import { fsReadBytes } from '../lib/ipc'
import { projectStore } from '../lib/stores/project.svelte'

const PATH = '/Users/dev/app/docs/README.md'

beforeEach(() => {
  projectStore.project = { id: 'p', name: 'app', folders: ['/Users/dev/app'] }
  let n = 0
  URL.createObjectURL = vi.fn(() => `blob:test/${++n}`)
  URL.revokeObjectURL = vi.fn()
})
afterEach(() => {
  cleanup()
  vi.mocked(fsReadBytes).mockReset()
})

describe('MarkdownPreview images', () => {
  it('replaces the pending placeholder of a local image with its blob url', async () => {
    vi.mocked(fsReadBytes).mockResolvedValue(new Uint8Array([1, 2, 3]))
    render(MarkdownPreview, { text: '![logo](img/a.png)', path: PATH })
    await waitFor(() => expect(screen.getByAltText('logo').getAttribute('src')).toMatch(/^blob:test\//))
    expect(fsReadBytes).toHaveBeenCalledWith('/Users/dev/app/docs/img/a.png', 10 * 1024 * 1024)
  })

  it('never requests remote, absolute, escaping or non-image references', async () => {
    render(MarkdownPreview, { text: '![a](https://x/a.png) ![b](/etc/a.png) ![c](../../../a.png) ![d](notes.txt)', path: PATH })
    await Promise.resolve()
    expect(fsReadBytes).not.toHaveBeenCalled()
    expect(document.querySelector('img')).toBeNull()
  })

  it('shows a small notice instead of an image that stays over budget', async () => {
    vi.mocked(fsReadBytes).mockRejectedValue({ code: 'too_large', message: 'x' })
    const { container } = render(MarkdownPreview, { text: '![grande](big.png)', path: PATH })
    await waitFor(() => expect(container.querySelector('.imgmiss')?.textContent).toBe('immagine non caricata: grande'))
    expect(container.querySelector('img')).toBeNull()
  })

  it('revokes its object urls on unmount', async () => {
    vi.mocked(fsReadBytes).mockResolvedValue(new Uint8Array([1]))
    const { unmount } = render(MarkdownPreview, { text: '![a](a.png)', path: PATH })
    await waitFor(() => expect(URL.createObjectURL).toHaveBeenCalled())
    await Promise.resolve()
    unmount()
    expect(URL.revokeObjectURL).toHaveBeenCalled()
  })
})
