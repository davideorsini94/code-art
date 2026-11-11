import { describe, expect, it } from 'vitest'
import { isVideo, mimeFor, resolveMarkdownImage, viewerFor } from './viewers'

describe('viewerFor', () => {
  it.each([
    ['README.md', 'markdown'], ['a.MARKDOWN', 'markdown'], ['x.mdx', 'markdown'],
    ['a.png', 'image'], ['a.JPG', 'image'], ['a.jpeg', 'image'], ['a.gif', 'image'], ['a.webp', 'image'],
    ['a.bmp', 'image'], ['a.ico', 'image'], ['a.avif', 'image'], ['logo.SVG', 'image'],
    ['doc.pdf', 'pdf'], ['doc.PDF', 'pdf'],
    ['a.mp3', 'media'], ['a.wav', 'media'], ['a.m4a', 'media'], ['a.ogg', 'media'], ['a.mp4', 'media'], ['a.MOV', 'media'], ['a.webm', 'media'],
    ['main.rs', 'text'], ['Makefile', 'text'], ['.gitignore', 'text'], ['.md', 'text'], ['noext', 'text'], ['a.tar.gz', 'binary'],
    ['app.zip', 'binary'], ['lib.dylib', 'binary'], ['font.woff2', 'binary'],
  ])('%s -> %s', (name, v) => expect(viewerFor(name)).toBe(v))
})

describe('mimeFor', () => {
  it('uses a fixed table, case-insensitive', () => {
    expect(mimeFor('a.png')).toBe('image/png')
    expect(mimeFor('a.JPG')).toBe('image/jpeg')
    expect(mimeFor('a.svg')).toBe('image/svg+xml')
    expect(mimeFor('a.pdf')).toBe('application/pdf')
    expect(mimeFor('a.mp3')).toBe('audio/mpeg')
    expect(mimeFor('a.mov')).toBe('video/quicktime')
    expect(mimeFor('a.webm')).toBe('video/webm')
    expect(mimeFor('a.html')).toBeNull()
    expect(mimeFor('a.png.exe')).toBeNull()
  })
  it('tells videos from audio', () => {
    expect(isVideo('a.mp4')).toBe(true)
    expect(isVideo('a.m4a')).toBe(false)
  })
})

describe('resolveMarkdownImage', () => {
  const roots = ['/Users/dev/app']
  const md = '/Users/dev/app/docs/guide.md'
  it('accepts relative paths inside the project', () => {
    expect(resolveMarkdownImage('img/a.png', md, roots)).toBe('/Users/dev/app/docs/img/a.png')
    expect(resolveMarkdownImage('./a.png', md, roots)).toBe('/Users/dev/app/docs/a.png')
    expect(resolveMarkdownImage('../shot.png', md, roots)).toBe('/Users/dev/app/shot.png')
    expect(resolveMarkdownImage('my%20pic.png?x=1#y', md, roots)).toBe('/Users/dev/app/docs/my pic.png')
  })
  it('rejects remote, scheme, protocol-relative and absolute paths', () => {
    for (const h of ['http://example.com/a.png', 'https://example.com/a.png', '//x/a.png', '/abs/a.png', 'data:image/png;base64,AAAA', 'file:///etc/a.png', 'javascript:alert(1)', '\\\\x\\a.png', ''])
      expect(resolveMarkdownImage(h, md, roots), h).toBeNull()
  })
  it('rejects paths escaping the project and non-image files', () => {
    expect(resolveMarkdownImage('../../x.png', md, roots)).toBeNull()
    expect(resolveMarkdownImage('../../../../etc/a.png', md, roots)).toBeNull()
    expect(resolveMarkdownImage('../../app2/a.png', md, roots)).toBeNull()
    expect(resolveMarkdownImage('a.txt', md, roots)).toBeNull()
    expect(resolveMarkdownImage('%2e%2e/%2e%2e/x.png', md, roots)).toBeNull()
  })
})
