import { describe, expect, it } from 'vitest'
import { relativeTime } from './time'

const MIN = 60_000
// 2026-11-11 (mercoledì) alle 15:00 locali
const now = new Date(2026, 10, 11, 15, 0, 0).getTime()

describe('relativeTime', () => {
  it('ora sotto il minuto (anche nel futuro lieve)', () => {
    expect(relativeTime(now - 20_000, now)).toBe('ora')
    expect(relativeTime(now + 5_000, now)).toBe('ora')
  })
  it('minuti', () => {
    expect(relativeTime(now - MIN, now)).toBe('1 min fa')
    expect(relativeTime(now - 2 * MIN, now)).toBe('2 min fa')
    expect(relativeTime(now - 59 * MIN, now)).toBe('59 min fa')
  })
  it('ore, nello stesso giorno', () => {
    expect(relativeTime(now - 60 * MIN, now)).toBe('1 ora fa')
    expect(relativeTime(now - 3 * 60 * MIN, now)).toBe('3 ore fa')
  })
  it('ieri', () => {
    expect(relativeTime(new Date(2026, 10, 10, 23, 30).getTime(), now)).toBe('ieri')
    expect(relativeTime(new Date(2026, 10, 10, 1, 0).getTime(), now)).toBe('ieri')
  })
  it('giorno della settimana entro 6 giorni', () => {
    expect(relativeTime(new Date(2026, 10, 9, 10).getTime(), now)).toBe('lun')
    expect(relativeTime(new Date(2026, 10, 5, 10).getTime(), now)).toBe('gio')
  })
  it('data oltre la settimana', () => {
    expect(relativeTime(new Date(2026, 9, 20, 10).getTime(), now)).toBe('20 ott')
  })
  it('timestamp 0 o assente = nessuna etichetta', () => {
    expect(relativeTime(0, now)).toBe('')
  })
})
