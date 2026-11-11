import { afterEach, describe, expect, it, vi } from 'vitest'

vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }))
vi.mock('../ipc', () => ({
  convDelete: vi.fn(async () => ({ kind: 'deletedWithWarning', message: 'residuo in ~/.claude' })),
  convArchive: vi.fn(async () => {}),
  convList: vi.fn(async () => []),
  convStart: vi.fn(async () => 'nuova'),
  convMarkViewed: vi.fn(async () => {}),
  convRename: vi.fn(async () => {}),
  convUnlockTitle: vi.fn(async () => {}),
}))

import { conversations, NOTICE_MS } from './conversations.svelte'
import { projectStore } from './project.svelte'
import * as ipc from '../ipc'

afterEach(() => {
  vi.useRealTimers()
  projectStore.untrusted = []
  conversations.notice = null
  conversations.error = null
})

describe('conversations store', () => {
  it('a notice disappears on the next action or after a few seconds', async () => {
    vi.useFakeTimers()
    await conversations.delete('a')
    expect(conversations.notice).toBe('residuo in ~/.claude')
    await conversations.archive('b', true)
    expect(conversations.notice).toBeNull()
    await conversations.delete('a')
    expect(conversations.notice).not.toBeNull()
    vi.advanceTimersByTime(NOTICE_MS + 1)
    expect(conversations.notice).toBeNull()
  })

  it('no new conversation in an untrusted folder: readable error, no call to the core', async () => {
    await conversations.start('p')
    projectStore.untrusted = ['/Users/dev/app']
    expect(await conversations.create()).toBeNull()
    expect(ipc.convStart).not.toHaveBeenCalled()
    expect(conversations.error).toBe('Cartella non fidata: sola lettura, Claude e terminale disattivati')
    conversations.stop()
  })
})
