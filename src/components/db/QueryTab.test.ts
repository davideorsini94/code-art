// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

vi.mock('../../lib/events', () => ({ listenDbState: vi.fn(async () => () => {}), listenSshState: vi.fn(async () => () => {}), listenProject: vi.fn(async () => () => {}) }))
vi.mock('../../lib/ipc', () => ({
  dbQuery: vi.fn(),
  dbQueryMore: vi.fn(),
  dbQueryClose: vi.fn(async () => {}),
  dbQueryCancel: vi.fn(async () => {}),
  dbMongoFind: vi.fn(),
  dbMongoAggregate: vi.fn(),
  dbExport: vi.fn(),
  dbHistory: vi.fn(async () => []),
  dbHistoryAdd: vi.fn(async () => {}),
  dbConnect: vi.fn(async () => {}),
  dbPgTree: vi.fn(async () => ({ schemas: [] })),
  dbMongoTree: vi.fn(async () => []),
}))
const cmds = vi.hoisted(() => ({ list: [] as (() => void)[], eds: [] as { setValue: (v: string) => void; getValue: () => string }[] }))
vi.mock('../../lib/monaco', () => ({
  monaco: {
    KeyMod: { CtrlCmd: 1 },
    KeyCode: { Enter: 3 },
    editor: {
      create: (_el: HTMLElement, o: { value: string }) => {
        let v = o.value
        const subs: (() => void)[] = []
        const ed = {
          getValue: () => v,
          setValue: (x: string) => { v = x; subs.forEach((f) => f()) },
          onDidChangeModelContent: (f: () => void) => { subs.push(f); return { dispose() {} } },
          addCommand: (_k: number, f: () => void) => { cmds.list.push(f) },
          dispose() {},
        }
        cmds.eds.push(ed)
        return ed
      },
    },
  },
}))
vi.mock('../../lib/stores/project.svelte', () => ({ projectStore: { project: { id: 'p1', folders: [] }, untrusted: [] } }))

import * as ipc from '../../lib/ipc'
import { db } from '../../lib/stores/db.svelte'
import { editor } from '../../lib/stores/editor.svelte'
import QueryTab from './QueryTab.svelte'

const ok = (rows: (string | null)[][] = [['1']], extra = {}) => ({ resultId: 7, columns: [{ name: 'a' }], rows, hasMore: false, ...extra })

function open(req: Parameters<typeof editor.openQuery>[0]) {
  editor.openQuery(req)
  return editor.activeTab!
}

beforeEach(() => {
  localStorage.clear()
  db.connections = [{ id: 'c1', name: 'Locale', kind: 'postgres' } as never, { id: 'm1', name: 'Mongo', kind: 'mongo' } as never]
  db.states = { c1: { state: 'connected' }, m1: { state: 'connected' } }
  vi.mocked(ipc.dbQuery).mockResolvedValue(ok() as never)
})
afterEach(() => {
  cleanup()
  vi.clearAllMocks()
  cmds.list.length = 0
  cmds.eds.length = 0
  editor.tabs = []
  editor.active = null
  db.states = {}
})

describe('QueryTab', () => {
  it('aprendo da una tabella esegue SELECT con identificatori citati', async () => {
    const tab = open({ connectionId: 'c1', kind: 'postgres', schema: 'pub"lic', table: 'u"sers' })
    render(QueryTab, { tab })
    await waitFor(() => expect(ipc.dbQuery).toHaveBeenCalledWith('c1', 'SELECT * FROM "pub""lic"."u""sers";'))
    await waitFor(() => expect(screen.getByText('1 riga')).toBeTruthy())
    expect(ipc.dbHistoryAdd).toHaveBeenCalledWith('c1', 'SELECT * FROM "pub""lic"."u""sers";')
    expect(editor.tabs[0].query?.resultId).toBe(7)
  })

  it('una query nuova non parte da sola; ⌘↵ (tastiera e comando dell\'editor) esegue', async () => {
    const tab = open({ connectionId: 'c1', kind: 'postgres' })
    const { container } = render(QueryTab, { tab })
    await waitFor(() => expect(cmds.eds.length).toBe(1))
    expect(ipc.dbQuery).not.toHaveBeenCalled()
    cmds.eds[0].setValue('select 1')
    await fireEvent.keyDown(container.querySelector('.qt')!, { key: 'Enter', metaKey: true })
    await waitFor(() => expect(ipc.dbQuery).toHaveBeenCalledWith('c1', 'select 1'))
    cmds.list[0]()
    await waitFor(() => expect(ipc.dbQuery).toHaveBeenCalledTimes(2))
  })

  it('durante l\'esecuzione "Annulla" chiama dbQueryCancel', async () => {
    let done: (v: unknown) => void = () => {}
    vi.mocked(ipc.dbQuery).mockReturnValue(new Promise((r) => (done = r)) as never)
    const tab = open({ connectionId: 'c1', kind: 'postgres' })
    render(QueryTab, { tab })
    await waitFor(() => expect(cmds.eds.length).toBe(1))
    cmds.eds[0].setValue('select pg_sleep(9)')
    expect(screen.queryByRole('button', { name: 'Annulla' })).toBeNull()
    await fireEvent.click(screen.getByRole('button', { name: /Esegui/ }))
    await fireEvent.click(await screen.findByRole('button', { name: 'Annulla' }))
    expect(ipc.dbQueryCancel).toHaveBeenCalledWith('c1')
    done(ok())
    await waitFor(() => expect(screen.queryByRole('button', { name: 'Annulla' })).toBeNull())
  })

  it('un rifiuto "annullato" non mostra errori', async () => {
    vi.mocked(ipc.dbQuery).mockRejectedValue('Operazione annullata')
    const tab = open({ connectionId: 'c1', kind: 'postgres' })
    render(QueryTab, { tab })
    await waitFor(() => expect(cmds.eds.length).toBe(1))
    cmds.eds[0].setValue('delete from t')
    await fireEvent.click(screen.getByRole('button', { name: /Esegui/ }))
    await waitFor(() => expect(screen.getByText('Esecuzione annullata')).toBeTruthy())
    expect(screen.queryByRole('alert')).toBeNull()
  })

  it('la cronologia sostituisce il testo dell\'editor', async () => {
    vi.mocked(ipc.dbHistory).mockResolvedValue([{ id: 1, text: 'select vecchia', atMs: 1 }, { id: 2, text: 'select nuova', atMs: 2 }])
    const tab = open({ connectionId: 'c1', kind: 'postgres' })
    render(QueryTab, { tab })
    await waitFor(() => expect(cmds.eds.length).toBe(1))
    await fireEvent.click(screen.getByRole('button', { name: /Cronologia/ }))
    const items = await screen.findAllByRole('option')
    expect(items.map((i) => i.textContent)).toEqual(['select nuova', 'select vecchia']) // più recente prima
    await fireEvent.click(items[1])
    expect(cmds.eds[0].getValue()).toBe('select vecchia')
    expect(screen.queryByRole('option')).toBeNull()
  })

  it('il testo si salva per connessione e si ripristina; il testo ripristinato non parte da solo', async () => {
    const tab = open({ connectionId: 'c1', kind: 'postgres', schema: 'public', table: 't' })
    const r = render(QueryTab, { tab })
    await waitFor(() => expect(cmds.eds.length).toBe(1))
    cmds.eds[0].setValue('select 42')
    await waitFor(() => expect(localStorage.getItem('codeart.dbquery.c1.t:public.t')).toBe('select 42'))
    r.unmount()
    vi.mocked(ipc.dbQuery).mockClear()
    cmds.eds.length = 0
    render(QueryTab, { tab: { ...tab, query: { ...tab.query!, autoRun: true } } })
    await waitFor(() => expect(cmds.eds.length).toBe(1))
    expect(cmds.eds[0].getValue()).toBe('select 42')
    expect(ipc.dbQuery).not.toHaveBeenCalled()
  })

  it('Esporta usa il resultId e mostra l\'esito; annullare il selettore non fa nulla', async () => {
    vi.mocked(ipc.dbExport).mockResolvedValueOnce(null).mockResolvedValueOnce({ path: '/tmp/r.csv', rows: 1234 })
    const tab = open({ connectionId: 'c1', kind: 'postgres', schema: 'public', table: 't' })
    render(QueryTab, { tab })
    const csv = await screen.findByRole('button', { name: /CSV/ })
    await waitFor(() => expect((csv as HTMLButtonElement).disabled).toBe(false))
    await fireEvent.click(csv)
    expect(ipc.dbExport).toHaveBeenCalledWith(7, 'csv')
    expect((csv as HTMLButtonElement).disabled).toBe(false)
    await fireEvent.click(csv)
    await waitFor(() => expect(screen.getByText(/Esportate 1\.234 righe in \/tmp\/r\.csv/)).toBeTruthy())
    expect((csv as HTMLButtonElement).disabled).toBe(true)
    expect(screen.getByRole('button', { name: /JSON/ })).toBeTruthy()
  })

  it('un risultato corto con altre righe carica da solo la pagina successiva e la accoda', async () => {
    vi.mocked(ipc.dbQuery).mockResolvedValue(ok([['1']], { hasMore: true }) as never)
    vi.mocked(ipc.dbQueryMore).mockResolvedValue({ rows: [['2']], hasMore: false })
    const tab = open({ connectionId: 'c1', kind: 'postgres', schema: 'public', table: 't' })
    render(QueryTab, { tab })
    await waitFor(() => expect(screen.getByText('2 righe')).toBeTruthy())
    expect(ipc.dbQueryMore).toHaveBeenCalledWith(7)
  })

  it('senza connessione mostra "Connessione chiusa" con Riconnetti e tiene il testo', async () => {
    db.states = { c1: { state: 'disconnected' } }
    const tab = open({ connectionId: 'c1', kind: 'postgres' })
    render(QueryTab, { tab })
    await waitFor(() => expect(cmds.eds.length).toBe(1))
    cmds.eds[0].setValue('select 1')
    await fireEvent.click(screen.getByRole('button', { name: /Esegui/ }))
    expect(screen.getByText(/Connessione chiusa/)).toBeTruthy()
    expect(ipc.dbQuery).not.toHaveBeenCalled()
    await fireEvent.click(screen.getByRole('button', { name: 'Riconnetti' }))
    expect(ipc.dbConnect).toHaveBeenCalledWith('c1')
    expect(cmds.eds[0].getValue()).toBe('select 1')
  })

  it('istruzioni senza risultato: righe interessate e avvisi', async () => {
    vi.mocked(ipc.dbQuery).mockResolvedValue({ columns: [], rows: [], hasMore: false, affected: [3, 1], notices: ['NOTICE: fatto'] } as never)
    const tab = open({ connectionId: 'c1', kind: 'postgres' })
    render(QueryTab, { tab })
    await waitFor(() => expect(cmds.eds.length).toBe(1))
    cmds.eds[0].setValue('update a; update b')
    await fireEvent.click(screen.getByRole('button', { name: /Esegui/ }))
    await waitFor(() => expect(screen.getByText('Istruzione 1: 3 righe interessate')).toBeTruthy())
    expect(screen.getByText('Istruzione 2: 1 riga interessata')).toBeTruthy()
    expect(screen.getByText('NOTICE: fatto')).toBeTruthy()
  })

  it('MongoDB: da una collezione esegue la ricerca con filtro vuoto', async () => {
    vi.mocked(ipc.dbMongoFind).mockResolvedValue(ok([['{"x":1}']], { columns: [{ name: '_id' }] }) as never)
    const tab = open({ connectionId: 'm1', kind: 'mongo', database: 'app', collection: 'users' })
    render(QueryTab, { tab })
    await waitFor(() => expect(ipc.dbMongoFind).toHaveBeenCalledWith('m1', 'app', 'users', '', '', ''))
    await fireEvent.click(screen.getByRole('radio', { name: 'Aggregazione' }))
    await waitFor(() => expect(cmds.eds.length).toBeGreaterThan(0))
    await fireEvent.click(screen.getByRole('button', { name: /Esegui/ }))
    await waitFor(() => expect(ipc.dbMongoAggregate).toHaveBeenCalledWith('m1', 'app', 'users', '[\n  \n]'))
  })
})
