import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { ChatItem, ClaudeEvent, Conversation } from '../types'

const handlers = new Map<string, (ev: { payload: unknown }) => void>()
vi.mock('../events', () => ({
  listenProject: vi.fn(async (name: string, projectId: string, cb: (ev: { payload: unknown }) => void) => {
    handlers.set(`${name}:${projectId}`, cb)
    return () => handlers.delete(`${name}:${projectId}`)
  }),
}))
const convHistory = vi.fn<(id: string) => Promise<ChatItem[]>>()
const convPending = vi.fn<(id: string) => Promise<ClaudeEvent[]>>()
vi.mock('../ipc', () => ({
  convHistory: (id: string) => convHistory(id),
  convPending: (id: string) => convPending(id),
  convAnswer: vi.fn(async () => {}),
  convList: vi.fn(async () => []),
  convMarkViewed: vi.fn(async () => {}),
  convStart: vi.fn(),
  convRename: vi.fn(),
  convUnlockTitle: vi.fn(),
  convArchive: vi.fn(),
  convDelete: vi.fn(),
}))

import { chat } from './chat.svelte'
import { conversations } from './conversations.svelte'

const conv = (over: Partial<Conversation> = {}): Conversation => ({
  sessionId: 'new', projectId: 'p1', cwd: '/Users/dev/app', title: '', prevTitle: null, titleChangedAt: null, titleLocked: false,
  summary: '', status: 'idle', owned: true, archived: false, lastActivity: 0, turnCount: 0, model: null, permissionMode: 'default',
  contextUsedPct: null, maybeOpenElsewhere: false, ...over,
})
const emit = (sessionId: string, event: ClaudeEvent) => handlers.get('conv:event:p1')!({ payload: { sessionId, event } })
const ask = (requestId: string): ClaudeEvent => ({ kind: 'ask', requestId, askType: 'permission', prompt: 'Bash: ls', options: [], toolName: 'Bash', input: {} })
const settle = () => new Promise((r) => setTimeout(r, 0))

beforeEach(async () => {
  handlers.clear()
  convHistory.mockReset()
  convPending.mockReset().mockResolvedValue([])
  for (const id of Object.keys(chat.states)) chat.drop(id)
  conversations.all = []
  await chat.start('p1')
})
afterEach(() => chat.stop())

describe('chat store: new conversation', () => {
  it('starts from an empty state when the session file does not exist yet, and streams the first turn', async () => {
    conversations.all = [conv()]
    convHistory.mockRejectedValueOnce('file della sessione non trovato')
    await chat.open('new')
    expect(chat.states.new).toBeDefined()
    expect(chat.errorFor('new')).toBeNull()

    chat.addUser('new', 'ciao')
    expect(chat.states.new.items.map((i) => i.kind)).toEqual(['user'])

    emit('new', { kind: 'textDelta', id: 'm:0', text: 'Ciao' })
    emit('new', { kind: 'textDelta', id: 'm:0', text: '!' })
    chat.flush()
    expect(chat.states.new.items[1]).toEqual({ kind: 'assistant', id: 'm:0', text: 'Ciao!' })

    // fine turno: la storia ora c'è e sostituisce lo stato live
    convHistory.mockResolvedValueOnce([
      { kind: 'user', id: 'u1', text: 'ciao', images: [] },
      { kind: 'assistant', id: 'a1', text: 'Ciao!' },
    ])
    emit('new', { kind: 'turnEnd', contextUsedPct: 3, isError: false })
    await settle()
    await settle()
    expect(chat.states.new.items.map((i) => i.id)).toEqual(['u1', 'a1'])
    expect(chat.states.new.contextUsedPct).toBe(3)
  })

  it('an empty history (core: owned conversation without file) is a normal empty state', async () => {
    convHistory.mockResolvedValueOnce([])
    await chat.open('new')
    expect(chat.states.new.items).toEqual([])
  })

  it('keeps the error for an imported conversation whose file is missing', async () => {
    conversations.all = [conv({ sessionId: 'old', owned: false, turnCount: 3 })]
    convHistory.mockRejectedValueOnce('untrusted_folder')
    await chat.open('old')
    expect(chat.states.old).toBeUndefined()
    expect(chat.errorFor('old')).toBe('Cartella non fidata: sola lettura, Claude e terminale disattivati')
  })
})

describe('chat store: events while no pane is mounted', () => {
  it('an ask for a cached conversation arriving with the pane closed is there when it reopens', async () => {
    convHistory.mockResolvedValue([])
    await chat.open('s1')
    emit('s1', ask('r1'))
    chat.flush()
    await chat.refresh('s1')
    expect(chat.states.s1.items.some((i) => i.kind === 'ask' && i.requestId === 'r1' && !i.answered)).toBe(true)
  })

  it('refresh re-reads pending asks of a cached conversation (missed while away)', async () => {
    convHistory.mockResolvedValue([])
    await chat.open('s1')
    convPending.mockResolvedValueOnce([ask('r2')])
    await chat.refresh('s1')
    expect(chat.states.s1.items.filter((i) => i.kind === 'ask').map((i) => (i as { requestId: string }).requestId)).toEqual(['r2'])
  })

  it('refresh of a conversation not in cache opens it with its pending asks', async () => {
    convHistory.mockResolvedValue([])
    convPending.mockResolvedValueOnce([ask('r3')])
    await chat.refresh('s2')
    expect(chat.states.s2.items.some((i) => i.kind === 'ask' && i.requestId === 'r3')).toBe(true)
  })
})
