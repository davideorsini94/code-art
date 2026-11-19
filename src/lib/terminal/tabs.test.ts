import { describe, expect, it } from 'vitest'
import { folderLabel, moveTab, tabTitle } from './tabs'

const folders = ['/Users/dev/app', '/Users/dev/lib/core']

describe('folderLabel', () => {
  it('uses the last segment of a subfolder', () => {
    expect(folderLabel('/Users/dev/app/src/ui', folders)).toBe('ui')
  })
  it('uses the name of a project folder', () => {
    expect(folderLabel('/Users/dev/lib/core', folders)).toBe('core')
  })
  it('ignores a trailing slash', () => {
    expect(folderLabel('/Users/dev/app/src/', folders)).toBe('src')
    expect(folderLabel('/Users/dev/app/', folders)).toBe('app')
  })
})

describe('tabTitle', () => {
  it('shows folder and branch', () => {
    expect(tabTitle({ cwd: '/Users/dev/app', branch: 'main', custom: null }, folders)).toBe('app · main')
  })
  it('shows only the folder without branch', () => {
    expect(tabTitle({ cwd: '/Users/dev/app', branch: null, custom: null }, folders)).toBe('app')
  })
  it('prefers a custom name', () => {
    expect(tabTitle({ cwd: '/Users/dev/app', branch: 'main', custom: 'server' }, folders)).toBe('server')
  })
  it('treats an empty custom name as automatic', () => {
    expect(tabTitle({ cwd: '/Users/dev/app', branch: 'dev', custom: '' }, folders)).toBe('app · dev')
  })
})

describe('moveTab', () => {
  it('moves forward', () => expect(moveTab([1, 2, 3, 4], 0, 2)).toEqual([2, 3, 1, 4]))
  it('moves backward', () => expect(moveTab([1, 2, 3, 4], 3, 1)).toEqual([1, 4, 2, 3]))
  it('returns an unchanged copy when out of range or same index', () => {
    const l = [1, 2, 3]
    for (const [f, t] of [[-1, 1], [0, 3], [5, 0], [1, 1]]) {
      const r = moveTab(l, f, t)
      expect(r).toEqual(l)
      expect(r).not.toBe(l)
    }
  })
})
