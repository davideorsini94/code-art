// Scelta del visualizzatore per tipo di file. Solo il nome decide (mai lo sniffing del contenuto),
// e il nome non finisce mai in HTML: qui si ricavano solo etichette fisse da tabelle fisse.
export type Viewer = 'markdown' | 'image' | 'pdf' | 'media' | 'text' | 'binary'
export type MarkdownMode = 'preview' | 'edit' | 'split'

const MIME = new Map<string, string>([
  ['png', 'image/png'], ['jpg', 'image/jpeg'], ['jpeg', 'image/jpeg'], ['gif', 'image/gif'], ['webp', 'image/webp'],
  ['bmp', 'image/bmp'], ['ico', 'image/x-icon'], ['avif', 'image/avif'], ['svg', 'image/svg+xml'],
  ['pdf', 'application/pdf'],
  ['mp3', 'audio/mpeg'], ['wav', 'audio/wav'], ['m4a', 'audio/mp4'], ['ogg', 'audio/ogg'],
  ['mp4', 'video/mp4'], ['mov', 'video/quicktime'], ['webm', 'video/webm'],
])
const IMAGES = new Set(['png', 'jpg', 'jpeg', 'gif', 'webp', 'bmp', 'ico', 'avif', 'svg'])
const MEDIA = new Set(['mp3', 'wav', 'm4a', 'ogg', 'mp4', 'mov', 'webm'])
const MARKDOWN = new Set(['md', 'markdown', 'mdx'])
// Estensioni che sono binarie di sicuro: non si prova nemmeno a leggerle come testo.
const BINARY = new Set([
  'zip', 'gz', 'tgz', 'tar', 'bz2', 'xz', '7z', 'rar', 'dmg', 'iso', 'pkg', 'exe', 'dll', 'so', 'dylib', 'o', 'a', 'class',
  'jar', 'wasm', 'bin', 'ttf', 'otf', 'woff', 'woff2', 'sqlite', 'sqlite3', 'db', 'psd', 'doc', 'docx', 'xls', 'xlsx',
  'ppt', 'pptx', 'flac', 'avi', 'mkv', 'heic', 'tiff', 'tif',
])

/** Estensione minuscola dopo l'ultimo punto ('' se non c'è o se il nome è solo un file nascosto come `.md`). */
function ext(name: string): string {
  const i = name.lastIndexOf('.')
  return i <= 0 ? '' : name.slice(i + 1).toLowerCase()
}

export function viewerFor(name: string): Viewer {
  const e = ext(name)
  if (MARKDOWN.has(e)) return 'markdown'
  if (IMAGES.has(e)) return 'image'
  if (e === 'pdf') return 'pdf'
  if (MEDIA.has(e)) return 'media'
  if (BINARY.has(e)) return 'binary'
  return 'text'
}

/** Tipo MIME da tabella fissa per estensione; `null` se non è un tipo di anteprima. */
export function mimeFor(name: string): string | null {
  return MIME.get(ext(name)) ?? null
}

export function isVideo(name: string): boolean {
  return mimeFor(name)?.startsWith('video/') ?? false
}

/** Il tipo apre una scheda con byte grezzi (non testo in Monaco). */
export const isBinaryViewer = (v: Viewer) => v === 'image' || v === 'pdf' || v === 'media' || v === 'binary'

const SCHEME = /^[a-z][a-z0-9+.-]*:/i

/**
 * Risolve l'indirizzo di un'immagine di un file markdown in un percorso assoluto, solo se è relativo e resta
 * dentro una delle cartelle del progetto. Rifiuta schemi (http:, data:, file:…), `//host`, percorsi assoluti,
 * `..` che esce dal progetto e tipi che non sono immagini. Il confinamento definitivo lo applica comunque il core.
 */
export function resolveMarkdownImage(href: string, mdPath: string, roots: string[]): string | null {
  const bad = (x: string) => !x || SCHEME.test(x) || x.startsWith('/') || x.startsWith('\\') || x.includes('\\') || x.includes('\0')
  let h = href.trim()
  if (bad(h)) return null
  h = h.split('#')[0].split('?')[0]
  try {
    h = decodeURIComponent(h)
  } catch {
    return null
  }
  if (bad(h)) return null // di nuovo dopo la decodifica: %00, %2f%2fhost, %5c, %3a
  const dir = mdPath.split('/').slice(0, -1)
  const out: string[] = []
  for (const seg of [...dir, ...h.split('/')]) {
    if (seg === '' || seg === '.') continue
    if (seg === '..') {
      if (out.length === 0) return null
      out.pop()
    } else out.push(seg)
  }
  const abs = '/' + out.join('/')
  const inside = roots.some((r) => {
    const root = r.replace(/\/+$/, '')
    return abs.startsWith(root + '/')
  })
  if (!inside || !IMAGES.has(ext(abs))) return null
  return abs
}
