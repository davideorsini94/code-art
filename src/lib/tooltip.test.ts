// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { placeTooltip, resetTooltipForTests, tooltip, type TooltipParam } from './tooltip'

const tip = () => document.getElementById('codeart-tooltip')
const shown = () => tip()?.style.display === 'block'

function mount(html: string, param: TooltipParam = 'Testo') {
  document.body.insertAdjacentHTML('beforeend', html)
  const el = document.body.lastElementChild as HTMLElement
  const act = tooltip(el, param)
  return { el, act }
}

const hover = (el: Element, init: PointerEventInit = {}) => el.dispatchEvent(new PointerEvent('pointerover', { bubbles: true, ...init }))
const leave = (el: Element, init: PointerEventInit = {}) => el.dispatchEvent(new PointerEvent('pointerout', { bubbles: true, ...init }))

beforeEach(() => {
  vi.useFakeTimers()
})
afterEach(() => {
  resetTooltipForTests()
  document.body.innerHTML = ''
  vi.useRealTimers()
})

describe('tooltip action', () => {
  it('appears after the hover delay, not before', () => {
    const { el } = mount('<button>Salva</button>')
    hover(el)
    vi.advanceTimersByTime(499)
    expect(shown()).toBe(false)
    vi.advanceTimersByTime(1)
    expect(shown()).toBe(true)
    expect(tip()!.textContent).toContain('Testo')
  })

  it('is immediate when another tooltip was visible a moment ago', () => {
    const a = mount('<button>A</button>', 'Uno')
    const b = mount('<button>B</button>', 'Due')
    hover(a.el)
    vi.advanceTimersByTime(500)
    leave(a.el)
    expect(shown()).toBe(false)
    vi.advanceTimersByTime(100)
    hover(b.el)
    expect(shown()).toBe(true)
    expect(tip()!.textContent).toContain('Due')
    // dopo la pausa torna il ritardo
    leave(b.el)
    vi.advanceTimersByTime(400)
    hover(a.el)
    expect(shown()).toBe(false)
  })

  it('shows on keyboard focus (focus-visible) and hides on blur', () => {
    const { el } = mount('<button>Salva</button>')
    const spy = vi.spyOn(el, 'matches').mockImplementation((s: string) => s === ':focus-visible')
    el.dispatchEvent(new FocusEvent('focusin', { bubbles: true }))
    expect(shown()).toBe(true)
    el.dispatchEvent(new FocusEvent('focusout', { bubbles: true }))
    expect(shown()).toBe(false)
    spy.mockImplementation(() => false) // focus da mouse: niente tooltip
    el.dispatchEvent(new FocusEvent('focusin', { bubbles: true }))
    expect(shown()).toBe(false)
  })

  it('hides on pointer leave, Escape, pointerdown and scroll', () => {
    const { el } = mount('<button>Salva</button>')
    const open = () => {
      hover(el)
      vi.advanceTimersByTime(500)
      expect(shown()).toBe(true)
    }
    open()
    leave(el)
    expect(shown()).toBe(false)
    open()
    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))
    expect(shown()).toBe(false)
    open()
    el.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true }))
    expect(shown()).toBe(false)
    open()
    document.body.dispatchEvent(new Event('scroll'))
    expect(shown()).toBe(false)
  })

  it('does not hide when the pointer moves between children of the target', () => {
    const { el } = mount('<button><span>x</span>Salva</button>')
    hover(el)
    vi.advanceTimersByTime(500)
    leave(el.firstElementChild!, { relatedTarget: el })
    expect(shown()).toBe(true)
  })

  it('never shows for touch input or while a button is pressed (drag)', () => {
    const { el } = mount('<button>Salva</button>')
    hover(el, { pointerType: 'touch' })
    vi.advanceTimersByTime(1000)
    expect(tip()).toBeNull()
    hover(el, { buttons: 1 })
    vi.advanceTimersByTime(1000)
    expect(tip()).toBeNull()
  })

  it('does not stop propagation of Escape', () => {
    const { el } = mount('<button>Salva</button>')
    const seen = vi.fn()
    window.addEventListener('keydown', seen)
    el.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))
    expect(seen).toHaveBeenCalled()
    window.removeEventListener('keydown', seen)
  })

  it('renders text as text, never as HTML', () => {
    const evil = '<img src=x onerror="window.__pwned=1">'
    const { el } = mount('<button>Salva</button>', { text: evil, shortcut: '<b>K</b>' })
    hover(el)
    vi.advanceTimersByTime(500)
    expect(tip()!.querySelector('img')).toBeNull()
    expect(tip()!.querySelector('b')).toBeNull()
    expect(tip()!.textContent).toContain(evil)
    expect(tip()!.textContent).toContain('<b>K</b>')
  })

  it('renders the shortcut as a keycap after the text', () => {
    const { el } = mount('<button>Salva</button>', { text: 'Salva il file', shortcut: '⌘S' })
    hover(el)
    vi.advanceTimersByTime(500)
    expect(tip()!.querySelector('.tt-text')!.textContent).toBe('Salva il file')
    const key = tip()!.querySelector('kbd') as HTMLElement
    expect(key.textContent).toBe('⌘S')
    expect(key.style.display).not.toBe('none')
  })

  it('sets aria-label only when the element has no accessible name of its own', () => {
    const icon = mount('<button>×</button>', 'Chiudi scheda')
    expect(icon.el.getAttribute('aria-label')).toBe('Chiudi scheda')
    const labelled = mount('<button>Salva</button>', 'Salva il file')
    expect(labelled.el.hasAttribute('aria-label')).toBe(false)
    const own = mount('<button aria-label="Chiudi a.ts">×</button>', 'Chiudi scheda')
    expect(own.el.getAttribute('aria-label')).toBe('Chiudi a.ts')
    const empty = mount('<div role="separator"></div>', 'Trascina')
    expect(empty.el.getAttribute('aria-label')).toBe('Trascina')
    // aggiornamento del testo: l'etichetta messa da noi lo segue
    icon.act.update('Chiudi')
    expect(icon.el.getAttribute('aria-label')).toBe('Chiudi')
    // e se ne va con l'azione
    icon.act.destroy()
    expect(icon.el.hasAttribute('aria-label')).toBe(false)
  })

  it('removes the native title so two tooltips never appear together', () => {
    const { el } = mount('<button title="vecchio">Salva</button>')
    expect(el.hasAttribute('title')).toBe(false)
  })

  it('removes itself when the target is destroyed or leaves the DOM', async () => {
    const a = mount('<button>A</button>')
    hover(a.el)
    vi.advanceTimersByTime(500)
    expect(shown()).toBe(true)
    a.act.destroy()
    expect(shown()).toBe(false)

    const b = mount('<button>B</button>')
    hover(b.el)
    vi.advanceTimersByTime(500)
    expect(shown()).toBe(true)
    b.el.remove()
    await vi.advanceTimersByTimeAsync(0) // MutationObserver
    expect(shown()).toBe(false)

    vi.advanceTimersByTime(400) // fuori dalla tolleranza dei 300 ms
    const c = mount('<button>C</button>')
    hover(c.el)
    c.el.remove() // rimosso durante l'attesa
    vi.advanceTimersByTime(500)
    expect(shown()).toBe(false)
  })

  it('uses one shared container for every target', () => {
    const a = mount('<button>A</button>', 'Uno')
    const b = mount('<button>B</button>', 'Due')
    hover(a.el)
    vi.advanceTimersByTime(500)
    hover(b.el)
    vi.advanceTimersByTime(500)
    expect(document.querySelectorAll('#codeart-tooltip')).toHaveLength(1)
    expect(document.querySelectorAll('[role="tooltip"]')).toHaveLength(1)
    expect(tip()!.parentElement).toBe(document.body)
    expect(tip()!.style.pointerEvents).toBe('none')
  })

  it('an empty text disables the tooltip', () => {
    const { el, act } = mount('<button>A</button>', 'Uno')
    act.update('')
    hover(el)
    vi.advanceTimersByTime(1000)
    expect(shown()).toBe(false)
  })
})

describe('tooltip action: more behaviour', () => {
  const open = (el: Element) => {
    hover(el)
    vi.advanceTimersByTime(500)
    expect(shown()).toBe(true)
  }

  it.each([
    ['contextmenu', (el: Element) => el.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true }))],
    ['dragstart', (el: Element) => el.dispatchEvent(new Event('dragstart', { bubbles: true }))],
    ['wheel', (el: Element) => el.dispatchEvent(new WheelEvent('wheel', { bubbles: true }))],
    ['window blur', () => window.dispatchEvent(new Event('blur'))],
  ])('hides on %s', (_n, fire) => {
    const { el } = mount('<button>Salva</button>')
    open(el)
    fire(el)
    expect(shown()).toBe(false)
  })

  it('a scroll elsewhere does not cancel a pending or visible tooltip', () => {
    const { el } = mount('<button>Salva</button>')
    const other = document.createElement('div')
    document.body.appendChild(other)
    hover(el)
    other.dispatchEvent(new Event('scroll')) // streaming altrove
    vi.advanceTimersByTime(500)
    expect(shown()).toBe(true)
    other.dispatchEvent(new Event('scroll'))
    expect(shown()).toBe(true)
  })

  it('a scroll of a container of the target (or of the document) hides it', () => {
    document.body.insertAdjacentHTML('beforeend', '<div id="sc"><button>Salva</button></div>')
    const sc = document.getElementById('sc')!
    const el = sc.firstElementChild as HTMLElement
    tooltip(el, 'Testo')
    open(el)
    sc.dispatchEvent(new Event('scroll'))
    expect(shown()).toBe(false)
    vi.advanceTimersByTime(400)
    open(el)
    document.dispatchEvent(new Event('scroll'))
    expect(shown()).toBe(false)
  })

  it('a keyboard-focus tooltip survives pointer moves over unrelated elements', () => {
    const { el } = mount('<button>Salva</button>')
    const other = document.createElement('div')
    document.body.appendChild(other)
    vi.spyOn(el, 'matches').mockReturnValue(true)
    el.dispatchEvent(new FocusEvent('focusin', { bubbles: true }))
    expect(shown()).toBe(true)
    hover(other)
    leave(other)
    leave(el)
    expect(shown()).toBe(true)
    el.dispatchEvent(new FocusEvent('focusout', { bubbles: true }))
    expect(shown()).toBe(false)
  })

  it('aria-label goes only on controls or elements with a nameable role, never on plain spans/divs', () => {
    const dot = mount('<span class="dot"></span>', 'Claude sta lavorando')
    expect(dot.el.hasAttribute('aria-label')).toBe(false)
    const handle = mount('<div class="handle"></div>', 'Trascina')
    expect(handle.el.hasAttribute('aria-label')).toBe(false)
    const img = mount('<span role="img"></span>', 'Modifiche non salvate')
    expect(img.el.getAttribute('aria-label')).toBe('Modifiche non salvate')
    const btn = mount('<button></button>', 'Chiudi')
    expect(btn.el.getAttribute('aria-label')).toBe('Chiudi')
    const status = mount('<span role="status"></span>', 'Stato')
    expect(status.el.getAttribute('aria-label')).toBe('Stato')
  })

  it('a dot inside a labelled control does not change the control accessible name', () => {
    document.body.insertAdjacentHTML('beforeend', '<div role="button" id="card"><span class="d"></span><span>Titolo</span></div>')
    const card = document.getElementById('card')!
    tooltip(card.querySelector('.d') as HTMLElement, 'Claude sta lavorando')
    expect(card.querySelector('[aria-label]')).toBeNull()
  })
})

describe('placeTooltip', () => {
  const vp = { width: 800, height: 600 }
  const size = { width: 120, height: 28 }

  it('puts the tooltip below and centred by default', () => {
    const p = placeTooltip({ left: 100, top: 100, width: 40, height: 20 }, size, vp)
    expect(p).toEqual({ x: 60, y: 128, placement: 'bottom' })
  })

  it('flips above when there is no room below', () => {
    const p = placeTooltip({ left: 100, top: 570, width: 40, height: 20 }, size, vp)
    expect(p.placement).toBe('top')
    expect(p.y).toBe(570 - 28 - 8)
  })

  it('flips below when a top placement does not fit', () => {
    const p = placeTooltip({ left: 100, top: 5, width: 40, height: 20 }, size, vp, 'top')
    expect(p.placement).toBe('bottom')
  })

  it('clamps inside the window horizontally', () => {
    expect(placeTooltip({ left: 0, top: 100, width: 10, height: 10 }, size, vp).x).toBe(6)
    expect(placeTooltip({ left: 795, top: 100, width: 10, height: 10 }, size, vp).x).toBe(800 - 120 - 6)
  })

  it('supports side placements and flips them', () => {
    expect(placeTooltip({ left: 300, top: 100, width: 40, height: 20 }, size, vp, 'right')).toMatchObject({ placement: 'right', x: 348 })
    expect(placeTooltip({ left: 780, top: 100, width: 20, height: 20 }, size, vp, 'right').placement).toBe('left')
  })

  it('never goes outside even when larger than the viewport', () => {
    const p = placeTooltip({ left: 10, top: 10, width: 10, height: 10 }, { width: 900, height: 700 }, vp)
    expect(p.x).toBeGreaterThanOrEqual(6)
    expect(p.y).toBeGreaterThanOrEqual(6)
  })
})
