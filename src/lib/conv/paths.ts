/** Tipo di dato di trascinamento con cui l'albero dei file offre il percorso assoluto di una voce. */
export const PATH_DRAG_TYPE = 'application/x-codeart-path'

/** Percorso relativo alla cartella del progetto che lo contiene (altrimenti quello assoluto). */
export function relativeToFolders(path: string, folders: string[]): string {
  for (const f of folders) {
    const base = f.replace(/\/+$/, '')
    if (path.startsWith(base + '/')) return path.slice(base.length + 1)
  }
  return path
}

/** Testo `@percorso` da inserire; i percorsi con spazi vanno tra virgolette come nei prompt di Claude Code. */
export function mentionFor(path: string, folders: string[]): string {
  const rel = relativeToFolders(path, folders)
  return /\s/.test(rel) ? `@"${rel}"` : `@${rel}`
}

/** Estensione accettata dal motore per un tipo MIME immagine, o null. */
export function imageExt(mime: string): string | null {
  return ({ 'image/png': 'png', 'image/jpeg': 'jpg', 'image/gif': 'gif', 'image/webp': 'webp' } as Record<string, string>)[mime] ?? null
}

export const MAX_IMAGES = 10
export const MAX_IMAGE_BYTES = 20 * 1024 * 1024
export const MAX_TOTAL_BYTES = 30 * 1024 * 1024
