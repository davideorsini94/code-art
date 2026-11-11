import { describe, expect, it } from 'vitest'
import { PtyInbox } from './pty-inbox'

const b = (n: number) => new Uint8Array([n])

describe('PtyInbox', () => {
  it('keeps early data until the tab takes it, in order', () => {
    const box = new PtyInbox()
    box.hold(1, b(1))
    box.hold(1, b(2))
    expect(box.take(1)).toEqual([b(1), b(2)])
    expect(box.take(1)).toEqual([])
    expect(box.size).toBe(0)
  })

  it('data for a closed PTY is dropped, never accumulated', () => {
    const box = new PtyInbox()
    box.hold(7, b(1))
    box.close(7)
    expect(box.size).toBe(0)
    for (let i = 0; i < 100; i++) box.hold(7, b(i))
    expect(box.size).toBe(0)
    expect(box.take(7)).toEqual([])
  })
})
