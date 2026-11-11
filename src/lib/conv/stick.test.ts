import { describe, expect, it } from 'vitest'
import { shouldStick } from './stick'

describe('shouldStick', () => {
  it('resets only when the conversation changes', () => {
    expect(shouldStick(false, { idChanged: true })).toBe(true)
    expect(shouldStick(false, { idChanged: false })).toBe(false)
    expect(shouldStick(true, { idChanged: false })).toBe(true)
  })
  it('follows the user scroll', () => {
    expect(shouldStick(true, { idChanged: false, scrolledUp: true })).toBe(false)
    expect(shouldStick(false, { idChanged: false, atBottom: true })).toBe(true)
  })
})
