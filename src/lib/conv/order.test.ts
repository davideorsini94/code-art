import { describe, expect, it } from 'vitest'
import type { Conversation } from '../types'
import { activeCount, sortConversations, upsertConversation, waitingCount, WINDOW_THRESHOLD, windowRange } from './order'

const mk = (sessionId: string, status: Conversation['status'], lastActivity: number, extra: Partial<Conversation> = {}): Conversation => ({
  sessionId, projectId: 'p', cwd: '/Users/dev/app', title: sessionId, prevTitle: null, titleChangedAt: null, titleLocked: false,
  summary: '', status, owned: false, archived: false, lastActivity, turnCount: 1, model: null, permissionMode: 'default',
  contextUsedPct: null, maybeOpenElsewhere: false, ...extra,
})

describe('sortConversations', () => {
  it('ordina waiting, working, idle; dentro il gruppo per lastActivity decrescente', () => {
    const list = [
      mk('idle-old', 'idle', 10),
      mk('work-new', 'working', 90),
      mk('wait-old', 'waiting', 20),
      mk('idle-new', 'idle', 80),
      mk('wait-new', 'waiting', 70),
      mk('work-old', 'working', 30),
    ]
    expect(sortConversations(list).map((c) => c.sessionId)).toEqual(['wait-new', 'wait-old', 'work-new', 'work-old', 'idle-new', 'idle-old'])
  })
  it('esclude le archiviate e non muta l\'input', () => {
    const list = [mk('a', 'idle', 1, { archived: true }), mk('b', 'idle', 2)]
    expect(sortConversations(list).map((c) => c.sessionId)).toEqual(['b'])
    expect(list).toHaveLength(2)
  })
  it('con includeArchived mostra solo le archiviate', () => {
    const list = [mk('a', 'idle', 1, { archived: true }), mk('b', 'idle', 2)]
    expect(sortConversations(list, true).map((c) => c.sessionId)).toEqual(['a'])
  })
})

describe('contatori', () => {
  const list = [mk('a', 'waiting', 1), mk('b', 'working', 2), mk('c', 'waiting', 3, { archived: true }), mk('d', 'idle', 4)]
  it('waitingCount ignora le archiviate', () => expect(waitingCount(list)).toBe(1))
  it('activeCount = working + waiting non archiviate', () => expect(activeCount(list)).toBe(2))
})

describe('upsertConversation', () => {
  it('sostituisce per sessionId senza duplicati, anche con una raffica', () => {
    let list: Conversation[] = [mk('a', 'idle', 1)]
    for (let i = 0; i < 200; i++) list = upsertConversation(list, mk(i % 2 ? 'a' : 'b', 'working', i))
    expect(list.map((c) => c.sessionId).sort()).toEqual(['a', 'b'])
    expect(list.find((c) => c.sessionId === 'a')!.lastActivity).toBe(199)
  })
  it('non muta la lista di partenza', () => {
    const base = [mk('a', 'idle', 1)]
    const next = upsertConversation(base, mk('z', 'idle', 2))
    expect(base).toHaveLength(1)
    expect(next).toHaveLength(2)
  })
})

describe('windowRange', () => {
  it('sotto la soglia mostra tutto', () => {
    expect(windowRange(WINDOW_THRESHOLD, 0, 400, 80)).toEqual({ start: 0, end: WINDOW_THRESHOLD })
  })
  it('oltre la soglia mostra solo le schede visibili più il margine', () => {
    const r = windowRange(500, 80 * 100, 400, 80)
    expect(r.start).toBeLessThanOrEqual(100)
    expect(r.start).toBeGreaterThan(80)
    expect(r.end).toBeGreaterThanOrEqual(105)
    expect(r.end).toBeLessThan(130)
  })
  it('è limitato a [0, count]', () => {
    expect(windowRange(500, 0, 400, 80).start).toBe(0)
    expect(windowRange(500, 1e9, 400, 80).end).toBe(500)
  })
})
