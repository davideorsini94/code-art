import DOMPurify from 'dompurify'
import { Marked } from 'marked'

/**
 * Unico punto da cui può nascere HTML per `{@html}`: markdown non fidato (risposte, piani) -> HTML sanificato.
 * - l'HTML grezzo nel sorgente non viene interpretato: scartato (i tag nudi come `<String>` restano visibili come testo);
 * - le immagini non vengono caricate (niente richieste di rete dall'interfaccia): resta il testo alternativo,
 *   salvo con un `resolveImage` che restituisca un URL `blob:` (anteprima markdown dei file del progetto);
 * - sopravvivono solo link http: e https: (gli unici che il permesso dell'opener lascia aprire), con rel="noopener noreferrer".
 */
const escapeHtml = (s: string) =>
  s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;').replace(/'/g, '&#39;')

/** Un tag nudo senza attributi (`<String>`, `</b>`) si mostra come testo; qualunque altro HTML grezzo viene scartato. */
const BARE_TAG = /^<\/?[A-Za-z][\w-]*>$/

/** Risolutore di immagini del render in corso (il parse è sincrono: una variabile di modulo basta). */
let resolver: ((href: string) => string | null) | undefined

const md = new Marked({
  gfm: true,
  breaks: false,
  renderer: {
    html: ({ text }) => (BARE_TAG.test(text.trim()) ? escapeHtml(text) : ''),
    image: ({ text, href, title }) => {
      const src = resolver?.(href) ?? null
      if (src === null || !/^blob:/i.test(src)) return escapeHtml(text)
      return `<img src="${escapeHtml(src)}" alt="${escapeHtml(text)}"${title ? ` title="${escapeHtml(title)}"` : ''}>`
    },
    checkbox: ({ checked }) => (checked ? '☑ ' : '☐ '),
  },
})

const purify = DOMPurify(window)
// href: solo http(s); src delle immagini: solo blob: (creato dalla pagina stessa, mai rete)
purify.addHook('uponSanitizeAttribute', (node, data) => {
  if (data.attrName === 'href' && !/^https?:/i.test(data.attrValue)) data.keepAttr = false
  if (data.attrName === 'src' && !(node.tagName === 'IMG' && /^blob:/i.test(data.attrValue))) data.keepAttr = false
})
purify.addHook('afterSanitizeAttributes', (node) => {
  if (node.tagName === 'A') node.setAttribute('rel', 'noopener noreferrer')
})

const CONFIG = {
  ALLOWED_TAGS: [
    'p', 'br', 'hr', 'strong', 'em', 'del', 'code', 'pre', 'blockquote', 'ul', 'ol', 'li', 'a',
    'h1', 'h2', 'h3', 'h4', 'h5', 'h6', 'table', 'thead', 'tbody', 'tr', 'th', 'td', 'span', 'img',
  ],
  ALLOWED_ATTR: ['href', 'rel', 'class', 'align', 'src', 'alt', 'title', 'width', 'height'],
  ALLOWED_URI_REGEXP: /^(?:https?|blob):/i,
  ALLOW_DATA_ATTR: false,
  ALLOW_ARIA_ATTR: false,
}

export type RenderOptions = {
  /** Trasforma l'indirizzo di un'immagine in un URL `blob:`; `null` = scartata (resta il testo alternativo). */
  resolveImage?: (href: string) => string | null
}

export function renderMarkdown(src: string, opts?: RenderOptions): string {
  resolver = opts?.resolveImage
  let raw: string
  try {
    raw = md.parse(src, { async: false }) as string
  } finally {
    resolver = undefined
  }
  return purify.sanitize(raw, CONFIG) as unknown as string
}

