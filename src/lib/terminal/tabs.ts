/** Nome dell'ultima cartella di `cwd` (lo slash finale è ignorato). */
export function folderLabel(cwd: string, _folders: string[]): string {
  const parts = cwd.split('/').filter(Boolean)
  return parts[parts.length - 1] ?? cwd
}

/** Titolo di una scheda: nome personalizzato, altrimenti cartella e branch. */
export function tabTitle(t: { cwd: string; branch: string | null; custom: string | null }, folders: string[]): string {
  if (t.custom) return t.custom
  const label = folderLabel(t.cwd, folders)
  return t.branch ? `${label} · ${t.branch}` : label
}

/** Nuovo array con l'elemento `from` spostato in `to`; indici non validi lasciano la lista invariata. */
export function moveTab<T>(list: T[], from: number, to: number): T[] {
  const out = [...list]
  const ok = (i: number) => Number.isInteger(i) && i >= 0 && i < list.length
  if (!ok(from) || !ok(to) || from === to) return out
  const [item] = out.splice(from, 1)
  out.splice(to, 0, item)
  return out
}
