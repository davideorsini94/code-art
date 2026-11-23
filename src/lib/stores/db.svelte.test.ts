// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from 'vitest'

const h = vi.hoisted(() => ({ cb: null as null | ((e: { payload: unknown }) => void) }))
vi.mock('../events', () => ({
  listenDbState: vi.fn(async (cb: (e: { payload: unknown }) => void) => {
    h.cb = cb
    return () => {}
  }),
}))
vi.mock('../ipc', () => ({
  dbConnections: vi.fn(async () => [
    { id: 'a', kind: 'postgres', name: 'Prova', host: 'db.example.test', port: 5432, user: null, database: null, tls: 'prefer', acceptInvalidCerts: false, sshServerId: null, hasPassword: false },
  ]),
  dbStates: vi.fn(async () => [{ id: 'a', state: 'connecting' }]),
  dbConnect: vi.fn(),
  dbDisconnect: vi.fn(),
  dbPasswordSet: vi.fn(),
  dbPasswordClear: vi.fn(),
  dbConnectionSave: vi.fn(async (i: { id?: string }) => ({ id: i.id ?? 'a', hasPassword: false })),
  dbConnectionDelete: vi.fn(),
  dbPgTree: vi.fn(async () => ({ schemas: [{ name: 'public', tables: [{ name: 't', kind: 'table' }] }] })),
  dbMongoTree: vi.fn(async () => []),
}))

import * as ipc from '../ipc'
import { DbStore } from './db.svelte'

let s: DbStore
beforeEach(() => {
  vi.clearAllMocks()
  s = new DbStore()
})

describe('DbStore', () => {
  it('loads connections and states, then applies db:state events', async () => {
    await s.init()
    expect(s.connections).toHaveLength(1)
    expect(s.stateOf('a')).toBe('connecting')
    h.cb!({ payload: { id: 'a', state: 'connected' } })
    expect(s.stateOf('a')).toBe('connected')
    await s.loadTree('a')
    expect(s.trees.a.kind).toBe('postgres')
    h.cb!({ payload: { id: 'a', state: 'error', message: 'rifiutato' } })
    expect(s.errors.a).toBe('rifiutato')
    expect(s.stateOf('zzz')).toBe('disconnected')
  })

  it("asks for the password on 'auth' and retries after it is set", async () => {
    await s.init()
    vi.mocked(ipc.dbConnect).mockRejectedValueOnce('auth')
    await s.connect('a')
    expect(s.passwordFor).toBe('a')
    expect(s.stateOf('a')).toBe('disconnected')
    await s.setPassword('a', 'segreto', true)
    expect(ipc.dbPasswordSet).toHaveBeenCalledWith('a', 'segreto')
    expect(ipc.dbConnect).toHaveBeenCalledTimes(2)
    expect(s.passwordFor).toBeNull()
    expect(s.stateOf('a')).toBe('connected')
  })

  it('records a connection error', async () => {
    await s.init()
    vi.mocked(ipc.dbConnect).mockRejectedValueOnce('Timeout')
    await s.connect('a')
    expect(s.stateOf('a')).toBe('error')
    expect(s.errors.a).toBe('Timeout')
  })

  it('disconnects an open connection before saving it and tells the user', async () => {
    await s.init()
    h.cb!({ payload: { id: 'a', state: 'connected' } })
    await s.save({ id: 'a', kind: 'postgres', name: 'x', host: 'h', user: null, database: null, tls: 'prefer', acceptInvalidCerts: false, sshServerId: null })
    expect(vi.mocked(ipc.dbDisconnect).mock.invocationCallOrder[0]).toBeLessThan(vi.mocked(ipc.dbConnectionSave).mock.invocationCallOrder[0])
    expect(s.notice).toContain('chiusa')
    expect(s.trees.a).toBeUndefined()
  })

  it('openQuery records the request in pendingQuery', () => {
    s.openQuery({ connectionId: 'a', kind: 'postgres', schema: 'public', table: 't' })
    expect(s.pendingQuery).toEqual({ connectionId: 'a', kind: 'postgres', schema: 'public', table: 't' })
  })
})
