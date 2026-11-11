const MIN = 60_000
const WEEKDAYS = ['dom', 'lun', 'mar', 'mer', 'gio', 'ven', 'sab']
const MONTHS = ['gen', 'feb', 'mar', 'apr', 'mag', 'giu', 'lug', 'ago', 'set', 'ott', 'nov', 'dic']

const startOfDay = (t: number) => {
  const d = new Date(t)
  return new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime()
}

/** Etichetta relativa italiana ("ora", "2 min fa", "ieri", "lun", "20 ott"). `ts` in millisecondi; 0 = sconosciuto. */
export function relativeTime(ts: number, now: number): string {
  if (!ts) return ''
  const diff = now - ts
  if (diff < MIN) return 'ora'
  if (diff < 60 * MIN) return `${Math.floor(diff / MIN)} min fa`
  const days = Math.round((startOfDay(now) - startOfDay(ts)) / 86_400_000)
  if (days <= 0) {
    const h = Math.floor(diff / (60 * MIN))
    return h === 1 ? '1 ora fa' : `${h} ore fa`
  }
  if (days === 1) return 'ieri'
  const d = new Date(ts)
  if (days < 7) return WEEKDAYS[d.getDay()]
  return `${d.getDate()} ${MONTHS[d.getMonth()]}`
}
