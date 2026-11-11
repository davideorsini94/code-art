import { describe, expect, it } from 'vitest'
import { displayedModel, modelLabel, modelOptions } from './model'

const n = { convModel: null, initModel: null, lastSetAt: null, initAt: null }
describe('displayedModel', () => {
  it('nothing known -> empty', () => expect(displayedModel(n)).toBe(''))
  it('init wins over the stored model when nothing was chosen', () =>
    expect(displayedModel({ ...n, convModel: 'sonnet', initModel: 'claude-opus-4', initAt: 1 })).toBe('claude-opus-4'))
  it('falls back to conv.model without init', () => expect(displayedModel({ ...n, convModel: 'sonnet' })).toBe('sonnet'))
  it('an explicit choice after init wins', () =>
    expect(displayedModel({ convModel: 'haiku', initModel: 'claude-opus-4', initAt: 1, lastSetAt: 2 })).toBe('haiku'))
  it('an explicit choice without init wins', () => expect(displayedModel({ ...n, convModel: 'haiku', lastSetAt: 2 })).toBe('haiku'))
  it('a later init wins over the explicit choice', () =>
    expect(displayedModel({ convModel: 'haiku', initModel: 'claude-sonnet-4', initAt: 3, lastSetAt: 2 })).toBe('claude-sonnet-4'))
  it('explicit choice with conv.model not yet updated falls back to init', () =>
    expect(displayedModel({ convModel: null, initModel: 'x', initAt: 1, lastSetAt: 2 })).toBe('x'))
})

describe('modelOptions / modelLabel', () => {
  it('lists the aliases in order, fable first', () => expect(modelOptions('')).toEqual(['fable', 'opus', 'sonnet', 'haiku']))
  it('does not duplicate a current alias', () => expect(modelOptions('haiku')).toEqual(['fable', 'opus', 'sonnet', 'haiku']))
  it('appends a model reported by the process', () => expect(modelOptions('claude-opus-4')).toEqual(['fable', 'opus', 'sonnet', 'haiku', 'claude-opus-4']))
  it('friendly labels for the aliases, raw id otherwise', () => {
    expect(['fable', 'opus', 'sonnet', 'haiku'].map(modelLabel)).toEqual(['Fable', 'Opus', 'Sonnet', 'Haiku'])
    expect(modelLabel('claude-opus-4')).toBe('claude-opus-4')
  })
})
