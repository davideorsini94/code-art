import { describe, expect, it } from 'vitest'
import { addInsert, currentValue, editCount, emptyEdits, isChanged, isDeleted, removeInsert, rowKey, setCell, setInsertCell, toRowEdits, toggleDelete } from './edits'

const cols = ['id', 'name', 'note']
const row = ['1', 'Ada', null]

describe('modifiche in sospeso', () => {
  it('cambia e ripristina una cella', () => {
    let e = setCell(emptyEdits(), row, [0], 1, 'Bea')
    expect(editCount(e)).toBe(1)
    expect(currentValue(e, row, [0], 1)).toBe('Bea')
    expect(isChanged(e, row, [0], 1)).toBe(true)
    e = setCell(e, row, [0], 1, 'Ada')
    expect(editCount(e)).toBe(0)
    expect(isChanged(e, row, [0], 1)).toBe(false)
  })
  it('NULL e stringa vuota sono diversi', () => {
    let e = setCell(emptyEdits(), row, [0], 2, '')
    expect(isChanged(e, row, [0], 2)).toBe(true)
    e = setCell(e, row, [0], 2, null)
    expect(editCount(e)).toBe(0)
    e = setCell(emptyEdits(), row, [0], 1, null)
    expect(toRowEdits(e, cols, [0])[0]).toEqual({ kind: 'update', pk: [['id', '1']], set: [['name', null]] })
  })
  it('eliminare scarta le modifiche; togliere il segno le lascia scartate; righe eliminate non si modificano', () => {
    let e = setCell(emptyEdits(), row, [0], 1, 'Bea')
    e = toggleDelete(e, row, [0])
    expect(e.updates.size).toBe(0)
    expect(isDeleted(e, row, [0])).toBe(true)
    expect(setCell(e, row, [0], 1, 'X')).toBe(e)
    e = toggleDelete(e, row, [0])
    expect(editCount(e)).toBe(0)
  })
  it('righe nuove: valori non specificati omessi, eliminarle le scarta', () => {
    let e = addInsert(emptyEdits(), 3)
    e = setInsertCell(e, 0, 1, 'Cy')
    e = setInsertCell(e, 0, 2, null)
    expect(toRowEdits(e, cols, [0])).toEqual([{ kind: 'insert', values: [['name', 'Cy'], ['note', null]] }])
    e = addInsert(e, 3)
    expect(editCount(e)).toBe(2)
    e = removeInsert(e, 1)
    e = removeInsert(e, 0)
    expect(editCount(e)).toBe(0)
  })
  it('chiave composta e ordine: inserimenti, aggiornamenti, eliminazioni', () => {
    const r1 = ['a', 'b', 'x']
    const r2 = ['a', 'c', 'y']
    expect(rowKey(r1, [0, 1])).not.toBe(rowKey(r2, [0, 1]))
    let e = setCell(emptyEdits(), r1, [0, 1], 2, 'z')
    e = toggleDelete(e, r2, [0, 1])
    e = addInsert(e, 3)
    expect(toRowEdits(e, ['k1', 'k2', 'v'], [0, 1])).toEqual([
      { kind: 'insert', values: [] },
      { kind: 'update', pk: [['k1', 'a'], ['k2', 'b']], set: [['v', 'z']] },
      { kind: 'delete', pk: [['k1', 'a'], ['k2', 'c']] },
    ])
  })
})
