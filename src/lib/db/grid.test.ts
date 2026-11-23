import { describe, expect, it } from 'vitest'
import { ROW_H, appendPage, clampWidth, emptyGrid, estimateWidth, fmtInt, mergeColumns, nearEnd, quoteIdent, selectAllSql, visibleColumns, visibleRows } from './grid'

describe('finestra visibile', () => {
  it('righe: dal primo visibile con margine, limitato ai confini', () => {
    expect(visibleRows(0, 240, 1000, 0)).toEqual({ start: 0, end: 10 })
    expect(visibleRows(ROW_H * 100, 240, 1000, 5)).toEqual({ start: 95, end: 115 })
    expect(visibleRows(ROW_H * 995, 240, 1000, 5)).toEqual({ start: 990, end: 1000 })
    expect(visibleRows(0, 240, 0)).toEqual({ start: 0, end: 0 })
  })
  it('colonne: somme cumulative e finestra', () => {
    const w = [100, 100, 100, 100, 100]
    const v = visibleColumns(250, 120, w, 0)
    expect(v.offsets).toEqual([0, 100, 200, 300, 400])
    expect(v.total).toBe(500)
    expect([v.start, v.end]).toEqual([2, 4])
    expect(visibleColumns(0, 50, w, 1)).toMatchObject({ start: 0, end: 2 })
  })
})

describe('larghezze', () => {
  it('stima da intestazione e contenuto, entro 60 e 400', () => {
    expect(estimateWidth('id', [['1']], 0)).toBe(60)
    expect(estimateWidth('x', [['a'.repeat(500)]], 0)).toBe(400)
    expect(estimateWidth('nome', [['Maria Rossi']], 0)).toBeGreaterThan(60)
    expect(clampWidth(10)).toBe(60)
    expect(clampWidth(999)).toBe(400)
  })
})

describe('mongo', () => {
  it('unisce le colonne con _id per prima e ordine di comparsa', () => {
    expect(mergeColumns(['a', 'b'], ['b', 'c', '_id'])).toEqual(['_id', 'a', 'b', 'c'])
    expect(mergeColumns([], ['x', '_id'])).toEqual(['_id', 'x'])
  })
  it('riallinea righe per nome e tiene le celle troncate', () => {
    let g = appendPage(emptyGrid(), { columns: [{ name: '_id' }, { name: 'a' }], rows: [['1', 'x']], truncated: [[0, 1]] }, true)
    g = appendPage(g, { columns: [{ name: '_id' }, { name: 'b' }, { name: 'a' }], rows: [['2', 'y', null]], truncated: [[0, 1]] }, true)
    expect(g.columns).toEqual(['_id', 'a', 'b'])
    expect(g.rows).toEqual([['1', 'x'], ['2', null, 'y']]) // righe vecchie più corte: le celle mancanti valgono null (cellValue)
    expect([...g.truncated].sort()).toEqual(['0:1', '1:2'])
  })
  it('_id che arriva dopo: righe e troncamenti si spostano', () => {
    let g = appendPage(emptyGrid(), { columns: [{ name: 'a' }], rows: [['x']], truncated: [[0, 0]] }, true)
    g = appendPage(g, { columns: [{ name: '_id' }, { name: 'a' }], rows: [['7', 'z']] }, true)
    expect(g.columns).toEqual(['_id', 'a'])
    expect(g.rows).toEqual([[null, 'x'], ['7', 'z']])
    expect(g.truncated.has('0:1')).toBe(true)
  })
})

describe('postgres', () => {
  it('accoda le pagine senza cambiare le colonne e sposta i troncamenti', () => {
    let g = appendPage(emptyGrid(), { columns: [{ name: 'a' }, { name: 'a' }], rows: [['1', null]] }, false)
    g = appendPage(g, { rows: [['2', '3'], ['4', '5']], truncated: [[1, 1]] }, false)
    expect(g.columns).toEqual(['a', 'a'])
    expect(g.rows).toHaveLength(3)
    expect(g.truncated.has('2:1')).toBe(true)
  })
})

describe('varie', () => {
  it('nearEnd entro 10 righe dalla fine', () => {
    expect(nearEnd(490, 500)).toBe(true)
    expect(nearEnd(480, 500)).toBe(false)
    expect(nearEnd(0, 0)).toBe(false)
  })
  it('cita gli identificatori raddoppiando i doppi apici', () => {
    expect(quoteIdent('a"b')).toBe('"a""b"')
    expect(selectAllSql('public', 'u"x')).toBe('SELECT * FROM "public"."u""x";')
  })
  it('formatta i numeri', () => {
    expect(fmtInt(100000)).toBe('100.000')
    expect(fmtInt(12)).toBe('12')
  })
})
