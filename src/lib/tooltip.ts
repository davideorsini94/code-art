/**
 * Tooltip dell'interfaccia: un'unica azione Svelte `use:tooltip={'Testo'}` o `use:tooltip={{ text, shortcut?, placement? }}`.
 *
 * - Un solo elemento condiviso in `document.body`; nessun nodo e nessun listener per ogni bersaglio: i bersagli
 *   si registrano in una WeakMap (e un attributo `data-tt`), gli eventi sono delegati al documento.
 * - Il testo entra sempre con `textContent` (può contenere nomi di file o titoli non fidati): mai HTML.
 * - Non ruba mai il focus né gli eventi del puntatore (`pointer-events: none`) e non ferma la propagazione.
 */

export type Placement = 'top' | 'bottom' | 'left' | 'right'
export type TooltipOptions = { text: string; shortcut?: string; placement?: Placement }
export type TooltipParam = string | TooltipOptions | null | undefined | false

export const SHOW_DELAY = 500
export const GRACE = 300
const GAP = 8
const MARGIN = 6
const CONTAINER_ID = 'codeart-tooltip'

type Box = { left: number; top: number; width: number; height: number }

/**
 * Posizione (coordinate fisse) del tooltip rispetto all'elemento. Parte dal lato preferito; se non ci sta e
 * l'opposto sì (o ha più spazio) si ribalta; poi si tiene dentro la finestra con un margine.
 */
export function placeTooltip(
  anchor: Box,
  size: { width: number; height: number },
  viewport: { width: number; height: number },
  placement: Placement = 'bottom',
  gap = GAP,
  margin = MARGIN,
): { x: number; y: number; placement: Placement } {
  const space = {
    top: anchor.top - margin,
    bottom: viewport.height - (anchor.top + anchor.height) - margin,
    left: anchor.left - margin,
    right: viewport.width - (anchor.left + anchor.width) - margin,
  }
  const need = { top: size.height + gap, bottom: size.height + gap, left: size.width + gap, right: size.width + gap }
  const opposite: Record<Placement, Placement> = { top: 'bottom', bottom: 'top', left: 'right', right: 'left' }
  let side = placement
  if (space[side] < need[side] && space[opposite[side]] > space[side]) side = opposite[side]

  const clamp = (v: number, lo: number, hi: number) => Math.max(lo, Math.min(v, Math.max(lo, hi)))
  let x: number
  let y: number
  if (side === 'top' || side === 'bottom') {
    x = clamp(anchor.left + anchor.width / 2 - size.width / 2, margin, viewport.width - size.width - margin)
    y = side === 'bottom' ? anchor.top + anchor.height + gap : anchor.top - size.height - gap
    y = clamp(y, margin, viewport.height - size.height - margin)
  } else {
    y = clamp(anchor.top + anchor.height / 2 - size.height / 2, margin, viewport.height - size.height - margin)
    x = side === 'right' ? anchor.left + anchor.width + gap : anchor.left - size.width - gap
    x = clamp(x, margin, viewport.width - size.width - margin)
  }
  return { x, y, placement: side }
}

type Entry = { text: string; shortcut?: string; placement: Placement; labelled: boolean }

const entries = new WeakMap<Element, Entry>()
let box: HTMLDivElement | null = null
let textEl: HTMLSpanElement | null = null
let keyEl: HTMLElement | null = null
let installed = false
let current: Element | null = null // bersaglio mostrato
let pending: Element | null = null // bersaglio in attesa del ritardo
let timer: ReturnType<typeof setTimeout> | undefined
let lastHiddenAt = -Infinity
let observer: MutationObserver | undefined

const normalize = (p: TooltipParam): { text: string; shortcut?: string; placement?: Placement } | null => {
  if (!p) return null
  const o = typeof p === 'string' ? { text: p } : p
  return o.text ? o : null
}

/** Il bersaglio ha già un nome accessibile (testo con lettere o cifre, aria-label, alt, etichetta)? */
function hasOwnName(node: Element): boolean {
  if (node.hasAttribute('aria-labelledby')) return true
  if (node.getAttribute('alt')) return true
  const labels = (node as HTMLInputElement).labels
  if (labels && labels.length > 0) return true
  return /[\p{L}\p{N}]/u.test(node.textContent ?? '')
}

function container(): HTMLDivElement {
  if (box?.isConnected) return box
  box = document.createElement('div')
  box.id = CONTAINER_ID
  box.className = 'codeart-tooltip'
  box.setAttribute('role', 'tooltip')
  box.setAttribute('aria-hidden', 'true')
  box.style.cssText = 'position:fixed;left:0;top:0;display:none;pointer-events:none;z-index:2147483000'
  textEl = document.createElement('span')
  textEl.className = 'tt-text'
  keyEl = document.createElement('kbd')
  keyEl.className = 'tt-key'
  box.append(textEl, keyEl)
  document.body.appendChild(box)
  return box
}

function hostFor(target: EventTarget | null): Element | null {
  if (!(target instanceof Element)) return null
  const host = target.closest('[data-tt]')
  return host && entries.has(host) ? host : null
}

function render(node: Element) {
  const e = entries.get(node)
  if (!e) return hide()
  const el = container()
  textEl!.textContent = e.text
  keyEl!.textContent = e.shortcut ?? ''
  keyEl!.style.display = e.shortcut ? '' : 'none'
  el.style.visibility = 'hidden'
  el.style.display = 'block'
  const r = node.getBoundingClientRect()
  const s = el.getBoundingClientRect()
  const pos = placeTooltip(r, { width: s.width, height: s.height }, { width: window.innerWidth, height: window.innerHeight }, e.placement)
  el.style.left = `${Math.round(pos.x)}px`
  el.style.top = `${Math.round(pos.y)}px`
  el.dataset.placement = pos.placement
  el.style.visibility = ''
}

function show(node: Element) {
  clearTimeout(timer)
  timer = undefined
  pending = null
  if (!node.isConnected || !entries.has(node)) return
  current = node
  render(node)
  observer ??= new MutationObserver(() => {
    if (current && !current.isConnected) hide()
  })
  observer.observe(document.body, { childList: true, subtree: true })
}

function hide() {
  clearTimeout(timer)
  timer = undefined
  pending = null
  if (!current) return
  current = null
  observer?.disconnect()
  if (box) box.style.display = 'none'
  lastHiddenAt = Date.now()
}

function schedule(node: Element) {
  if (node === current || node === pending) return
  const wasShown = current !== null
  hide()
  pending = node
  const quick = wasShown || Date.now() - lastHiddenAt < GRACE
  if (quick) return show(node)
  timer = setTimeout(() => show(node), SHOW_DELAY)
}

function focusVisible(el: Element): boolean {
  try {
    return el.matches(':focus-visible')
  } catch {
    return true
  }
}

const onOver = (e: Event) => {
  const pe = e as PointerEvent
  if (pe.pointerType === 'touch' || pe.buttons) return
  const host = hostFor(e.target)
  if (host) schedule(host)
  else if (current || pending) hide()
}

const onOut = (e: Event) => {
  const host = hostFor(e.target)
  if (!host || (host !== current && host !== pending)) return
  const to = (e as PointerEvent).relatedTarget
  if (to instanceof Node && host.contains(to)) return
  hide()
}

const onFocusIn = (e: Event) => {
  const host = hostFor(e.target)
  if (host && focusVisible(host)) {
    hide()
    show(host)
  }
}

const onFocusOut = (e: Event) => {
  const host = hostFor(e.target)
  if (host && (host === current || host === pending)) hide()
}

const onKey = (e: Event) => {
  if ((e as KeyboardEvent).key === 'Escape') hide()
}

const onAny = () => {
  if (current || pending) hide()
}

const DOC_EVENTS: [string, EventListener, boolean][] = [
  ['pointerover', onOver, true],
  ['pointerout', onOut, true],
  ['focusin', onFocusIn, true],
  ['focusout', onFocusOut, true],
  ['pointerdown', onAny, true],
  ['keydown', onKey, true],
  ['scroll', onAny, true],
  ['wheel', onAny, true],
  ['contextmenu', onAny, true],
  ['dragstart', onAny, true],
]

function install() {
  if (installed || typeof document === 'undefined') return
  installed = true
  for (const [t, fn, cap] of DOC_EVENTS) document.addEventListener(t, fn, { capture: cap, passive: true })
  window.addEventListener('blur', onAny)
}

/** Solo per i test: toglie listener, elemento condiviso e stato. */
export function resetTooltipForTests() {
  hide()
  if (installed) {
    for (const [t, fn, cap] of DOC_EVENTS) document.removeEventListener(t, fn, { capture: cap })
    window.removeEventListener('blur', onAny)
  }
  installed = false
  box?.remove()
  box = textEl = keyEl = null
  lastHiddenAt = -Infinity
}

function apply(node: HTMLElement, param: TooltipParam) {
  const prev = entries.get(node)
  const o = normalize(param)
  if (!o) {
    if (prev?.labelled) node.removeAttribute('aria-label')
    entries.delete(node)
    node.removeAttribute('data-tt')
    if (current === node || pending === node) hide()
    return
  }
  node.removeAttribute('title') // niente doppio tooltip nativo
  // nome accessibile: solo per i controlli senza un nome proprio (icone); lo si aggiorna se lo avevamo messo noi
  let labelled = prev?.labelled ?? false
  if (labelled) node.setAttribute('aria-label', o.text)
  else if (!node.hasAttribute('aria-label') && !hasOwnName(node)) {
    node.setAttribute('aria-label', o.text)
    labelled = true
  }
  entries.set(node, { text: o.text, shortcut: o.shortcut, placement: o.placement ?? 'bottom', labelled })
  node.setAttribute('data-tt', '')
  if (current === node) render(node)
}

export function tooltip(node: HTMLElement, param: TooltipParam) {
  install()
  apply(node, param)
  return {
    update(next: TooltipParam) {
      apply(node, next)
    },
    destroy() {
      if (current === node || pending === node) hide()
      if (entries.get(node)?.labelled) node.removeAttribute('aria-label')
      entries.delete(node)
      node.removeAttribute('data-tt')
    },
  }
}
