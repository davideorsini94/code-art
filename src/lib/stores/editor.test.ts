import { describe, expect, it } from 'vitest'
import { onDiskChange, repath } from './editor.svelte'

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
