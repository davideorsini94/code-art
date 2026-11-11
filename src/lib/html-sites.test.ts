// Ogni `{@html}` dei componenti deve passare da `renderMarkdown` (markdown non fidato -> HTML sanificato).
// Un nuovo `{@html}` su dati non sanificati fa fallire questo test.
import { readdirSync, readFileSync, statSync } from 'node:fs'
import { join } from 'node:path'
import { describe, expect, it } from 'vitest'

const ROOT = join(__dirname, '..')

function sources(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const p = join(dir, name)
    if (statSync(p).isDirectory()) return sources(p)
    return /\.(svelte|ts|js)$/.test(name) && !/\.test\.(ts|js)$/.test(name) ? [p] : []
  })
}

describe('{@html} sites', () => {
  it('only render sanitized markdown', () => {
    const sites = sources(ROOT).flatMap((file) =>
      [...readFileSync(file, 'utf8').matchAll(/\{@html\s+([^}]*)\}/g)].map((m) => ({ file: file.slice(ROOT.length + 1), expr: m[1] })),
    )
    expect(sites.length).toBeGreaterThan(0)
    for (const { file, expr } of sites) expect(expr, file).toMatch(/^renderMarkdown\([^()]*\)\s*$/)
  })

  it('no innerHTML or outerHTML assignments', () => {
    for (const file of sources(ROOT)) {
      const text = readFileSync(file, 'utf8')
      expect(/\.(inner|outer)HTML\s*=|insertAdjacentHTML\(|document\.write\(/.test(text), file).toBe(false)
    }
  })
})
