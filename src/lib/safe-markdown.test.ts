// @vitest-environment jsdom
import { describe, expect, it } from 'vitest'
import { renderMarkdown } from './safe-markdown'

describe('renderMarkdown', () => {
  it('strips event handlers from raw html', () => {
    const out = renderMarkdown('<img src=x onerror=alert(1)>')
    expect(out).not.toContain('onerror')
    expect(out).not.toContain('<img')
  })
  it('drops script tags', () => {
    expect(renderMarkdown('<script>x</script>')).not.toContain('<script')
    expect(renderMarkdown('ciao <script>alert(1)</script>')).not.toContain('<script')
  })
  it('drops javascript: and data: links', () => {
    expect(renderMarkdown('[a](javascript:alert(1))')).not.toContain('javascript:')
    expect(renderMarkdown('[a](data:text/html;base64,AAAA)')).not.toContain('data:')
    expect(renderMarkdown('[a](file:///etc/passwd)')).not.toContain('file:')
  })
  it('renders a fenced code block', () => {
    const out = renderMarkdown('```ts\nconst a = 1 < 2\n```')
    expect(out).toContain('<pre>')
    expect(out).toContain('<code')
    expect(out).toContain('const a = 1 &lt; 2')
  })
  it('renders safe links with rel noopener noreferrer', () => {
    for (const url of ['https://example.com/a', 'http://example.com']) {
      const out = renderMarkdown(`[x](${url})`)
      expect(out).toContain(`href="${url}"`)
      expect(out).toContain('rel="noopener noreferrer"')
    }
  })
  it('keeps only http(s) links (the opener permission allows nothing else)', () => {
    expect(renderMarkdown('[x](mailto:a@example.com)')).not.toContain('href=')
    expect(renderMarkdown('[x](vbscript:msgbox)')).not.toContain('href=')
  })
  it('does not render raw html as html', () => {
    const out = renderMarkdown('<b>grassetto</b> e <iframe src="https://x"></iframe><form action="/x"><input></form>')
    expect(out).not.toContain('<iframe')
    expect(out).not.toContain('<form')
    expect(out).not.toContain('<input')
  })
  it('drops images (no network requests from the interface)', () => {
    expect(renderMarkdown('![x](https://example.com/a.png)')).not.toContain('<img')
  })
})

describe('renderMarkdown with an image resolver', () => {
  const resolveImage = (href: string) => (href === 'a.png' ? 'blob:codeart/1' : null)
  it('shows resolved images as <img> with only src, alt and title', () => {
    const out = renderMarkdown('![logo](a.png "Titolo")', { resolveImage })
    expect(out).toContain('<img')
    expect(out).toContain('src="blob:codeart/1"')
    expect(out).toContain('alt="logo"')
    expect(out).toContain('title="Titolo"')
    expect(out).not.toMatch(/onerror|onload|style=/)
  })
  it('drops images the resolver rejects, keeping the alt text', () => {
    for (const href of ['https://example.com/a.png', 'data:image/png;base64,AAAA', '//x/a.png', 'b.png']) {
      const out = renderMarkdown(`![alt](${href})`, { resolveImage })
      expect(out).not.toContain('<img')
      expect(out).toContain('alt')
    }
  })
  it('never lets a non-blob src through, even if the resolver returns one', () => {
    for (const bad of ['https://evil.example/x.png', 'javascript:alert(1)', 'data:image/svg+xml,<svg/>', 'file:///etc/passwd']) {
      expect(renderMarkdown('![x](a.png)', { resolveImage: () => bad })).not.toContain('<img')
    }
  })
  it('stays sanitised: raw img, script and javascript links', () => {
    const out = renderMarkdown('<img src=x onerror=alert(1)> <script>alert(1)</script> [a](javascript:alert(1)) [b](blob:codeart/1)', { resolveImage })
    expect(out).not.toMatch(/<img|onerror|<script|javascript:|href="blob:/)
  })
  it('renders task lists and tables', () => {
    const out = renderMarkdown('- [x] fatto\n- [ ] da fare\n\n| a | b |\n|---|---|\n| 1 | 2 |')
    expect(out).toContain('☑')
    expect(out).toContain('☐')
    expect(out).toContain('<table>')
    expect(out).not.toContain('<input')
  })
})

describe('conversation pane rendering (no resolver)', () => {
  it('has no <input> and no images even with task lists and image syntax', () => {
    const out = renderMarkdown('- [x] a\n- [ ] b\n\n![x](img/a.png) ![y](https://example.com/a.png) ![z](blob:test/1)')
    expect(out).not.toContain('<input')
    expect(out).not.toContain('<img')
  })
})
