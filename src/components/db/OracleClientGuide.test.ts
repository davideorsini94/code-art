// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte'
import { afterEach, describe, expect, it, vi } from 'vitest'

vi.mock('../../lib/events', () => ({ listenDbState: vi.fn(async () => () => {}), listenSshState: vi.fn(async () => () => {}), listenProject: vi.fn(async () => () => {}) }))
vi.mock('../../lib/ipc', () => ({
  dbOracleClientStatus: vi.fn(),
  dbOracleClientPick: vi.fn(),
  dbOracleClientUse: vi.fn(),
  dbConnect: vi.fn(async () => {}),
  dbOraTree: vi.fn(async () => ({ schemas: [] })),
}))
vi.mock('../../lib/external', () => ({ openExternal: vi.fn(() => true) }))

import * as ipc from '../../lib/ipc'
import { openExternal } from '../../lib/external'
import { db } from '../../lib/stores/db.svelte'
import OracleClientGuide from './OracleClientGuide.svelte'

const st = (o = {}) => ({ dir: null, loaded: false, detected: [], error: null, ...o })

afterEach(() => {
  cleanup()
  vi.clearAllMocks()
  db.oracleClient = null
  db.guideFor = null
  db.connections = []
})

describe('OracleClientGuide', () => {
  it('mostra i passi e apre la pagina di Oracle con openExternal', async () => {
    db.oracleClient = st()
    render(OracleClientGuide)
    expect(screen.getByText(/Basic Light/)).toBeTruthy()
    expect(screen.getByText('install_ic.sh')).toBeTruthy()
    await fireEvent.click(screen.getByRole('button', { name: 'pagina di download' }))
    expect(openExternal).toHaveBeenCalledWith('https://www.oracle.com/database/technologies/instant-client/macos-arm64-downloads.html')
  })

  it('selettore: aggiorna lo stato e riprova la connessione in attesa', async () => {
    db.connections = [{ id: 'o1', kind: 'oracle' } as never]
    db.oracleClient = st()
    db.guideFor = 'o1'
    vi.mocked(ipc.dbOracleClientPick).mockResolvedValue(st({ dir: '/ic', loaded: true }))
    render(OracleClientGuide, { modal: true })
    await fireEvent.click(screen.getByRole('button', { name: 'Scegli la cartella…' }))
    await waitFor(() => expect(ipc.dbConnect).toHaveBeenCalledWith('o1'))
    expect(db.guideFor).toBeNull()
    expect(screen.getByText('Caricato')).toBeTruthy()
  })

  it('selettore annullato: nessun cambiamento', async () => {
    db.oracleClient = st()
    vi.mocked(ipc.dbOracleClientPick).mockResolvedValue(null)
    render(OracleClientGuide)
    await fireEvent.click(screen.getByRole('button', { name: 'Scegli la cartella…' }))
    expect(db.oracleClient).toEqual(st())
    expect(ipc.dbConnect).not.toHaveBeenCalled()
  })

  it('le cartelle trovate sono pulsanti che usano dbOracleClientUse', async () => {
    db.oracleClient = st({ detected: ['/opt/oracle/ic'] })
    vi.mocked(ipc.dbOracleClientUse).mockResolvedValue(st({ dir: '/opt/oracle/ic', detected: ['/opt/oracle/ic'] }))
    render(OracleClientGuide)
    await fireEvent.click(screen.getByRole('button', { name: '/opt/oracle/ic' }))
    expect(ipc.dbOracleClientUse).toHaveBeenCalledWith('/opt/oracle/ic')
    await screen.findByText('Trovato')
  })

  it('errore di quarantena: rimedio e comando copiabile', async () => {
    const cmd = 'xattr -dr com.apple.quarantine "/ic"'
    db.oracleClient = st({ dir: '/ic', error: `macOS ha messo in quarantena la libreria: esegui \`${cmd}\` nel Terminale e riprova` })
    const writeText = vi.fn(async () => {})
    Object.assign(navigator, { clipboard: { writeText } })
    render(OracleClientGuide)
    expect(screen.getByText('Errore')).toBeTruthy()
    await fireEvent.click(screen.getByRole('button', { name: 'Copia' }))
    expect(writeText).toHaveBeenCalledWith(cmd)
  })

  it('nota del riavvio quando serve cambiare cartella', () => {
    db.oracleClient = st({ dir: '/b', error: 'riavvia l\'app per usare un\'altra cartella' })
    render(OracleClientGuide)
    expect(screen.getByText(/serve riavviare l'app/)).toBeTruthy()
  })
})
