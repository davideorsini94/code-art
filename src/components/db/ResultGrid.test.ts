// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from '@testing-library/svelte'
import { afterEach, describe, expect, it, vi } from 'vitest'
import ResultGrid from './ResultGrid.svelte'

afterEach(cleanup)

function scrollTo(el: Element, top: number) {
  Object.defineProperty(el, 'scrollTop', { value: top, configurable: true })
  return fireEvent.scroll(el)
}

describe('ResultGrid', () => {
  it('NULL in corsivo grigio è diverso dalla stringa vuota', () => {
    const { container } = render(ResultGrid, { columns: ['a', 'b'], rows: [[null, ''], ['x', null]] })
    const cells = [...container.querySelectorAll('[role=gridcell]')]
    expect(cells).toHaveLength(4)
    expect(cells[0].querySelector('.null')?.textContent).toBe('NULL')
    expect(cells[1].textContent?.trim()).toBe('') // stringa vuota: cella presente, niente testo
    expect(cells[1].querySelector('.null')).toBeNull()
    expect(cells[2].textContent?.trim()).toBe('x')
  })

  it('segna le celle troncate', () => {
    const { container } = render(ResultGrid, { columns: ['a'], rows: [['aaa'], ['bbb']], truncated: new Set(['1:0']) })
    const badges = container.querySelectorAll('.trunc')
    expect(badges).toHaveLength(1)
    expect(badges[0].getAttribute('aria-label')).toBe('Valore troncato a 64 KB')
    expect(container.querySelectorAll('[role=gridcell]')[1].querySelector('.trunc')).not.toBeNull()
  })

  it('piè di pagina: conteggio, altre righe e limite', () => {
    const onmore = vi.fn()
    const { rerender } = render(ResultGrid, { columns: ['a'], rows: [['1']], hasMore: true, onmore })
    expect(screen.getByText('1 riga')).toBeTruthy()
    expect(screen.getByText(/Altre righe disponibili/)).toBeTruthy()
    expect(screen.getByRole('button', { name: 'Carica altre 500' })).toBeTruthy()
    return rerender({ columns: ['a'], rows: Array.from({ length: 100000 }, () => ['x']), hasMore: true, onmore }).then(() => {
      expect(screen.getByText('100.000 righe')).toBeTruthy()
      expect(screen.getByText(/Limite di 100\.000 righe raggiunto/)).toBeTruthy()
      expect(screen.queryByRole('button', { name: /Carica altre/ })).toBeNull()
    })
  })

  it('"Carica altre 500" chiede la pagina successiva', async () => {
    const onmore = vi.fn()
    render(ResultGrid, { columns: ['a'], rows: Array.from({ length: 600 }, (_, i) => [String(i)]), hasMore: true, onmore })
    await fireEvent.click(screen.getByRole('button', { name: 'Carica altre 500' }))
    expect(onmore).toHaveBeenCalledTimes(1)
  })

  it('virtualizza: disegna solo le righe visibili', () => {
    const { container } = render(ResultGrid, { columns: ['a'], rows: Array.from({ length: 5000 }, (_, i) => [String(i)]) })
    expect(container.querySelectorAll('[role=row]').length).toBeLessThan(60)
  })

  it('scorrere vicino alla fine chiede altre righe una volta sola', async () => {
    const onmore = vi.fn()
    const rows = Array.from({ length: 500 }, (_, i) => [String(i)])
    const { container, rerender } = render(ResultGrid, { columns: ['a'], rows, hasMore: true, onmore })
    const vp = container.querySelector('.viewport')!
    expect(onmore).not.toHaveBeenCalled()
    await scrollTo(vp, 24 * 495 - 400)
    await scrollTo(vp, 24 * 496 - 400)
    expect(onmore).toHaveBeenCalledTimes(1)
    // arrivate le righe nuove si può chiedere ancora
    await rerender({ columns: ['a'], rows: rows.concat(rows), hasMore: true, onmore })
    await scrollTo(vp, 24 * 995 - 400)
    expect(onmore).toHaveBeenCalledTimes(2)
  })

  it('clic seleziona la cella e ⌘C copia il testo', async () => {
    const writeText = vi.fn(async () => {})
    Object.defineProperty(navigator, 'clipboard', { value: { writeText }, configurable: true })
    const { container } = render(ResultGrid, { columns: ['a'], rows: [['ciao']] })
    await fireEvent.click(container.querySelector('[role=gridcell]')!)
    await fireEvent.keyDown(container.querySelector('.grid')!, { key: 'c', metaKey: true })
    expect(writeText).toHaveBeenCalledWith('ciao')
  })

  it('trascinare il bordo dell\'intestazione cambia la larghezza (minimo 60)', async () => {
    const { container } = render(ResultGrid, { columns: ['a'], rows: [['1']] })
    const grip = container.querySelector('.grip')!
    const head = () => container.querySelector('.hcell') as HTMLElement
    const w0 = parseInt(head().style.width)
    await fireEvent.pointerDown(grip, { clientX: 100, pointerId: 1 })
    await fireEvent.pointerMove(grip, { clientX: 150, pointerId: 1 })
    expect(parseInt(head().style.width)).toBe(Math.max(60, w0 + 50))
    await fireEvent.pointerMove(grip, { clientX: -500, pointerId: 1 })
    expect(parseInt(head().style.width)).toBe(60)
  })

  describe('modifica', () => {
    const pkProps = () => ({ columns: ['id', 'name'], rows: [['1', 'Ada'], ['2', null]], pk: [0] })
    it('Esc annulla, ∅ imposta NULL, celle e righe segnate', async () => {
      const onedit = vi.fn()
      const { container } = render(ResultGrid, { ...pkProps(), onedit })
      const cells = () => [...container.querySelectorAll('[role=gridcell]')] as HTMLElement[]
      await fireEvent.dblClick(cells()[1])
      const inp = screen.getByLabelText('Valore della cella') as HTMLInputElement
      expect(inp.value).toBe('Ada')
      await fireEvent.input(inp, { target: { value: 'X' } })
      await fireEvent.keyDown(inp, { key: 'Escape' })
      expect(screen.queryByLabelText('Valore della cella')).toBeNull()
      expect(onedit).not.toHaveBeenCalled()
      await fireEvent.dblClick(cells()[1])
      await fireEvent.click(screen.getByRole('button', { name: 'Imposta NULL' }))
      expect(onedit).toHaveBeenCalledTimes(1)
      const next = onedit.mock.calls[0][0]
      expect(next.updates.size).toBe(1)
      expect([...next.updates.values()][0].changes.get(1)).toBeNull()
    })
    it('evidenzia modifica, riga nuova ed eliminata', async () => {
      const edits = {
        updates: new Map([['["1"]', { original: ['1', 'Ada'], changes: new Map([[1, 'Zoe']]) }]]),
        inserts: [[undefined, 'Nuovo']],
        deletes: new Map([['["2"]', ['2', null]]]),
      }
      const { container } = render(ResultGrid, { ...pkProps(), edits, onedit: vi.fn() })
      expect(container.querySelectorAll('.cell.chg')).toHaveLength(1)
      expect(container.querySelectorAll('.row.ins')).toHaveLength(1)
      expect(container.querySelectorAll('.row.del')).toHaveLength(1)
      expect(screen.getByText('Zoe')).toBeTruthy()
    })
    it('＋ Riga aggiunge una riga nuova', async () => {
      const onedit = vi.fn()
      render(ResultGrid, { ...pkProps(), onedit })
      await fireEvent.click(screen.getByRole('button', { name: '＋ Riga' }))
      expect(onedit.mock.calls[0][0].inserts).toHaveLength(1)
    })
    it('senza pk non si modifica e la nota è visibile', async () => {
      render(ResultGrid, { columns: ['a'], rows: [['1']], note: 'Sola lettura: vista' })
      await fireEvent.dblClick(screen.getByText('1', { selector: '.txt' }))
      expect(screen.queryByLabelText('Valore della cella')).toBeNull()
      expect(screen.getByText(/Sola lettura: vista/)).toBeTruthy()
    })
  })
})
