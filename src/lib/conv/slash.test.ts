import { describe, expect, it } from 'vitest'
import { completeSlash, slashMatches } from './slash'

const cmds = ['compact', 'clear', 'code-review', 'commit']

describe('slashMatches', () => {
  it('matches by prefix', () => {
    expect(slashMatches('/co', 3, cmds)).toEqual(['compact', 'code-review', 'commit'])
  })
  it('is inactive when the slash is not at the start of the message', () => {
    expect(slashMatches('ciao /co', 8, cmds)).toEqual([])
    expect(slashMatches(' /co', 4, cmds)).toEqual([])
  })
  it('is inactive once the cursor has moved past the command word', () => {
    expect(slashMatches('/compact foo', 12, cmds)).toEqual([])
    expect(slashMatches('/co', 0, cmds)).toEqual([])
  })
  it('uses only the text before the cursor', () => {
    expect(slashMatches('/commit', 3, cmds)).toEqual(['compact', 'code-review', 'commit'])
  })
  it('lists prefix matches before substring matches', () => {
    expect(slashMatches('/re', 3, ['pre-release', 'review', 'refactor', 'core'])).toEqual(['review', 'refactor', 'pre-release', 'core'])
  })
  it('returns at most 8 results', () => {
    const many = Array.from({ length: 20 }, (_, i) => `cmd${i}`)
    expect(slashMatches('/c', 2, many)).toHaveLength(8)
    expect(slashMatches('/', 1, many)).toHaveLength(8)
  })
  it('is case-insensitive and de-duplicates', () => {
    expect(slashMatches('/CO', 3, ['compact', 'compact'])).toEqual(['compact'])
  })
  it('does not trigger on non-letter input', () => {
    expect(slashMatches('/1', 2, ['1x'])).toEqual([])
    expect(slashMatches('/a/b', 4, ['a'])).toEqual([])
  })
})

describe('completeSlash', () => {
  it('replaces the typed word and adds a space, keeping the rest', () => {
    expect(completeSlash('/co bar', 3, 'compact')).toEqual({ text: '/compact  bar', cursor: 9 })
    expect(completeSlash('/co', 3, 'compact')).toEqual({ text: '/compact ', cursor: 9 })
  })
})
