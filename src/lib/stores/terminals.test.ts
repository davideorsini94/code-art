import { describe, expect, it } from 'vitest'
import { TerminalStore } from './terminals.svelte'

describe('terminals store', () => {
  it('toggles and shows', () => {
    const s = new TerminalStore()
    expect(s.visible).toBe(false)
    s.toggle()
    expect(s.visible).toBe(true)
    s.toggle()
    s.show()
    s.show()
    expect(s.visible).toBe(true)
  })

  it('open makes the panel visible and queues requests in order', () => {
    const s = new TerminalStore()
    s.open('/Users/dev/app/src')
    s.open()
    s.open('/Users/dev/app/src')
    expect(s.visible).toBe(true)
    expect(s.requests.map((r) => r.kind === 'local' ? r.cwd : r.dir)).toEqual(['/Users/dev/app/src', undefined, '/Users/dev/app/src'])
    const seqs = s.requests.map((r) => r.seq)
    expect(new Set(seqs).size).toBe(3)
    expect([...seqs].sort((a, b) => a - b)).toEqual(seqs)
  })

  it('openRemote queues a remote request and shows the panel', () => {
    const s = new TerminalStore()
    s.openRemote('s1', '/home/dev/app')
    expect(s.visible).toBe(true)
    expect(s.requests).toMatchObject([{ kind: 'remote', serverId: 's1', dir: '/home/dev/app' }])
  })

  it('take returns and empties the queue', () => {
    const s = new TerminalStore()
    s.open('/Users/dev/a')
    s.open('/Users/dev/b')
    expect(s.take().map((r) => r.kind === 'local' ? r.cwd : r.dir)).toEqual(['/Users/dev/a', '/Users/dev/b'])
    expect(s.requests).toEqual([])
    expect(s.take()).toEqual([])
  })
})
