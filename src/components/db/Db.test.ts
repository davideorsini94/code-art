// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte'
import { afterEach, describe, expect, it, vi } from 'vitest'

vi.mock('../../lib/events', () => ({ listenDbState: vi.fn(async () => () => {}), listenSshState: vi.fn(async () => () => {}), listenProject: vi.fn(async () => () => {}) }))
const HOSTILE = '<img src=x onerror=alert(1)>'
vi.mock('../../lib/ipc', () => ({
  dbConnections: vi.fn(async () => []),
  dbStates: vi.fn(async () => []),
  dbConnect: vi.fn(),
  dbDisconnect: vi.fn(),
  dbPasswordSet: vi.fn(),
  dbPasswordClear: vi.fn(),
  dbConnectionSave: vi.fn(async (i: { id?: string }) => ({ id: i.id ?? 'n1', hasPassword: false })),
  dbConnectionDelete: vi.fn(),
  dbPgTree: vi.fn(async () => ({ schemas: [{ name: 's"; DROP', tables: [{ name: HOSTILE, kind: 'table' }, { name: 'v1', kind: 'view' }] }] })),
  dbOraTree: vi.fn(async () => ({ schemas: [{ name: 'HR', tables: [{ name: 'EMP', kind: 'table' }] }] })),
  dbOracleClientStatus: vi.fn(),
  dbMongoTree: vi.fn(async () => [{ database: HOSTILE, collections: ['<b>c</b>'] }]),
  sshServers: vi.fn(async () => [{ id: 'srv1', name: 'Bastione', host: 'h', user: null, port: null, identityFile: null, startDir: null, source: 'saved', hasPassword: false }]),
  sshStates: vi.fn(async () => []),
}))
vi.mock('../../lib/stores/project.svelte', () => ({ projectStore: { project: { id: 'p1', folders: [] } } }))

import * as ipc from '../../lib/ipc'
import { db } from '../../lib/stores/db.svelte'
import { ssh } from '../../lib/stores/ssh.svelte'
import DbForm from './DbForm.svelte'
import DbTree from './DbTree.svelte'

afterEach(() => {
  cleanup()
  vi.clearAllMocks()
  db.pendingQuery = null
  db.trees = {}
  db.connections = []
  db.guideFor = null
  db.oracleClient = null
})

describe('DbForm', () => {
  it('switching kind updates the default port only when it was empty or the other default', async () => {
    const { container } = render(DbForm, { onclose: () => {} })
    const port = screen.getByLabelText('Porta') as HTMLInputElement
    const kind = container.querySelector('select') as HTMLSelectElement
    expect(port.value).toBe('5432')
    await fireEvent.change(kind, { target: { value: 'mongo' } })
    expect(port.value).toBe('27017')
    await fireEvent.input(port, { target: { value: '5999' } })
    await fireEvent.change(kind, { target: { value: 'postgres' } })
    expect(port.value).toBe('5999')
  })

  it('saves the connection, sends the password separately and clears the field', async () => {
    await ssh.init()
    const onclose = vi.fn()
    render(DbForm, { onclose })
    await fireEvent.input(screen.getByLabelText('Nome'), { target: { value: 'Mia' } })
    await fireEvent.input(screen.getByLabelText('Host'), { target: { value: 'db.example.test' } })
    const pw = screen.getByLabelText(/Password/) as HTMLInputElement
    await fireEvent.input(pw, { target: { value: 'segreto' } })
    await fireEvent.click(screen.getByRole('button', { name: 'Salva' }))
    await waitFor(() => expect(onclose).toHaveBeenCalled())
    const input = vi.mocked(ipc.dbConnectionSave).mock.calls[0][0]
    expect(JSON.stringify(input)).not.toContain('segreto')
    expect(input).toMatchObject({ kind: 'postgres', name: 'Mia', host: 'db.example.test', port: 5432, tls: 'prefer', sshServerId: null })
    expect(ipc.dbPasswordSet).toHaveBeenCalledWith('n1', 'segreto')
    expect(pw.value).toBe('')
  })

  it('shows core errors inline and lists SSH servers', async () => {
    await ssh.init()
    vi.mocked(ipc.dbConnectionSave).mockRejectedValueOnce('Host obbligatorio')
    render(DbForm, { onclose: () => {} })
    expect(await screen.findByRole('option', { name: 'Bastione' })).toBeTruthy()
    await fireEvent.click(screen.getByRole('button', { name: 'Salva' }))
    expect((await screen.findByRole('alert')).textContent).toContain('Host obbligatorio')
  })
})

describe('DbForm Oracle', () => {
  it('Oracle: porta 1521, Servizio obbligatorio, niente TLS e nota sul tunnel', async () => {
    const { container } = render(DbForm, { onclose: () => {} })
    await fireEvent.change(container.querySelector('select') as HTMLSelectElement, { target: { value: 'oracle' } })
    expect((screen.getByLabelText('Porta') as HTMLInputElement).value).toBe('1521')
    expect(screen.getByLabelText(/Servizio/)).toBeTruthy()
    expect(screen.queryByLabelText('Database')).toBeNull()
    expect(screen.queryByLabelText(/^TLS/)).toBeNull()
    expect(screen.queryByText(/certificati non validi/i)).toBeNull()
    expect(screen.getByText('Solo TCP: per cifrare la connessione usa un tunnel SSH')).toBeTruthy()
    await fireEvent.input(screen.getByLabelText('Nome'), { target: { value: 'O' } })
    await fireEvent.input(screen.getByLabelText('Host'), { target: { value: 'h' } })
    await fireEvent.click(screen.getByRole('button', { name: 'Salva' }))
    await screen.findByText(/Indica il servizio/)
    expect(ipc.dbConnectionSave).not.toHaveBeenCalled()
    await fireEvent.input(screen.getByLabelText(/Servizio/), { target: { value: 'XEPDB1' } })
    await fireEvent.click(screen.getByRole('button', { name: 'Salva' }))
    await waitFor(() => expect(ipc.dbConnectionSave).toHaveBeenCalled())
    expect(vi.mocked(ipc.dbConnectionSave).mock.calls[0][0]).toMatchObject({ kind: 'oracle', port: 1521, database: 'XEPDB1', tls: 'disable' })
  })
})

describe('DbTree', () => {
  it('Oracle: albero da dbOraTree e apertura della tabella con nomi esatti', async () => {
    db.connections = [{ id: 'o1', kind: 'oracle', name: 'x', host: 'h', port: 1521, user: null, database: 'S', tls: 'disable', acceptInvalidCerts: false, sshServerId: null, hasPassword: false }]
    await db.loadTree('o1')
    expect(ipc.dbOraTree).toHaveBeenCalledWith('o1')
    render(DbTree, { connectionId: 'o1', kind: 'oracle' })
    await fireEvent.click(screen.getByText('HR'))
    await fireEvent.click(screen.getByText('EMP'))
    expect(db.pendingQuery).toEqual({ connectionId: 'o1', kind: 'oracle', schema: 'HR', table: 'EMP' })
  })

  it('un errore oracle_client alla connessione apre la guida', async () => {
    db.connections = [{ id: 'o1', kind: 'oracle', name: 'x', host: 'h', port: 1521, user: null, database: 'S', tls: 'disable', acceptInvalidCerts: false, sshServerId: null, hasPassword: false }]
    vi.mocked(ipc.dbConnect).mockRejectedValueOnce('oracle_client')
    vi.mocked(ipc.dbOracleClientStatus).mockResolvedValue({ dir: null, loaded: false, detected: [], error: null })
    await db.connect('o1')
    expect(db.guideFor).toBe('o1')
    db.guideFor = null
    // errore di rete con client caricato: niente guida
    vi.mocked(ipc.dbConnect).mockRejectedValueOnce('rifiutata')
    vi.mocked(ipc.dbOracleClientStatus).mockResolvedValue({ dir: '/x', loaded: true, detected: [], error: null })
    await db.connect('o1')
    expect(db.guideFor).toBeNull()
  })

  it('renders Postgres schemas and tables as text and opens a table on click', async () => {
    db.connections = [{ id: 'c1', kind: 'postgres', name: 'x', host: 'h', port: 5432, user: null, database: null, tls: 'prefer', acceptInvalidCerts: false, sshServerId: null, hasPassword: false }]
    await db.loadTree('c1')
    const { container } = render(DbTree, { connectionId: 'c1', kind: 'postgres' })
    await fireEvent.click(screen.getByText('s"; DROP'))
    expect(container.querySelector('img')).toBeNull()
    await fireEvent.click(screen.getByText(HOSTILE))
    expect(db.pendingQuery).toEqual({ connectionId: 'c1', kind: 'postgres', schema: 's"; DROP', table: HOSTILE })
  })

  it('renders Mongo databases and collections as text and opens a collection', async () => {
    db.connections = [{ id: 'm1', kind: 'mongo', name: 'x', host: 'h', port: 27017, user: null, database: null, tls: 'prefer', acceptInvalidCerts: false, sshServerId: null, hasPassword: false }]
    await db.loadTree('m1')
    const { container } = render(DbTree, { connectionId: 'm1', kind: 'mongo' })
    await fireEvent.click(screen.getByText(HOSTILE))
    expect(container.querySelector('img, b')).toBeNull()
    await fireEvent.click(screen.getByText('<b>c</b>'))
    expect(db.pendingQuery).toEqual({ connectionId: 'm1', kind: 'mongo', database: HOSTILE, collection: '<b>c</b>' })
  })

  it('context menu: new query drops the table; a disabled tree opens nothing', async () => {
    db.connections = [{ id: 'c1', kind: 'postgres', name: 'x', host: 'h', port: 5432, user: null, database: null, tls: 'prefer', acceptInvalidCerts: false, sshServerId: null, hasPassword: false }]
    await db.loadTree('c1')
    const { rerender } = render(DbTree, { connectionId: 'c1', kind: 'postgres' })
    await fireEvent.click(screen.getByText('s"; DROP'))
    await fireEvent.contextMenu(screen.getByText('v1'))
    await fireEvent.click(screen.getByRole('menuitem', { name: 'Nuova query' }))
    expect(db.pendingQuery).toEqual({ connectionId: 'c1', kind: 'postgres', schema: 's"; DROP' })
    db.pendingQuery = null
    await rerender({ connectionId: 'c1', kind: 'postgres', disabled: true })
    await fireEvent.click(screen.getByText('v1'))
    expect(db.pendingQuery).toBeNull()
  })
})
