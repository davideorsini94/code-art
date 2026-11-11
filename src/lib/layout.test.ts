import { describe, expect, it } from 'vitest'
import { canCollapse } from './layout'

describe('canCollapse', () => {
  it('non permette di chiudere convlist se conv è già chiusa', () => {
    expect(canCollapse('convlist', new Set(['conv']))).toBe(false)
  })
  it('non permette di chiudere conv se convlist è già chiusa', () => {
    expect(canCollapse('conv', new Set(['convlist']))).toBe(false)
  })
  it('permette di chiudere convlist se conv è aperta', () => {
    expect(canCollapse('convlist', new Set())).toBe(true)
  })
  it('permette di chiudere conv se convlist è aperta', () => {
    expect(canCollapse('conv', new Set(['sidebar']))).toBe(true)
  })
  it('non limita le altre colonne', () => {
    expect(canCollapse('sidebar', new Set(['convlist', 'conv']))).toBe(true)
  })
})
