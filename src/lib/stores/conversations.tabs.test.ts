// @vitest-environment jsdom
import { describe, expect, it, vi } from 'vitest'

vi.mock('../events', () => ({ listenProject: vi.fn(async () => () => {}) }))
const mk = (sessionId: string) => ({
  sessionId, projectId: 'p', cwd: '/Users/dev/app', title: sessionId, prevTitle: null, titleChangedAt: null, titleLocked: false,
  summary: '', status: 'idle', owned: false, archived: false, lastActivity: 1, turnCount: 1, model: null, permissionMode: 'default',
  contextUsedPct: null, maybeOpenElsewhere: false,
})
vi.mock('../ipc', () => ({
  convList: vi.fn(async () => [mk('a'), mk('b')]),
  convMarkViewed: vi.fn(async () => {}),
}))

import { conversations } from './conversations.svelte'

describe('schede ricordate', () => {
  it('ripristina le schede salvate scartando quelle sparite', async () => {
    localStorage.setItem('codeart.convtabs.p', JSON.stringify({ tabs: ['b', 'sparita', 'a'], activeId: 'sparita' }))
    await conversations.start('p')
    expect(conversations.tabs).toEqual(['b', 'a'])
    expect(conversations.activeId).toBe('b')
    expect(conversations.openConvs.map((c) => c.sessionId)).toEqual(['b', 'a'])
    conversations.close('b')
    expect(JSON.parse(localStorage.getItem('codeart.convtabs.p')!)).toEqual({ tabs: ['a'], activeId: 'a' })
    conversations.stop()
  })
})
