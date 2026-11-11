import { describe, expect, it } from 'vitest'
import { onDiskChange } from './editor.svelte'

const disk = (text: string) => ({ text, mtimeMs: 2 })

describe('onDiskChange', () => {
  it('ricarica se la tab non ha modifiche locali e il testo su disco differisce', () => {
    expect(onDiskChange({ text: 'a', savedText: 'a' }, disk('b'))).toBe('reload')
  })
  it('chiede se la tab ha modifiche locali e il disco è cambiato', () => {
    expect(onDiskChange({ text: 'a!', savedText: 'a' }, disk('b'))).toBe('prompt')
  })
  it('non fa nulla se il testo su disco è uguale', () => {
    expect(onDiskChange({ text: 'a', savedText: 'a' }, disk('a'))).toBe('none')
    expect(onDiskChange({ text: 'a!', savedText: 'a' }, disk('a'))).toBe('none')
  })
})
