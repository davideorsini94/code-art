import DOMPurify from 'dompurify'
import { Marked } from 'marked'

/**
 * Unico punto da cui può nascere HTML per `{@html}`: markdown non fidato (risposte, piani) -> HTML sanificato.
 * - l'HTML grezzo nel sorgente non viene interpretato: scartato (i tag nudi come `<String>` restano visibili come testo);
 * - le immagini non vengono caricate (niente richieste di rete dall'interfaccia): resta il testo alternativo;
 * - sopravvivono solo link http:, https: e mailto:, con rel="noopener noreferrer".
 */
const escapeHtml = (s: string) =>
  s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;').replace(/'/g, '&#39;')

/** Un tag nudo senza attributi (`<String>`, `</b>`) si mostra come testo; qualunque altro HTML grezzo viene scartato. */
const BARE_TAG = /^<\/?[A-Za-z][\w-]*>$/

const md = new Marked({
  gfm: true,
  breaks: false,
  renderer: {
    html: ({ text }) => (BARE_TAG.test(text.trim()) ? escapeHtml(text) : ''),
    image: ({ text }) => escapeHtml(text),
  },
})

const purify = DOMPurify(window)
purify.addHook('afterSanitizeAttributes', (node) => {
  if (node.tagName === 'A') node.setAttribute('rel', 'noopener noreferrer')
})

const CONFIG = {
  ALLOWED_TAGS: [
    'p', 'br', 'hr', 'strong', 'em', 'del', 'code', 'pre', 'blockquote', 'ul', 'ol', 'li', 'a',
    'h1', 'h2', 'h3', 'h4', 'h5', 'h6', 'table', 'thead', 'tbody', 'tr', 'th', 'td', 'span',
  ],
  ALLOWED_ATTR: ['href', 'rel', 'class', 'align'],
  ALLOWED_URI_REGEXP: /^(?:https?:|mailto:)/i,
  ALLOW_DATA_ATTR: false,
  ALLOW_ARIA_ATTR: false,
}

export function renderMarkdown(src: string): string {
  const raw = md.parse(src, { async: false }) as string
  return purify.sanitize(raw, CONFIG) as unknown as string
}

/** Vero se l'indirizzo si può aprire nel browser di sistema. */
export function isSafeLink(href: string): boolean {
  return /^(?:https?:|mailto:)/i.test(href.trim())
}
