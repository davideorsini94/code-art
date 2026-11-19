export type NewMenuItem = { kind: 'folder' | 'repo' | 'file' | 'other'; path?: string }

/** Cartella che contiene `path` (la radice resta "/"). */
export function parentDir(path: string): string {
  const i = path.replace(/\/+$/, '').lastIndexOf('/')
  return i <= 0 ? '/' : path.slice(0, i)
}

/** Voci del menu ＋: cartelle del progetto, repository (senza doppioni), cartella del file aperto, "Altra cartella…". */
export function buildNewMenu(o: { folders: string[]; repos: string[]; activeFile: string | null }): NewMenuItem[] {
  const seen = new Set<string>()
  const items: NewMenuItem[] = []
  const add = (kind: 'folder' | 'repo' | 'file', path: string) => {
    if (seen.has(path)) return
    seen.add(path)
    items.push({ kind, path })
  }
  o.folders.forEach((f) => add('folder', f))
  o.repos.forEach((r) => add('repo', r))
  if (o.activeFile) add('file', parentDir(o.activeFile))
  items.push({ kind: 'other' })
  return items
}

/** Vero se l'unica scelta possibile è la cartella principale (nessun menu da mostrare). */
export function onlyPrimary(items: NewMenuItem[]): boolean {
  const real = items.filter((i) => i.kind !== 'other')
  return real.length === 1 && real[0].kind === 'folder'
}

/** Nome scelto dall'utente: vuoto = ritorna al titolo automatico. */
export function normalizeCustom(text: string): string | null {
  const t = text.trim()
  return t === '' ? null : t
}

/** Posizione d'inserimento (0..n, tra le schede) → indice finale per `moveTab`. */
export function slotToIndex(slot: number, from: number): number {
  return slot > from ? slot - 1 : slot
}
