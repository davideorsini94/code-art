// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte'
import { flushSync } from 'svelte'
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest'

vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }))
vi.mock('../lib/events', () => ({ listenProject: vi.fn(async () => () => {}) }))
vi.mock('@tauri-apps/plugin-opener', () => ({ openUrl: vi.fn() }))
vi.mock('../lib/ipc', () => ({
  fsRead: vi.fn(async () => ({ text: '# Titolo\n\nciao', mtimeMs: 1 })),
  fsReadBytes: vi.fn(async () => new Uint8Array([1, 2, 3])),
  fsWrite: vi.fn(async () => 2),
  asFsError: (e: unknown) => (e && typeof e === 'object' && 'code' in e ? e : { code: 'io', message: String(e) }),
}))
vi.mock('../lib/monaco', () => {
  const model = () => {
    let v = ''
    return {
      getValue: () => v, setValue: (x: string) => (v = x), dispose() {}, getAlternativeVersionId: () => 1,
      onDidChangeContent: () => ({ dispose() {} }),
      init: (x: string) => (v = x),
    }
  }
  const ed = () => ({ addCommand() {}, updateOptions() {}, setModel() {}, getModel: () => null, dispose() {}, setValue() {} })
  return {
    monaco: {
      KeyMod: { CtrlCmd: 1 }, KeyCode: { KeyS: 1 }, Uri: { file: (p: string) => p, parse: (p: string) => p },
      editor: {
        create: ed, createDiffEditor: () => ({ ...ed(), setModel() {} }),
        createModel: (text: string) => { const m = model(); m.init(text); return m },
      },
    },
  }
})

import EditorPane from './EditorPane.svelte'
import { editor } from '../lib/stores/editor.svelte'
import { fsRead, fsWrite } from '../lib/ipc'

beforeAll(() => {
  window.matchMedia = ((q: string) => ({ matches: false, media: q, addEventListener() {}, removeEventListener() {} })) as unknown as typeof window.matchMedia
})
afterEach(() => {
  cleanup()
  editor.tabs = []
  editor.active = null
})

describe('EditorPane markdown tabs', () => {
  it('opens a .md file in preview mode and switches to the editor with "Modifica"', async () => {
    render(EditorPane)
    await editor.open('/Users/dev/app/README.md')
    flushSync()
    await waitFor(() => expect(screen.getByTestId('md-preview')).toBeTruthy())
    expect(screen.getByTestId('editors').classList.contains('hidden')).toBe(true)
    expect(screen.getByRole('radio', { name: 'Anteprima' }).getAttribute('aria-checked')).toBe('true')

    await fireEvent.click(screen.getByRole('radio', { name: 'Modifica' }))
    expect(screen.getByTestId('editors').classList.contains('hidden')).toBe(false)
    expect(screen.queryByTestId('md-preview')).toBeNull()

    await fireEvent.click(screen.getByRole('radio', { name: 'Affiancate' }))
    expect(screen.getByTestId('editors').classList.contains('hidden')).toBe(false)
    await waitFor(() => expect(screen.getByTestId('md-preview')).toBeTruthy())
  })

  it('remembers the mode per file', async () => {
    render(EditorPane)
    await editor.open('/Users/dev/app/a.md')
    editor.setMode('/Users/dev/app/a.md', 'edit')
    await editor.open('/Users/dev/app/b.md')
    flushSync()
    expect(editor.modeOf('/Users/dev/app/b.md')).toBe('preview')
    expect(editor.modeOf('/Users/dev/app/a.md')).toBe('edit')
  })

  it('does not show the markdown toolbar for plain text files', async () => {
    render(EditorPane)
    await editor.open('/Users/dev/app/main.rs')
    flushSync()
    expect(screen.queryByRole('radiogroup')).toBeNull()
    expect(screen.getByTestId('editors').classList.contains('hidden')).toBe(false)
  })

  it('opens images without reading them as text, and "Apri come testo" reopens an SVG as text', async () => {
    render(EditorPane)
    vi.mocked(fsRead).mockClear()
    await editor.open('/Users/dev/app/logo.svg')
    expect(editor.activeTab?.viewer).toBe('image')
    expect(fsRead).not.toHaveBeenCalled()
    await editor.open('/Users/dev/app/logo.svg', true)
    expect(editor.activeTab?.viewer).toBe('text')
    expect(fsRead).toHaveBeenCalledTimes(1)
    expect(editor.tabs.filter((t) => t.path.endsWith('logo.svg'))).toHaveLength(1)
  })

  it('⌘S saves the active tab at window level, but not a second time when Monaco has the focus', async () => {
    render(EditorPane)
    await editor.open('/Users/dev/app/main.rs')
    flushSync()
    const t = editor.activeTab!
    editor.bind(t.id, { snapshot: () => ({ text: 'nuovo', commit() {} }) })
    vi.mocked(fsWrite).mockClear()
    await fireEvent.keyDown(document.body, { key: 's', metaKey: true })
    expect(fsWrite).toHaveBeenCalledTimes(1)
    const inMonaco = document.createElement('div')
    inMonaco.className = 'monaco-editor'
    const area = document.createElement('textarea')
    inMonaco.appendChild(area)
    document.body.appendChild(inMonaco)
    area.focus()
    await fireEvent.keyDown(area, { key: 's', metaKey: true })
    expect(fsWrite).toHaveBeenCalledTimes(1)
    inMonaco.remove()
  })

  it('closing a dirty tab shows the inline "Modifiche non salvate" bar', async () => {
    render(EditorPane)
    await editor.open('/Users/dev/app/main.rs')
    editor.setDirty(editor.activeTab!.id, true)
    flushSync()
    await fireEvent.click(screen.getByRole('button', { name: 'Chiudi main.rs' }))
    expect(screen.getByText('Modifiche non salvate in main.rs')).toBeTruthy()
    await fireEvent.click(screen.getByText('Scarta'))
    expect(editor.tabs).toHaveLength(0)
  })
})
