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
})
