// Rete di sicurezza: ogni controllo cliccabile nei componenti deve avere `use:tooltip` oppure `data-no-tooltip="motivo"`.
// Euristica: legge i tag HTML (minuscoli) dei file .svelte e controlla quelli interattivi.
import { readdirSync, readFileSync, statSync } from 'node:fs'
import { join, relative } from 'node:path'
import { describe, expect, it } from 'vitest'

const ROOT = join(__dirname, '..')

function svelteFiles(dir: string): string[] {
  return readdirSync(dir).flatMap((n) => {
    const p = join(dir, n)
    return statSync(p).isDirectory() ? svelteFiles(p) : p.endsWith('.svelte') ? [p] : []
  })
}

/** Tag di apertura `<nome ...>` con attributi (rispetta virgolette e graffe, dove compaiono `>` e `=>`). */
export function openingTags(src: string): { name: string; text: string; line: number }[] {
  const out: { name: string; text: string; line: number }[] = []
  const markup = src.replace(/<script[\s\S]*?<\/script>/g, (m) => m.replace(/[^\n]/g, ' ')).replace(/<style[\s\S]*?<\/style>/g, (m) => m.replace(/[^\n]/g, ' ')).replace(/<!--[\s\S]*?-->/g, (m) => m.replace(/[^\n]/g, ' '))
  const re = /<([a-z][a-z0-9-]*)(?=[\s>/])/g
  let m: RegExpExecArray | null
  while ((m = re.exec(markup))) {
    let i = m.index + m[0].length
    let depth = 0
    let quote = ''
    for (; i < markup.length; i++) {
      const c = markup[i]
      if (quote) {
        if (c === quote && markup[i - 1] !== '\\') quote = ''
      } else if (depth > 0) {
        if (c === '{') depth++
        else if (c === '}') depth--
        else if (c === '"' || c === "'" || c === '`') quote = c
      } else if (c === '{') depth++
      else if (c === '"' || c === "'") quote = c
      else if (c === '>') break
    }
    out.push({ name: m[1], text: markup.slice(m.index, i + 1), line: markup.slice(0, m.index).split('\n').length })
  }
  return out
}

const INTERACTIVE_ROLES = /\brole=["'](button|tab|menuitem|radio|treeitem|option)["']/
const HANDLERS = /\s(onclick|ondblclick|onpointerdown|onpointerup|onmousedown|onkeydown)=/
const FOCUSABLE_NO_ROLE = /\stabindex=["']0["']/

function isInteractive({ name, text }: { name: string; text: string }): boolean {
  if (name === 'button' || name === 'select' || (name === 'a' && /\shref=/.test(text))) return true
  if (name === 'input' || name === 'textarea') return false // campi di testo: si scrive, non si clicca
  return INTERACTIVE_ROLES.test(text) || HANDLERS.test(text) || (FOCUSABLE_NO_ROLE.test(text) && !/\srole=/.test(text))
}

export function missingTooltips(src: string): { name: string; line: number }[] {
  return openingTags(src)
    .filter(isInteractive)
    .filter(({ text }) => !/\suse:tooltip\b/.test(text) && !/\sdata-no-tooltip="[^"]{8,}"/.test(text))
    .map(({ name, line }) => ({ name, line }))
}

describe('every interactive element has a tooltip', () => {
  const files = [...svelteFiles(join(ROOT, 'components')), join(ROOT, 'App.svelte')]

  it('finds the components', () => expect(files.length).toBeGreaterThan(20))

  for (const f of files) {
    it(relative(ROOT, f), () => {
      expect(missingTooltips(readFileSync(f, 'utf8'))).toEqual([])
    })
  }

  it('the heuristic flags a bare button, and accepts use:tooltip or a reasoned opt-out', () => {
    expect(missingTooltips('<button onclick={() => a(1)}>x</button>')).toEqual([{ name: 'button', line: 1 }])
    expect(missingTooltips('<div role="tab" tabindex="0">x</div>')).toHaveLength(1)
    expect(missingTooltips('<div onclick={(e) => e.x > 1 ? a() : b()}>x</div>')).toHaveLength(1)
    expect(missingTooltips("<button use:tooltip={'Salva'} onclick={f}>x</button>")).toEqual([])
    expect(missingTooltips('<button data-no-tooltip="ha già un testo esplicito" onclick={f}>x</button>')).toEqual([])
    expect(missingTooltips('<button data-no-tooltip="" onclick={f}>x</button>')).toHaveLength(1)
    expect(missingTooltips('<div onkeydown={f}>x</div>')).toHaveLength(1)
    expect(missingTooltips('<div onmousedown={f}>x</div>')).toHaveLength(1)
    expect(missingTooltips('<span tabindex="0">x</span>')).toHaveLength(1)
    expect(missingTooltips('<!-- <button> --><p>testo</p>')).toEqual([])
  })
})
