import { readFileSync, readdirSync } from 'node:fs'
import { describe, expect, it } from 'vitest'
import { monacoColors, palette, withAlpha, xtermTheme } from '../lib/theme'

const css = readFileSync(new URL('./glass.css', import.meta.url), 'utf8')
const app = readFileSync(new URL('../app.css', import.meta.url), 'utf8')

function token(name: string): string {
  const m = css.match(new RegExp(`${name}\\s*:\\s*([^;]+);`))
  expect(m, `token ${name}`).not.toBeNull()
  return m![1].trim()
}

function luminance(hex: string): number {
  const c = [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16) / 255)
  const [r, g, b] = c.map((v) => (v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4))
  return 0.2126 * r + 0.7152 * g + 0.0722 * b
}

function contrast(a: string, b: string): number {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x)
  return (hi + 0.05) / (lo + 0.05)
}

const COLOR = /#[0-9a-f]{3,8}\b|rgba?\(|hsla?\(/i

describe('glass.css', () => {
  it.each([
    ['--wait', '#ffb454'],
    ['--work', '#5ee0c1'],
    ['--done', '#7d8aa6'],
    ['--accent', '#8fa8ff'],
  ])('definisce %s con il valore %s', (name, value) => {
    expect(css).toMatch(new RegExp(`${name}\\s*:\\s*${value}\\s*;`, 'i'))
  })

  it.each([
    '--bg', '--surface-1', '--surface-2', '--surface-pop', '--edge', '--edge-soft', '--well', '--panel-shadow',
    '--accent-soft', '--accent-line', '--wait-soft', '--wait-mid', '--wait-strong', '--wait-line', '--err-soft',
  ])('definisce il token %s', (name) => {
    token(name)
  })

  it('le superfici di base sono esadecimali pieni', () => {
    for (const n of ['--bg', '--bg-top', '--surface-1', '--surface-2']) expect(token(n)).toMatch(/^#[0-9a-f]{6}$/i)
  })

  it.each(['--surface-1', '--surface-2', '--bg'])('--txt e --dim superano AA (4.5:1) su %s', (s) => {
    expect(contrast(token('--txt'), token(s))).toBeGreaterThanOrEqual(4.5)
    expect(contrast(token('--dim'), token(s))).toBeGreaterThanOrEqual(4.5)
  })

  it('pannelli ed editor/terminale condividono superficie, bordo e luce', () => {
    const block = css.match(/\.glass,\s*\.surface-opaque\s*\{([^}]*)\}/)
    expect(block).not.toBeNull()
    expect(block![1]).toMatch(/background:\s*var\(--surface-1\)/)
    expect(block![1]).toMatch(/box-shadow:\s*var\(--panel-shadow\)/)
    const strong = css.match(/\.glass-strong\s*\{([^}]*)\}/)
    expect(strong![1]).toMatch(/box-shadow:\s*var\(--panel-shadow\)/)
  })

  it('i colori stanno solo nel blocco :root', () => {
    const outside = css.replace(/\/\*[\s\S]*?\*\//g, '').replace(/:root\s*\{[^}]*\}/, '')
    expect(outside).not.toMatch(COLOR)
  })

  it('azzera animazioni e transizioni con prefers-reduced-motion', () => {
    const block = css.match(/@media \(prefers-reduced-motion: reduce\)\s*\{([\s\S]*?\})\s*\}/)
    expect(block).not.toBeNull()
    expect(block![1]).toMatch(/animation[^;]*:\s*none/)
    expect(block![1]).toMatch(/transition[^;]*:\s*none/)
  })

  it('lo sfondo della pagina è opaco e uniforme (nessuna aurora multicolore)', () => {
    const body = app.match(/\nbody\s*\{([^}]*)\}/)
    expect(body).not.toBeNull()
    const bg = body![1].match(/background:([^;]*);/)![1].trim()
    expect(bg).toMatch(/,\s*var\(--bg\)\s*$/)
    expect(bg).not.toMatch(/radial-gradient/)
    expect(bg).not.toMatch(COLOR)
  })
})

describe('nessun colore scritto a mano fuori dai token', () => {
  const dir = new URL('../components/', import.meta.url)
  const files: (readonly [string, string])[] = readdirSync(dir)
    .filter((f) => f.endsWith('.svelte'))
    .map((f) => [`components/${f}`, readFileSync(new URL(f, dir), 'utf8')] as const)
  files.push(['App.svelte', readFileSync(new URL('../App.svelte', import.meta.url), 'utf8')])
  files.push(['app.css', app])

  it.each(files)('%s usa solo variabili', (_name, src) => {
    const lines = src.split('\n').filter((l) => COLOR.test(l))
    expect(lines).toEqual([])
  })
})

describe('src/lib/theme.ts (Monaco, xterm)', () => {
  it.each([
    ['bg', '--bg'], ['surface1', '--surface-1'], ['surface2', '--surface-2'], ['txt', '--txt'], ['dim', '--dim'],
    ['faint', '--faint'], ['accent', '--accent'], ['work', '--work'], ['wait', '--wait'], ['err', '--err'],
  ] as const)('palette.%s coincide con %s', (key, name) => {
    expect(palette[key].toLowerCase()).toBe(token(name).toLowerCase())
  })

  it('withAlpha produce #rrggbbaa', () => {
    expect(withAlpha('#8fa8ff', 0.25)).toBe('#8fa8ff40')
    expect(withAlpha('#ffffff', 0)).toBe('#ffffff00')
    expect(withAlpha('#ffffff', 1)).toBe('#ffffffff')
  })

  it('editor e terminale sono trasparenti: lo sfondo è quello del pannello', () => {
    expect(monacoColors['editor.background']).toBe('#00000000')
    expect(xtermTheme.background).toBe('#00000000')
    expect(monacoColors['editorWidget.background']).toBe(palette.surface2)
  })
})
