// @vitest-environment jsdom
import { cleanup, render } from '@testing-library/svelte'
import { flushSync } from 'svelte'
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest'

vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }))
vi.mock('@tauri-apps/plugin-opener', () => ({ openUrl: vi.fn() }))
vi.mock('../lib/ipc', () => ({
  convHistory: vi.fn(async () => []),
  convPending: vi.fn(async () => []),
  convMarkViewed: vi.fn(async () => {}),
  convAnswer: vi.fn(async () => {}),
  convList: vi.fn(async () => []),
  convStart: vi.fn(),
  convRename: vi.fn(),
  convUnlockTitle: vi.fn(),
  convArchive: vi.fn(),
}))

import ConvPane from './ConvPane.svelte'
import { chat } from '../lib/stores/chat.svelte'
import { conversations } from '../lib/stores/conversations.svelte'
import { initialState } from '../lib/conv/reduce'
import type { Conversation } from '../lib/types'

const conv: Conversation = {
  sessionId: 's1', projectId: 'p', cwd: '/Users/dev/app', title: 'Titolo', prevTitle: null, titleChangedAt: null, titleLocked: false,
  summary: '', status: 'idle', owned: false, archived: false, lastActivity: 0, turnCount: 0, model: null, permissionMode: 'default',
  contextUsedPct: null, maybeOpenElsewhere: false,
}

beforeAll(() => {
  vi.stubGlobal('ResizeObserver', class { observe() {} disconnect() {} unobserve() {} })
})
afterEach(cleanup)

describe('ConvPane', () => {
  it('a states change with the same id does not re-call chat.refresh (no re-stick every frame)', async () => {
    const open = vi.spyOn(chat, 'refresh')
    conversations.all = [conv]
    conversations.activeId = 's1'
    render(ConvPane)
    flushSync()
    expect(open).toHaveBeenCalledTimes(1)
    for (let i = 0; i < 3; i++) {
      chat.states = { ...chat.states, s1: initialState([{ kind: 'assistant', id: `a${i}`, text: 'x' }]) }
      flushSync()
    }
    expect(open).toHaveBeenCalledTimes(1)
    conversations.activeId = null
  })
})
