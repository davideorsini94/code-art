// @vitest-environment jsdom
import { afterEach, describe, expect, it } from 'vitest'
import { guardWindowDrops } from './dropguard'

let off: (() => void) | undefined
afterEach(() => off?.())

const fire = (target: EventTarget, type: string) => {
  const e = new Event(type, { bubbles: true, cancelable: true })
  target.dispatchEvent(e)
  return e
}

describe('guardWindowDrops', () => {
  it('a drop or dragover anywhere in the window is cancelled (no navigation)', () => {
    off = guardWindowDrops(window)
    const el = document.createElement('div')
    document.body.appendChild(el)
    expect(fire(el, 'dragover').defaultPrevented).toBe(true)
    expect(fire(el, 'drop').defaultPrevented).toBe(true)
    expect(fire(window, 'drop').defaultPrevented).toBe(true)
  })
  it('removing the guard restores the default', () => {
    guardWindowDrops(window)()
    expect(fire(document.body, 'drop').defaultPrevented).toBe(false)
  })
})
