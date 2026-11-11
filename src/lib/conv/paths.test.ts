import { describe, expect, it } from 'vitest'
import { imageExt, mentionFor, relativeToFolders } from './paths'

describe('paths', () => {
  const folders = ['/Users/dev/app', '/Users/dev/lib/']
  it('makes paths relative to the folder that contains them', () => {
    expect(relativeToFolders('/Users/dev/app/src/a.ts', folders)).toBe('src/a.ts')
    expect(relativeToFolders('/Users/dev/lib/x.ts', folders)).toBe('x.ts')
  })
  it('does not match a sibling folder with the same prefix', () => {
    expect(relativeToFolders('/Users/dev/app2/a.ts', folders)).toBe('/Users/dev/app2/a.ts')
  })
  it('builds @mentions, quoting names with spaces', () => {
    expect(mentionFor('/Users/dev/app/src/a.ts', folders)).toBe('@src/a.ts')
    expect(mentionFor('/Users/dev/app/my file.ts', folders)).toBe('@"my file.ts"')
  })
  it('maps image MIME types to engine extensions', () => {
    expect(imageExt('image/jpeg')).toBe('jpg')
    expect(imageExt('image/svg+xml')).toBeNull()
  })
})
