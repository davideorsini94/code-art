import { describe, expect, it } from 'vitest'
import { buildNewMenu, normalizeCustom, onlyPrimary, parentDir, slotToIndex } from './menu'

const folders = ['/Users/dev/app', '/Users/dev/lib']

describe('parentDir', () => {
  it('returns the containing folder', () => {
    expect(parentDir('/Users/dev/app/src/a.ts')).toBe('/Users/dev/app/src')
  })
  it('returns root for a top-level file', () => {
    expect(parentDir('/a.ts')).toBe('/')
  })
})

describe('buildNewMenu', () => {
  it('lists project folders, then repos without duplicates, then the file folder and "other"', () => {
    const items = buildNewMenu({ folders, repos: ['/Users/dev/app', '/Users/dev/lib/core'], activeFile: '/Users/dev/app/src/a.ts' })
    expect(items.map((i) => [i.kind, i.path])).toEqual([
      ['folder', '/Users/dev/app'],
      ['folder', '/Users/dev/lib'],
      ['repo', '/Users/dev/lib/core'],
      ['file', '/Users/dev/app/src'],
      ['other', undefined],
    ])
  })
  it('omits the file entry without an active file', () => {
    const items = buildNewMenu({ folders, repos: [], activeFile: null })
    expect(items.some((i) => i.kind === 'file')).toBe(false)
  })
  it('omits the file entry when its folder is already listed', () => {
    const items = buildNewMenu({ folders: ['/Users/dev/app'], repos: [], activeFile: '/Users/dev/app/a.ts' })
    expect(items.some((i) => i.kind === 'file')).toBe(false)
  })
})

describe('onlyPrimary', () => {
  it('is true when the only choice is the primary folder', () => {
    expect(onlyPrimary(buildNewMenu({ folders: ['/Users/dev/app'], repos: ['/Users/dev/app'], activeFile: null }))).toBe(true)
  })
  it('is false with another repo, file folder or several folders', () => {
    expect(onlyPrimary(buildNewMenu({ folders: ['/Users/dev/app'], repos: ['/Users/dev/app/x'], activeFile: null }))).toBe(false)
    expect(onlyPrimary(buildNewMenu({ folders, repos: [], activeFile: null }))).toBe(false)
    expect(onlyPrimary(buildNewMenu({ folders: ['/Users/dev/app'], repos: [], activeFile: '/Users/dev/app/src/a.ts' }))).toBe(false)
  })
})

describe('normalizeCustom', () => {
  it('trims and maps empty to null', () => {
    expect(normalizeCustom('  api  ')).toBe('api')
    expect(normalizeCustom('   ')).toBeNull()
    expect(normalizeCustom('')).toBeNull()
  })
})

describe('slotToIndex', () => {
  it('converts an insertion slot into the final index', () => {
    expect(slotToIndex(0, 2)).toBe(0)
    expect(slotToIndex(0, 3)).toBe(0)
    expect(slotToIndex(2, 0)).toBe(1)
    expect(slotToIndex(2, 3)).toBe(2)
    expect(slotToIndex(2, 1)).toBe(1)
  })
})
