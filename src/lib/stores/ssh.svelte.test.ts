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
import { SshStore, resolveRoot } from './ssh.svelte'

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

  it('a cancelled host-key dialog leaves the server disconnected, without an error', async () => {
    vi.mocked(ipc.sshConnect).mockRejectedValueOnce('Connessione annullata')
    await s.connect('a')
    expect(s.stateOf('a')).toBe('disconnected')
    expect(s.errors.a).toBeUndefined()
    expect(s.passwordFor).toBeNull()
  })

  it('saving or removing a server forgets its tree root', async () => {
    vi.mocked(ipc.sshServerSave).mockResolvedValueOnce({ id: 'a' } as never)
    s.roots = { a: '/home/dev', b: '/srv' }
    await s.save({ id: 'a', name: 'Prova', host: 'staging.example.test', user: null, port: null, identityFile: null, startDir: null })
    expect(s.roots).toEqual({ b: '/srv' })
    await s.remove('b')
    expect(s.roots).toEqual({})
  })

  it('resolves the tree root to an absolute path', () => {
    expect(resolveRoot(null, '/home/dev')).toBe('/home/dev')
    expect(resolveRoot('~', '/home/dev/')).toBe('/home/dev')
    expect(resolveRoot('~/app/', '/home/dev')).toBe('/home/dev/app')
    expect(resolveRoot('/srv/app//', '/home/dev')).toBe('/srv/app')
    expect(resolveRoot('/', '/home/dev')).toBe('/')
    expect(resolveRoot('~', '/')).toBe('/')
  })
})
