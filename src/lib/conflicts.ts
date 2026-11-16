/** Un conflitto di unione: righe 1-based dei marcatori; `ours`/`theirs` sono i testi dei due lati, senza l'ultimo a capo. */
export type Conflict = { start: number; mid: number; end: number; base?: number; ours: string; theirs: string }

export type Pick = 'ours' | 'theirs' | 'both'

/** Righe con il loro terminatore (`\n` o `\r\n`), così il testo si ricompone identico. */
const split = (text: string): string[] => text.match(/[^\n]*\n|[^\n]+$/g) ?? []
const bare = (line: string) => line.replace(/\r?\n$/, '')
const body = (lines: string[]) => lines.join('').replace(/\r?\n$/, '')

const isMarker = (line: string, mark: string) => {
  const b = bare(line)
  return b === mark || b.startsWith(mark + ' ')
}
const isMid = (line: string) => bare(line) === '======='

/** Trova i conflitti (marcatori `<<<<<<<`, `=======`, `>>>>>>>`, con `|||||||` per lo stile diff3). Marcatori sbilanciati o fuori ordine si ignorano. */
export function parseConflicts(text: string): Conflict[] {
  const lines = split(text)
  const out: Conflict[] = []
  let start = -1
  let base = -1
  let mid = -1
  for (let i = 0; i < lines.length; i++) {
    const l = lines[i]
    if (isMarker(l, '<<<<<<<')) {
      start = i
      base = mid = -1
    } else if (start >= 0 && mid < 0 && base < 0 && isMarker(l, '|||||||')) {
      base = i
    } else if (start >= 0 && mid < 0 && isMid(l)) {
      mid = i
    } else if (start >= 0 && mid >= 0 && isMarker(l, '>>>>>>>')) {
      const oursEnd = base >= 0 ? base : mid
      const c: Conflict = { start: start + 1, mid: mid + 1, end: i + 1, ours: body(lines.slice(start + 1, oursEnd)), theirs: body(lines.slice(mid + 1, i)) }
      if (base >= 0) c.base = base + 1
      out.push(c)
      start = -1
      base = mid = -1
    }
  }
  return out
}

/** Sostituisce il blocco del conflitto con il lato scelto, mantenendo i fine riga del file. */
export function resolve(text: string, c: Conflict, pick: Pick): string {
  const lines = split(text)
  const oursEnd = (c.base ?? c.mid) - 1
  const ours = lines.slice(c.start, oursEnd)
  const theirs = lines.slice(c.mid, c.end - 1)
  const kept = pick === 'ours' ? ours : pick === 'theirs' ? theirs : [...ours, ...theirs]
  return [...lines.slice(0, c.start - 1), ...kept, ...lines.slice(c.end)].join('')
}
