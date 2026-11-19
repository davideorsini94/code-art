// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from 'vitest'

const h = vi.hoisted(() => ({ cb: null as null | ((e: { payload: unknown }) => void) }))
vi.mock('../events', () => ({
  listenSshState: vi.fn(async (cb: (e: { payload: unknown }) => void) => {
    h.cb = cb
    return () => {}
  }),
}))
vi.mock('../ipc', () => ({
  sshServers: vi.fn(async () => [
    { id: 'a', name: 'Prova', host: 'example.test', user: null, port: null, identityFile: null, startDir: null, source: 'saved', hasPassword: false },
  ]),
  sshStates: vi.fn(async () => [{ id: 'a', state: 'connecting' }]),
  sshConnect: vi.fn(),
  sshDisconnect: vi.fn(),
  sshPasswordSet: vi.fn(),
  sshPasswordClear: vi.fn(),
  sshServerSave: vi.fn(),
  sshServerDelete: vi.fn(),
  sshDownload: vi.fn(),
}))
vi.mock('./project.svelte', () => ({ projectStore: { project: { folders: ['/Users/dev/app'] } } }))

import * as ipc from '../ipc'
import { SshStore } from './ssh.svelte'

let s: SshStore
beforeEach(() => {
  vi.clearAllMocks()
  s = new SshStore()
})

describe('SshStore', () => {
  it('loads servers and states, then applies state events', async () => {
    await s.init()
    expect(s.servers).toHaveLength(1)
    expect(s.stateOf('a')).toBe('connecting')
    h.cb!({ payload: { id: 'a', state: 'connected' } })
    expect(s.connected.map((x) => x.name)).toEqual(['Prova'])
    h.cb!({ payload: { id: 'a', state: 'error', message: 'rifiutato' } })
    expect(s.stateOf('a')).toBe('error')
    expect(s.errors.a).toBe('rifiutato')
    expect(s.stateOf('zzz')).toBe('disconnected')
  })

  it("asks for the password on 'auth' and retries after it is set", async () => {
    vi.mocked(ipc.sshConnect).mockRejectedValueOnce('auth')
    await s.connect('a')
    expect(s.passwordFor).toBe('a')
    expect(s.stateOf('a')).toBe('disconnected')
    await s.setPassword('a', 'x', true)
    expect(ipc.sshPasswordSet).toHaveBeenCalledWith('a', 'x')
    expect(ipc.sshConnect).toHaveBeenCalledTimes(2)
    expect(s.passwordFor).toBeNull()
  })

  it('records other connection errors', async () => {
    vi.mocked(ipc.sshConnect).mockRejectedValueOnce('chiave cambiata')
    await s.connect('a')
    expect(s.passwordFor).toBeNull()
    expect(s.stateOf('a')).toBe('error')
    expect(s.errors.a).toBe('chiave cambiata')
  })

  it('downloads into the first project folder', async () => {
    await s.download('a', '/home/dev/f.txt')
    expect(ipc.sshDownload).toHaveBeenCalledWith('a', ['/home/dev/f.txt'], '/Users/dev/app')
    expect(s.notice).toContain('/Users/dev/app')
  })
})
