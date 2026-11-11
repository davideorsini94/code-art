import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { absolutePath, commands, createLatest, fuzzyRank, segments } from './search'

describe('fuzzyRank', () => {
  it('matches a subsequence and ranks the best file first', () => {
    expect(fuzzyRank('invsvc', ['src/pdf.ts', 'src/invoice.service.ts'], 10)[0]).toBe('src/invoice.service.ts')
  })
  it('drops non-matching items and respects the limit', () => {
    expect(fuzzyRank('zzz', ['a.ts', 'b.ts'], 10)).toEqual([])
    expect(fuzzyRank('a', ['a1', 'a2', 'a3'], 2)).toHaveLength(2)
  })
  it('prefers a match on the file name over one in the path', () => {
    expect(fuzzyRank('pdf', ['pdfs/readme.md', 'src/pdf.ts'], 10)).toEqual(['src/pdf.ts', 'pdfs/readme.md'])
  })
  it('prefers contiguous matches and is case-insensitive', () => {
    expect(fuzzyRank('ABC', ['a_b_c.ts', 'abc.ts'], 10)[0]).toBe('abc.ts')
  })
  it('returns the first items for an empty query', () => {
    expect(fuzzyRank('', ['x', 'y', 'z'], 2)).toEqual(['x', 'y'])
  })
})

describe('segments', () => {
  it('splits text into plain and highlighted spans without losing characters', () => {
    const s = segments('<b>abc</b>', [3, 4])
    expect(s.map((x) => x.t).join('')).toBe('<b>abc</b>')
    expect(s.filter((x) => x.hit).map((x) => x.t).join('')).toBe('ab')
  })
})

describe('absolutePath', () => {
  it('joins the single folder, and resolves the folder-name prefix with several', () => {
    expect(absolutePath(['/Users/dev/a'], 'src/x.ts')).toBe('/Users/dev/a/src/x.ts')
    expect(absolutePath(['/Users/dev/a', '/Users/dev/b'], 'b/src/x.ts')).toBe('/Users/dev/b/src/x.ts')
  })
})

describe('commands', () => {
  it('has the minimal registry with unique ids', () => {
    const ids = commands.map((c) => c.id)
    expect(new Set(ids).size).toBe(ids.length)
    expect(commands.map((c) => c.title)).toEqual(
      expect.arrayContaining([
        'Nuova conversazione',
        'Apri cartella',
        'Aggiungi cartella al progetto',
        'Rinomina progetto',
        'Mostra/nascondi terminale',
        'Archivia conversazione',
        'Elimina conversazione',
        'Cambia modalità permessi predefinita',
      ]),
    )
  })
})

describe('createLatest', () => {
  beforeEach(() => vi.useFakeTimers())
  afterEach(() => vi.useRealTimers())

  it('debounces and delivers only the latest query', async () => {
    const calls: string[] = []
    const got: string[] = []
    const resolvers: Record<string, (v: string) => void> = {}
    const l = createLatest<string>(
      (q) => {
        calls.push(q)
        return new Promise((r) => (resolvers[q] = r))
      },
      (_q, r) => got.push(r),
      200,
    )
    l.query('ab')
    await vi.advanceTimersByTimeAsync(100)
    l.query('abc')
    await vi.advanceTimersByTimeAsync(200)
    expect(calls).toEqual(['abc']) // 'ab' mai partita
    l.query('abcd')
    await vi.advanceTimersByTimeAsync(200)
    resolvers['abcd']('nuovo')
    resolvers['abc']('vecchio') // arriva dopo ma è superato
    await vi.advanceTimersByTimeAsync(0)
    expect(got).toEqual(['nuovo'])
  })

  it('cancel drops pending and in-flight results', async () => {
    const got: string[] = []
    let res!: (v: string) => void
    const l = createLatest<string>(() => new Promise((r) => (res = r)), (_q, r) => got.push(r), 200)
    l.query('abc')
    await vi.advanceTimersByTimeAsync(200)
    l.cancel()
    res('x')
    await vi.advanceTimersByTimeAsync(0)
    expect(got).toEqual([])
  })
})
