import { describe, expect, it } from 'vitest'
import { parseConflicts, resolve } from './conflicts'

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
