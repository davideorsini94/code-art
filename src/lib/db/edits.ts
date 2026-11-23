// Modifiche in sospeso della griglia Postgres: logica pura (niente DOM), immutabile. Le colonne si indicano per indice
// (i nomi del risultato possono ripetersi); `toRowEdits` produce le modifiche per il core (valori solo testo, `null` = NULL).
import type { PgRowEdit } from '../types'
import type { Cell } from './grid'

/** Valore di una riga nuova: `undefined` = non specificato (il database usa il valore predefinito). */
export type NewCell = string | null | undefined
export type PendingEdits = {
  /** Per chiave di riga (valori della chiave primaria della riga originale). */
  updates: ReadonlyMap<string, { original: Cell[]; changes: ReadonlyMap<number, string | null> }>
  inserts: readonly NewCell[][]
  /** Per chiave di riga, con la riga originale. */
  deletes: ReadonlyMap<string, Cell[]>
}

export const emptyEdits = (): PendingEdits => ({ updates: new Map(), inserts: [], deletes: new Map() })

export const rowKey = (row: Cell[], pk: number[]) => JSON.stringify(pk.map((i) => row[i] ?? null))

export const editCount = (e: PendingEdits) => e.updates.size + e.inserts.length + e.deletes.size

/** Cambia una cella di una riga esistente. Riportarla al valore originale elimina la modifica. Righe eliminate: nessun effetto. */
export function setCell(e: PendingEdits, row: Cell[], pk: number[], col: number, value: string | null): PendingEdits {
  const key = rowKey(row, pk)
  if (e.deletes.has(key)) return e
  const updates = new Map(e.updates)
  const changes = new Map(updates.get(key)?.changes ?? [])
  if (value === (row[col] ?? null)) changes.delete(col)
  else changes.set(col, value)
  if (changes.size === 0) updates.delete(key)
  else updates.set(key, { original: row, changes })
  return { ...e, updates }
}

/** Valore da mostrare: la modifica in sospeso, se c'è. */
export function currentValue(e: PendingEdits, row: Cell[], pk: number[], col: number): Cell {
  const ch = e.updates.get(rowKey(row, pk))?.changes
  return ch && ch.has(col) ? ch.get(col)! : (row[col] ?? null)
}
export const isChanged = (e: PendingEdits, row: Cell[], pk: number[], col: number) => e.updates.get(rowKey(row, pk))?.changes.has(col) ?? false
export const isDeleted = (e: PendingEdits, row: Cell[], pk: number[]) => e.deletes.has(rowKey(row, pk))

/** Segna (o toglie il segno) di riga eliminata; eliminare scarta le modifiche della riga. */
export function toggleDelete(e: PendingEdits, row: Cell[], pk: number[]): PendingEdits {
  const key = rowKey(row, pk)
  const deletes = new Map(e.deletes)
  if (deletes.has(key)) {
    deletes.delete(key)
    return { ...e, deletes }
  }
  const updates = new Map(e.updates)
  updates.delete(key)
  deletes.set(key, row)
  return { ...e, updates, deletes }
}

export const addInsert = (e: PendingEdits, cols: number): PendingEdits => ({ ...e, inserts: [new Array<NewCell>(cols).fill(undefined), ...e.inserts] })
export function setInsertCell(e: PendingEdits, i: number, col: number, value: string | null): PendingEdits {
  if (i < 0 || i >= e.inserts.length) return e
  const inserts = e.inserts.slice()
  const row = inserts[i].slice()
  row[col] = value
  inserts[i] = row
  return { ...e, inserts }
}
/** Eliminare una riga nuova la scarta e basta. */
export const removeInsert = (e: PendingEdits, i: number): PendingEdits => ({ ...e, inserts: e.inserts.filter((_, k) => k !== i) })

/** Modifiche per il core: prima gli inserimenti, poi gli aggiornamenti, poi le eliminazioni. */
export function toRowEdits(e: PendingEdits, columns: string[], pk: number[]): PgRowEdit[] {
  const out: PgRowEdit[] = []
  for (const row of e.inserts) {
    const values: [string, string | null][] = []
    row.forEach((v, i) => {
      if (v !== undefined) values.push([columns[i], v])
    })
    out.push({ kind: 'insert', values })
  }
  for (const u of e.updates.values()) {
    out.push({ kind: 'update', pk: pk.map((i) => [columns[i], u.original[i] ?? null]), set: [...u.changes].map(([i, v]) => [columns[i], v]) })
  }
  for (const row of e.deletes.values()) out.push({ kind: 'delete', pk: pk.map((i) => [columns[i], row[i] ?? null]) })
  return out
}
