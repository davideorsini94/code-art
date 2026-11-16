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

  it('an editable diff reads the file mtime, tracks dirty and saves the modified side through the normal path', async () => {
    await editor.openDiff('/p/a.ts', 'indice', 'x', { editable: true })
    const t = editor.activeTab!
    expect(t.key).toBe('diff:/p/a.ts')
    expect(t.diff).toEqual({ before: 'indice', after: 'disco', editable: true }) // il testo di `after` è quello letto dal disco
    expect(t.mtimeMs).toBe(1)
    editor.bind(t.id, { snapshot: () => ({ text: 'modificato', commit: () => editor.setDirty(t.id, false) }) })
    editor.setDirty(t.id, true)
    expect(editor.dirtyCount).toBe(1)
    await editor.save('/p/a.ts', t.key)
    expect(fsWrite).toHaveBeenCalledWith('/p/a.ts', 'modificato', 1)
    expect(t.mtimeMs).toBe(2)
    expect(t.dirty).toBe(false)
  })

  it('a conflicting save of an editable diff shows the "cambiato su disco" prompt, and Ricarica refreshes the modified side', async () => {
    await editor.openDiff('/p/a.ts', 'i', 'x', { editable: true })
    const t = editor.activeTab!
    editor.bind(t.id, { snapshot: () => ({ text: 'mio', commit: () => {} }) })
    fsWrite.mockRejectedValueOnce({ code: 'conflict', message: 'cambiato' })
    await editor.save('/p/a.ts', t.key)
    expect(t.prompt).toBe(true)
    fsRead.mockResolvedValueOnce({ text: 'nuovo', mtimeMs: 5 })
    const rev = t.rev
    await editor.reload('/p/a.ts', t.key)
    expect(t.diff?.after).toBe('nuovo')
    expect(t.rev).toBe(rev + 1)
    expect(t.prompt).toBe(false)
  })

  it('a read-only diff (staged, or unreadable file) is never saved', async () => {
    await editor.openDiff('/p/a.ts', 'i', 'x') // in stage
    const ro = editor.activeTab!
    editor.bind(ro.id, { snapshot: () => ({ text: 'mio', commit: () => {} }) })
    await editor.save('/p/a.ts', ro.key)
    fsRead.mockRejectedValueOnce({ code: 'not_text', message: 'x' })
    await editor.openDiff('/p/b.ts', 'i', 'x', { editable: true })
    expect(editor.activeTab?.diff?.editable).toBeUndefined()
    await editor.save('/p/b.ts', editor.activeTab!.key)
    expect(fsWrite).not.toHaveBeenCalled()
  })

  it('an external change reloads a clean editable diff and does not touch a read-only one', async () => {
    await editor.openDiff('/p/a.ts', 'i', 'x', { editable: true })
    const t = editor.activeTab!
    fsRead.mockResolvedValueOnce({ text: 'esterno', mtimeMs: 9 })
    await editor.applyChanged(['/p/a.ts'])
    expect(t.diff?.after).toBe('esterno')
    expect(t.mtimeMs).toBe(9)
  })

  it('"Apri diff" opens beside a dirty file tab instead of replacing it', async () => {
    const t = await dirtyFile('/p/a.ts')
    await editor.openDiff('/p/a.ts', 'prima', 'dopo')
    expect(editor.tabs.map((x) => x.key)).toEqual(['/p/a.ts', 'diff:/p/a.ts'])
    expect(editor.activeTab?.diff).toEqual({ before: 'prima', after: 'dopo' })
    expect(editor.tabs.find((x) => x.id === t.id)?.dirty).toBe(true)
    expect(editor.dirtyCount).toBe(1)
    await editor.openDiff('/p/a.ts', 'x', 'y') // un solo diff per file
    expect(editor.tabs).toHaveLength(2)
  })

  it('reopening a diff whose modified side has unsaved edits keeps those edits (activates the tab)', async () => {
    await editor.openDiff('/p/a.ts', 'indice', 'x', { editable: true })
    const t = editor.activeTab!
    editor.setDirty(t.id, true)
    await editor.openDiff('/p/b.ts', 'i', 'x')
    await editor.openDiff('/p/a.ts', 'indice2', 'x', { editable: true })
    expect(editor.tabs.filter((x) => x.key === 'diff:/p/a.ts')).toHaveLength(1)
    expect(editor.activeTab?.id).toBe(t.id)
    expect(editor.activeTab?.dirty).toBe(true)
    // pulito: si sostituisce con i dati nuovi
    editor.setDirty(t.id, false)
    await editor.openDiff('/p/a.ts', 'indice3', 'x', { editable: true })
    expect(editor.activeTab?.diff?.before).toBe('indice3')
  })

  it('a disk change under unsaved (e.g. conflict-resolution) edits asks instead of overwriting', async () => {
    const t = await dirtyFile('/p/a.ts')
    fsRead.mockResolvedValueOnce({ text: 'rebase in corso', mtimeMs: 7 })
    await editor.applyChanged(['/p/a.ts'])
    expect(t.prompt).toBe(true)
    expect(t.baseText).not.toBe('rebase in corso')
    expect(t.dirty).toBe(true)
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

describe('openAt', () => {
  it('apre il file e chiede di mostrare riga e colonna sulla sua scheda', async () => {
    await editor.openAt('/p/src/a.ts', 12, 5)
    expect(editor.active).toBe('/p/src/a.ts')
    expect(editor.reveal).toEqual({ key: editor.active, line: 12, col: 5 })
  })
})
