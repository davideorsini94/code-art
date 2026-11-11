import { readFileSync } from 'node:fs'
import { describe, expect, it } from 'vitest'

const css = readFileSync(new URL('./glass.css', import.meta.url), 'utf8')

describe('glass.css', () => {
  it.each([
    ['--wait', '#ffb454'],
    ['--work', '#5ee0c1'],
    ['--done', '#7d8aa6'],
    ['--accent', '#8fa8ff'],
  ])('definisce %s con il valore %s', (name, value) => {
    expect(css).toMatch(new RegExp(`${name}\\s*:\\s*${value}\\s*;`, 'i'))
  })

  it('.surface-opaque ha alpha >= 0.85', () => {
    const block = css.match(/\.surface-opaque\s*\{([^}]*)\}/)
    expect(block).not.toBeNull()
    const rgba = block![1].match(/background[^:]*:\s*rgba\(([^)]*)\)/)
    expect(rgba).not.toBeNull()
    const alpha = parseFloat(rgba![1].split(',')[3])
    expect(alpha).toBeGreaterThanOrEqual(0.85)
  })

  it('azzera animazioni e transizioni con prefers-reduced-motion', () => {
    const block = css.match(/@media \(prefers-reduced-motion: reduce\)\s*\{([\s\S]*?\})\s*\}/)
    expect(block).not.toBeNull()
    expect(block![1]).toMatch(/animation[^;]*:\s*none/)
    expect(block![1]).toMatch(/transition[^;]*:\s*none/)
  })
})
