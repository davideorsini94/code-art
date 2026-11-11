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
