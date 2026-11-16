import { describe, expect, it } from 'vitest'
import { parseConflicts, resolve, resolveEdit, type Conflict, type Pick } from './conflicts'

const two = ['a', '<<<<<<< HEAD', 'mia1', '=======', 'sua1', 'sua2', '>>>>>>> feature', 'b', '<<<<<<< HEAD', 'x', '=======', 'y', '>>>>>>> feature', 'c', ''].join('\n')

describe('parseConflicts', () => {
  it('parses_two_conflicts', () => {
    expect(parseConflicts(two)).toEqual([
      { start: 2, mid: 4, end: 7, ours: 'mia1', theirs: 'sua1\nsua2' },
      { start: 9, mid: 11, end: 13, ours: 'x', theirs: 'y' },
    ])
  })

  it('parses_diff3_base', () => {
    const t = ['<<<<<<< HEAD', 'mia', '||||||| base', 'vecchia', '=======', 'sua', '>>>>>>> feature', ''].join('\n')
    expect(parseConflicts(t)).toEqual([{ start: 1, mid: 5, end: 7, base: 3, ours: 'mia', theirs: 'sua' }])
  })

  it('ignores_unbalanced_markers', () => {
    expect(parseConflicts('<<<<<<< HEAD\nmia\n=======\nsua\n')).toEqual([])
    expect(parseConflicts('a\n=======\nb\n>>>>>>> x\n')).toEqual([])
    expect(parseConflicts('titolo\n=======\n')).toEqual([])
    // un marcatore iniziale riaperto scarta quello senza fine
    expect(parseConflicts('<<<<<<< a\nx\n<<<<<<< b\nm\n=======\ns\n>>>>>>> b\n')).toEqual([{ start: 3, mid: 5, end: 7, ours: 'm', theirs: 's' }])
  })
})

describe('resolve', () => {
  it('resolve_each_pick', () => {
    const c = parseConflicts(two)[0]
    expect(resolve(two, c, 'ours')).toBe(['a', 'mia1', 'b', '<<<<<<< HEAD', 'x', '=======', 'y', '>>>>>>> feature', 'c', ''].join('\n'))
    expect(resolve(two, c, 'theirs')).toBe(['a', 'sua1', 'sua2', 'b', '<<<<<<< HEAD', 'x', '=======', 'y', '>>>>>>> feature', 'c', ''].join('\n'))
    expect(resolve(two, c, 'both')).toBe(['a', 'mia1', 'sua1', 'sua2', 'b', '<<<<<<< HEAD', 'x', '=======', 'y', '>>>>>>> feature', 'c', ''].join('\n'))
  })

  it('resolve_keeps_diff3_pick_without_base', () => {
    const t = ['<<<<<<< HEAD', 'mia', '||||||| base', 'vecchia', '=======', 'sua', '>>>>>>> f', ''].join('\n')
    const c = parseConflicts(t)[0]
    expect(resolve(t, c, 'both')).toBe('mia\nsua\n')
  })

  it('resolve_keeps_line_endings (CRLF)', () => {
    const t = ['a', '<<<<<<< HEAD', 'mia', '=======', 'sua', '>>>>>>> f', 'z', ''].join('\r\n')
    const cs = parseConflicts(t)
    expect(cs).toEqual([{ start: 2, mid: 4, end: 6, ours: 'mia', theirs: 'sua' }])
    expect(resolve(t, cs[0], 'both')).toBe('a\r\nmia\r\nsua\r\nz\r\n')
    expect(resolve(t, cs[0], 'theirs')).toBe('a\r\nsua\r\nz\r\n')
  })
})

/** Applica una modifica `resolveEdit` come farebbe Monaco (riga/colonna 1-based, a capo \n o \r\n). */
function apply(text: string, c: Conflict, pick: Pick) {
  const e = resolveEdit(text, c, pick)
  const ls = text.match(/[^\n]*\n|[^\n]+$/g) ?? []
  const at = (line: number, col: number) => ls.slice(0, line - 1).join('').length + col - 1
  return text.slice(0, at(e.startLine, 1)) + e.text + text.slice(at(e.endLine, e.endColumn))
}

describe('edge cases', () => {
  it('conflict at the very start of the file', () => {
    const t = '<<<<<<< a\nx\n=======\ny\n>>>>>>> b\nresto\n'
    const c = parseConflicts(t)
    expect(c).toEqual([{ start: 1, mid: 3, end: 5, ours: 'x', theirs: 'y' }])
    expect(resolve(t, c[0], 'theirs')).toBe('y\nresto\n')
  })

  it('conflict at the end without a trailing newline keeps the file without one', () => {
    const t = 'a\n<<<<<<< a\nx\n=======\ny\n>>>>>>> b'
    const c = parseConflicts(t)[0]
    expect(resolve(t, c, 'ours')).toBe('a\nx')
    expect(resolve(t, c, 'both')).toBe('a\nx\ny')
    expect(resolve(t, c, 'theirs')).toBe('a\ny')
  })

  it('empty sides', () => {
    const t = 'a\n<<<<<<< a\n=======\ny\n>>>>>>> b\nz\n'
    const c = parseConflicts(t)[0]
    expect(c.ours).toBe('')
    expect(resolve(t, c, 'ours')).toBe('a\nz\n')
    expect(resolve(t, c, 'both')).toBe('a\ny\nz\n')
    const u = 'a\n<<<<<<< a\nx\n=======\n>>>>>>> b\nz\n'
    expect(resolve(u, parseConflicts(u)[0], 'theirs')).toBe('a\nz\n')
  })

  it('CRLF with diff3 base', () => {
    const t = ['a', '<<<<<<< HEAD', 'mia', '||||||| base', 'vecchia', '=======', 'sua', '>>>>>>> f', 'z', ''].join('\r\n')
    const c = parseConflicts(t)
    expect(c).toEqual([{ start: 2, mid: 6, end: 8, base: 4, ours: 'mia', theirs: 'sua' }])
    expect(resolve(t, c[0], 'both')).toBe('a\r\nmia\r\nsua\r\nz\r\n')
  })

  it('a setext underline inside "ours" is taken as the separator (known limit of the marker format)', () => {
    const t = '<<<<<<< a\nTitolo\n=======\nsua\n>>>>>>> b\n'
    expect(parseConflicts(t)[0]).toMatchObject({ ours: 'Titolo', theirs: 'sua' })
  })

  it.each<Pick>(['ours', 'theirs', 'both'])('resolveEdit covers only the conflict lines and matches resolve (%s)', (pick) => {
    for (const t of [two, 'a\n<<<<<<< a\nx\n=======\ny\n>>>>>>> b', '<<<<<<< a\n=======\n>>>>>>> b\n', '<<<<<<< a\r\nx\r\n=======\r\ny\r\n>>>>>>> b\r\nq\r\n']) {
      for (const c of parseConflicts(t)) {
        expect(apply(t, c, pick)).toBe(resolve(t, c, pick))
        const e = resolveEdit(t, c, pick)
        expect(e.startLine).toBe(c.start)
      }
    }
  })
})
