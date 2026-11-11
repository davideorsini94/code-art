import { beforeEach, describe, expect, it, vi } from 'vitest'

const fsRead = vi.fn(async (_p: string) => ({ text: 'disco', mtimeMs: 1 }))
const fsWrite = vi.fn(async (_p: string, _t: string, _m: number | null) => 2)
vi.mock('../ipc', () => ({
  fsRead: (p: string) => fsRead(p),
  fsWrite: (p: string, t: string, m: number | null) => fsWrite(p, t, m),
  asFsError: (e: unknown) => (e && typeof e === 'object' && 'code' in e ? e : { code: 'io', message: String(e) }),
}))
vi.mock('../events', () => ({ listenProject: vi.fn(async () => () => {}) }))
vi.mock('./project.svelte', () => ({ projectStore: { untrusted: [] } }))

import { editor, onDiskChange, repath } from './editor.svelte'

const disk = (text: string) => ({ text, mtimeMs: 2 })

describe('onDiskChange', () => {
  it('ricarica se la tab non ha modifiche locali e il testo su disco differisce', () => {
    expect(onDiskChange({ dirty: false, baseText: 'a' }, disk('b'))).toBe('reload')
  })
  it('chiede se la tab ha modifiche locali e il disco è cambiato', () => {
    expect(onDiskChange({ dirty: true, baseText: 'a' }, disk('b'))).toBe('prompt')
  })
  it('non fa nulla se il testo su disco è uguale', () => {
    expect(onDiskChange({ dirty: false, baseText: 'a' }, disk('a'))).toBe('none')
    expect(onDiskChange({ dirty: true, baseText: 'a' }, disk('a'))).toBe('none')
  })
})

describe('repath', () => {
  const tab = (path: string, diff: unknown = null) => ({ path, name: path.split('/').pop()!, diff })
  it('sposta la tab del file rinominato', () => {
    const r = repath([tab('/p/a.ts'), tab('/p/b.ts')], '/p/a.ts', '/p/c.ts')
    expect(r.map((t) => [t.path, t.name])).toEqual([['/p/c.ts', 'c.ts'], ['/p/b.ts', 'b.ts']])
  })
  it('sposta i file dentro una cartella rinominata, non i prefissi simili', () => {
    const r = repath([tab('/p/src/x.ts'), tab('/p/src2/y.ts')], '/p/src', '/p/lib/src')
    expect(r.map((t) => t.path)).toEqual(['/p/lib/src/x.ts', '/p/src2/y.ts'])
  })
})

describe('editor tabs with unsaved changes', () => {
  beforeEach(() => {
    for (const t of [...editor.tabs]) editor.close(t.key)
    fsWrite.mockClear()
  })
  const dirtyFile = async (path: string) => {
    await editor.open(path)
    const t = editor.tabs.find((x) => x.key === path)!
    editor.bind(t.id, { snapshot: () => ({ text: 'mio', commit: () => editor.setDirty(t.id, false) }) })
    editor.setDirty(t.id, true)
    return t
  }

  it('"Apri diff" opens beside a dirty file tab instead of replacing it', async () => {
    const t = await dirtyFile('/p/a.ts')
    editor.openDiff('/p/a.ts', 'prima', 'dopo')
    expect(editor.tabs.map((x) => x.key)).toEqual(['/p/a.ts', 'diff:/p/a.ts'])
    expect(editor.activeTab?.diff).toEqual({ before: 'prima', after: 'dopo' })
    expect(editor.tabs.find((x) => x.id === t.id)?.dirty).toBe(true)
    expect(editor.dirtyCount).toBe(1)
    editor.openDiff('/p/a.ts', 'x', 'y') // un solo diff per file
    expect(editor.tabs).toHaveLength(2)
  })

  it('closing a dirty tab asks first: Annulla keeps it, Scarta closes it', async () => {
    await dirtyFile('/p/a.ts')
    editor.requestClose('/p/a.ts')
    expect(editor.closing).toBe('/p/a.ts')
    expect(editor.tabs).toHaveLength(1)
    await editor.resolveClose('cancel')
    expect(editor.closing).toBeNull()
    expect(editor.tabs).toHaveLength(1)
    editor.requestClose('/p/a.ts')
    await editor.resolveClose('discard')
    expect(editor.tabs).toHaveLength(0)
    expect(fsWrite).not.toHaveBeenCalled()
  })

  it('Salva writes the file then closes; a clean tab closes at once', async () => {
    await dirtyFile('/p/a.ts')
    editor.requestClose('/p/a.ts')
    await editor.resolveClose('save')
    expect(fsWrite).toHaveBeenCalledWith('/p/a.ts', 'mio', 1)
    expect(editor.tabs).toHaveLength(0)
    await editor.open('/p/b.ts')
    editor.requestClose('/p/b.ts')
    expect(editor.closing).toBeNull()
    expect(editor.tabs).toHaveLength(0)
  })

  it('a failed save keeps the tab and the bar', async () => {
    await dirtyFile('/p/a.ts')
    fsWrite.mockRejectedValueOnce({ code: 'conflict', message: 'cambiato' })
    editor.requestClose('/p/a.ts')
    await editor.resolveClose('save')
    expect(editor.tabs).toHaveLength(1)
    expect(editor.closing).toBe('/p/a.ts')
  })
})
