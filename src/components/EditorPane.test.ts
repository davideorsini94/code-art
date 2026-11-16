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
const diffOptions = vi.hoisted(() => vi.fn())
const zones = vi.hoisted(() => ({ list: [] as { afterLineNumber: number; domNode: HTMLElement }[] }))
vi.mock('../lib/monaco', () => {
  const model = () => {
    let v = ''
    let ver = 1
    const subs: (() => void)[] = []
    return {
      getValue: () => v, setValue: (x: string) => { v = x; ver++; subs.forEach((f) => f()) }, dispose() {}, getAlternativeVersionId: () => ver,
      onDidChangeContent: (f: () => void) => { subs.push(f); return { dispose() { subs.splice(subs.indexOf(f), 1) } } },
      getFullModelRange: () => ({}),
      pushEditOperations: (_s: unknown, ops: { text: string }[]) => { v = ops[0].text; ver++; subs.forEach((f) => f()); return null },
      init: (x: string) => (v = x),
    }
  }
  const ed = () => ({
    addCommand() {}, updateOptions() {}, setModel() {}, getModel: () => null, dispose() {}, setValue() {},
    createDecorationsCollection: () => ({ set() {}, clear() {} }),
    changeViewZones: (cb: (a: { addZone: (z: { afterLineNumber: number; domNode: HTMLElement }) => string; removeZone: (id: string) => void }) => void) =>
      cb({ addZone: (z) => { zones.list.push(z); return String(zones.list.length) }, removeZone: () => { zones.list.length = 0 } }),
    revealLineInCenter() {}, setPosition() {}, focus() {},
  })
  return {
    monaco: {
      KeyMod: { CtrlCmd: 1 }, KeyCode: { KeyS: 1 }, Uri: { file: (p: string) => p, parse: (p: string) => p },
      editor: {
        create: ed, createDiffEditor: () => ({ ...ed(), updateOptions: diffOptions, setModel() {}, getModifiedEditor: () => ed() }),
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

describe('EditorPane diff tabs', () => {
  it('an editable (unstaged) diff makes the modified side editable, the original never; ⌘S path saves it', async () => {
    render(EditorPane)
    await editor.openDiff('/Users/dev/app/a.ts', 'indice', 'x', { editable: true })
    flushSync()
    await waitFor(() => expect(diffOptions).toHaveBeenCalledWith({ readOnly: false, originalEditable: false }))
    const t = editor.activeTab!
    await editor.save(t.path, t.key)
    expect(fsWrite).toHaveBeenCalledWith('/Users/dev/app/a.ts', '# Titolo\n\nciao', 1)
  })

  it('a staged diff stays read-only on both sides and cannot be saved', async () => {
    diffOptions.mockClear()
    vi.mocked(fsWrite).mockClear()
    render(EditorPane)
    await editor.openDiff('/Users/dev/app/b.ts', 'indice', 'x')
    flushSync()
    await waitFor(() => expect(diffOptions).toHaveBeenCalledWith({ readOnly: true, originalEditable: false }))
    await editor.save('/Users/dev/app/b.ts', editor.activeTab!.key)
    expect(fsWrite).not.toHaveBeenCalled()
  })
})

describe('EditorPane merge conflicts', () => {
  const marked = 'a\n<<<<<<< HEAD\nmia\n=======\nsua\n>>>>>>> f\nb\n'

  it('shows the count and an actions row above each conflict; an action edits the text and marks the tab dirty', async () => {
    zones.list.length = 0
    vi.mocked(fsRead).mockResolvedValueOnce({ text: marked, mtimeMs: 1 })
    render(EditorPane)
    await editor.open('/Users/dev/app/m.ts')
    flushSync()
    await waitFor(() => expect(screen.getByText('1 conflitto')).toBeTruthy())
    expect(zones.list).toHaveLength(1)
    expect(zones.list[0].afterLineNumber).toBe(1)
    const labels = [...zones.list[0].domNode.querySelectorAll('button')].map((b) => b.textContent)
    expect(labels).toEqual(['Tieni la tua', 'Tieni l\'altra', 'Tieni entrambe'])
    expect(editor.activeTab!.dirty).toBe(false)
    zones.list[0].domNode.querySelectorAll('button')[0].click()
    flushSync()
    await waitFor(() => expect(editor.activeTab!.dirty).toBe(true))
    await waitFor(() => expect(screen.queryByText('1 conflitto')).toBeNull())
    expect(zones.list).toHaveLength(0)
  })

  it('shows no conflict controls for a file without markers', async () => {
    render(EditorPane)
    await editor.open('/Users/dev/app/clean.ts')
    flushSync()
    expect(screen.queryByText(/conflitt/)).toBeNull()
  })
})
