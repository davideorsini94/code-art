import { beforeEach, describe, expect, it, vi } from 'vitest'

const fsRead = vi.fn(async (_p: string) => ({ text: 'disco', mtimeMs: 1 }))
const fsWrite = vi.fn(async (_p: string, _t: string, _m: number | null) => 2)
const sshRead = vi.fn(async (_id: string, _p: string) => ({ text: 'server', version: '5:1' }))
const sshWrite = vi.fn(async (_id: string, _p: string, _t: string, _v: string) => '6:2')
vi.mock('../ipc', () => ({
  sshRead: (id: string, p: string) => sshRead(id, p),
  sshWrite: (id: string, p: string, t: string, v: string) => sshWrite(id, p, t, v),
  fsRead: (p: string) => fsRead(p),
  fsWrite: (p: string, t: string, m: number | null) => fsWrite(p, t, m),
  dbQueryClose: vi.fn(async () => {}),
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

describe('remote tabs', () => {
  beforeEach(() => {
    for (const t of [...editor.tabs]) editor.close(t.key)
    vi.clearAllMocks()
    sshRead.mockImplementation(async () => ({ text: 'server', version: '5:1' }))
    sshWrite.mockImplementation(async () => '6:2')
  })
  const dirtyRemote = async () => {
    await editor.openRemote('s1', '/home/dev/app/a.ts')
    const t = editor.tabs.find((x) => x.key === 'ssh:s1:/home/dev/app/a.ts')!
    editor.bind(t.id, { snapshot: () => ({ text: 'mio', commit: () => editor.setDirty(t.id, false) }) })
    editor.setDirty(t.id, true)
    return t
  }

  it('opens a remote file with its own key, name and text, without touching the local fs', async () => {
    await editor.openRemote('s1', '/home/dev/app/a.ts')
    const t = editor.activeTab!
    expect(t.key).toBe('ssh:s1:/home/dev/app/a.ts')
    expect(t.name).toBe('a.ts')
    expect(t.path).toBe('/home/dev/app/a.ts')
    expect(t.baseText).toBe('server')
    expect(t.remote).toEqual({ serverId: 's1', path: '/home/dev/app/a.ts', version: '5:1' })
    expect(fsRead).not.toHaveBeenCalled()
    await editor.openRemote('s1', '/home/dev/app/a.ts')
    expect(editor.tabs).toHaveLength(1)
  })

  it('shows images and binaries as unavailable previews', async () => {
    await editor.openRemote('s1', '/home/dev/logo.png')
    expect(editor.activeTab!.error).toBe('remote_preview')
    expect(editor.activeTab!.message).toBe('Anteprima non disponibile per i file remoti')
    expect(sshRead).not.toHaveBeenCalled()
  })

  it('saves with the stored version and keeps the new one', async () => {
    const t = await dirtyRemote()
    await editor.save(t.path, t.key)
    expect(sshWrite).toHaveBeenCalledWith('s1', '/home/dev/app/a.ts', 'mio', '5:1')
    expect(t.remote!.version).toBe('6:2')
    expect(t.dirty).toBe(false)
    expect(t.baseText).toBe('mio')
  })

  it('a conflict raises the prompt; overwrite re-reads the version then writes', async () => {
    const t = await dirtyRemote()
    sshWrite.mockRejectedValueOnce({ code: 'conflict', message: 'cambiato' })
    await editor.save(t.path, t.key)
    expect(t.prompt).toBe(true)
    expect(t.dirty).toBe(true)
    sshRead.mockResolvedValueOnce({ text: 'altro', version: '9:9' })
    await editor.keepMine(t.path, t.key)
    expect(sshWrite).toHaveBeenLastCalledWith('s1', '/home/dev/app/a.ts', 'mio', '9:9')
    expect(t.prompt).toBe(false)
    expect(t.dirty).toBe(false)
  })

  it('reload discards local edits and takes the server text; dismiss keeps them', async () => {
    const t = await dirtyRemote()
    t.prompt = true
    editor.dismissPrompt(t.key)
    expect(t.prompt).toBe(false)
    expect(t.dirty).toBe(true)
    sshRead.mockResolvedValueOnce({ text: 'nuovo', version: '7:7' })
    await editor.reload(t.path, t.key)
    expect(t.baseText).toBe('nuovo')
    expect(t.remote!.version).toBe('7:7')
    expect(t.dirty).toBe(false)
  })

  it('an io error (e.g. dropped connection) keeps the tab dirty with a readable message', async () => {
    const t = await dirtyRemote()
    sshWrite.mockRejectedValueOnce({ code: 'io', message: 'Connessione chiusa' })
    await editor.save(t.path, t.key)
    expect(t.dirty).toBe(true)
    expect(t.prompt).toBe(false)
    expect(t.message).toBe('Connessione chiusa')
  })

  it('local fs events and renames ignore remote tabs', async () => {
    await editor.openRemote('s1', '/home/dev/app/a.ts')
    fsRead.mockClear()
    await editor.applyChanged(['/home/dev/app/a.ts'])
    expect(fsRead).not.toHaveBeenCalled()
    editor.repath('/home/dev/app', '/x')
    expect(editor.tabs[0].path).toBe('/home/dev/app/a.ts')
    editor.closeUnder('/home/dev/app')
    expect(editor.tabs).toHaveLength(1)
  })

  it('two saves in a row do not overlap: the second uses the new version (no false conflict)', async () => {
    const t = await dirtyRemote()
    let release!: () => void
    sshWrite.mockImplementationOnce(
      (_id, _p, _t, v) =>
        new Promise((res, rej) => {
          release = () => (v === '5:1' ? res('6:2') : rej({ code: 'conflict', message: 'cambiato' }))
        }),
    )
    sshWrite.mockImplementation(async (_id, _p, _t, v) => {
      if (v !== '6:2') throw { code: 'conflict', message: 'cambiato' }
      return '7:3'
    })
    const first = editor.save(t.path, t.key)
    const second = editor.save(t.path, t.key)
    const third = editor.save(t.path, t.key)
    await Promise.resolve()
    expect(sshWrite).toHaveBeenCalledTimes(1)
    release()
    await Promise.all([first, second, third])
    expect(sshWrite).toHaveBeenCalledTimes(2) // la terza si è unita alla seconda
    expect(sshWrite.mock.calls.map((c) => c[3])).toEqual(['5:1', '6:2'])
    expect(t.prompt).toBe(false)
    expect(t.remote!.version).toBe('7:3')
  })

  it('local saves are serialized too', async () => {
    await editor.open('/p/z.ts')
    const t = editor.tabs.find((x) => x.key === '/p/z.ts')!
    editor.bind(t.id, { snapshot: () => ({ text: 'mio', commit: () => {} }) })
    let release!: () => void
    fsWrite.mockImplementationOnce(() => new Promise((res) => (release = () => res(5))))
    fsWrite.mockImplementation(async (_p, _t, m) => (m === 5 ? 6 : Promise.reject({ code: 'conflict' })))
    const a = editor.save(t.path)
    const b = editor.save(t.path)
    await Promise.resolve()
    expect(fsWrite).toHaveBeenCalledTimes(1)
    release()
    await Promise.all([a, b])
    expect(fsWrite.mock.calls.map((c) => c[2])).toEqual([1, 5])
    expect(t.prompt).toBe(false)
    fsWrite.mockImplementation(async () => 2)
  })

  it('a rename on the server moves the open tabs of that server only', async () => {
    await editor.openRemote('s1', '/home/dev/app/a.ts')
    await editor.openRemote('s1', '/home/dev/app2/b.ts')
    await editor.openRemote('s2', '/home/dev/app/a.ts')
    const active = editor.active
    editor.repathRemote('s1', '/home/dev/app', '/home/dev/lib')
    expect(editor.tabs.map((t) => t.key)).toEqual(['ssh:s1:/home/dev/lib/a.ts', 'ssh:s1:/home/dev/app2/b.ts', 'ssh:s2:/home/dev/app/a.ts'])
    expect(editor.tabs[0].remote).toEqual({ serverId: 's1', path: '/home/dev/lib/a.ts', version: '5:1' })
    expect(editor.tabs[0].path).toBe('/home/dev/lib/a.ts')
    expect(editor.active).toBe(active)
    editor.repathRemote('s1', '/home/dev/lib/a.ts', '/home/dev/lib/c.ts')
    expect(editor.tabs[0].name).toBe('c.ts')
    editor.active = editor.tabs[0].key
    const t = editor.tabs[0]
    editor.bind(t.id, { snapshot: () => ({ text: 'mio', commit: () => {} }) })
    await editor.save(t.path, t.key)
    expect(sshWrite).toHaveBeenLastCalledWith('s1', '/home/dev/lib/c.ts', 'mio', '5:1')
  })

  it('a delete on the server keeps the tab text, dirty, and saving re-creates the file', async () => {
    const t = await dirtyRemote()
    editor.setDirty(t.id, false)
    await editor.openRemote('s1', '/home/dev/other.ts')
    editor.markRemoteRemoved('s1', '/home/dev/app')
    expect(t.dirty).toBe(true)
    expect(t.remote!.version).toBe('')
    expect(t.notice).toMatch(/Eliminato sul server/)
    expect(editor.tabs.find((x) => x.path === '/home/dev/other.ts')!.notice).toBeUndefined()
    expect(editor.tabs).toHaveLength(2)
    await editor.save(t.path, t.key)
    expect(sshWrite).toHaveBeenLastCalledWith('s1', '/home/dev/app/a.ts', 'mio', '')
    expect(t.notice).toBeUndefined()
    expect(t.dirty).toBe(false)
  })
})

describe('schede query', () => {
  it('chiave db:<connessione>:<n>, titolo tabella o "Query n", mai modificata', () => {
    editor.tabs = []
    editor.openQuery({ connectionId: 'c1', kind: 'postgres', schema: 'public', table: 'users' })
    editor.openQuery({ connectionId: 'c1', kind: 'postgres' })
    editor.openQuery({ connectionId: 'c1', kind: 'postgres' })
    expect(editor.tabs.map((t) => [t.key, t.name, t.dirty])).toEqual([
      ['db:c1:1', 'users', false],
      ['db:c1:2', 'Query 2', false],
      ['db:c1:3', 'Query 3', false],
    ])
    expect(editor.active).toBe('db:c1:3')
    editor.openQuery({ connectionId: 'c1', kind: 'postgres', schema: 'public', table: 'users' })
    expect(editor.tabs).toHaveLength(3) // la stessa tabella riporta in primo piano
    expect(editor.active).toBe('db:c1:1')
    editor.requestClose('db:c1:2')
    expect(editor.tabs).toHaveLength(2)
    editor.tabs = []
  })
  it('chiudere la scheda chiude il risultato', async () => {
    const ipc = await import('../ipc')
    editor.tabs = []
    editor.openQuery({ connectionId: 'c9', kind: 'mongo', database: 'd', collection: 'c' })
    const key = editor.active!
    editor.setQueryResult(key, 42)
    editor.close(key)
    expect(ipc.dbQueryClose).toHaveBeenCalledWith(42)
  })
})
