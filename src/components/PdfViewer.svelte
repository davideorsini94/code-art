<script lang="ts">
  // PDF con pdf.js (caricato al primo uso, worker locale). Le pagine si disegnano su <canvas> solo quando
  // entrano nella vista, una alla volta, con un tetto ai pixel del canvas. Niente testo selezionabile,
  // niente annotazioni né link: il contenuto del PDF non può far partire nulla.
  import { onDestroy } from 'svelte'
  import { fsReadBytes, asFsError } from '../lib/ipc'
  import { tooltip } from '../lib/tooltip'

  let { path, rev }: { path: string; rev: number } = $props()

  type Doc = import('pdfjs-dist').PDFDocumentProxy
  const MAX_CANVAS_PIXELS = 16_000_000
  const GAP = 12

  let doc: Doc | null = null
  let status = $state<'loading' | 'ready' | 'error'>('loading')
  let errMsg = $state('')
  let count = $state(0)
  let base = $state({ w: 612, h: 792 }) // dimensioni a scala 1 della prima pagina (segnaposto per le altre)
  const sizes = new Map<number, { w: number; h: number }>() // dimensioni reali, note dopo il disegno
  let sizesTick = $state(0)
  let zoom = $state(1)
  let fitMode = $state(true)
  let current = $state(1)
  let scroller = $state<HTMLDivElement>()
  const els = new Map<number, HTMLDivElement>()
  let observer: IntersectionObserver | undefined
  let gen = 0
  let queue: number[] = []
  let rendering = false
  let alive = true

  const pageSize = (n: number) => {
    void sizesTick
    const s = sizes.get(n) ?? base
    return { w: Math.round(s.w * zoom), h: Math.round(s.h * zoom) }
  }

  async function load(p: string) {
    const myGen = ++gen
    status = 'loading'
    queue = []
    sizes.clear()
    els.forEach((el) => el.querySelector('canvas')?.remove())
    const old = doc
    doc = null
    void old?.loadingTask.destroy()
    try {
      const bytes = await fsReadBytes(p)
      const { openPdf } = await import('../lib/pdf')
      const d = await openPdf(bytes).promise
      if (!alive || myGen !== gen) return void d.loadingTask.destroy()
      const first = await d.getPage(1)
      const vp = first.getViewport({ scale: 1 })
      base = { w: vp.width, h: vp.height }
      first.cleanup()
      doc = d
      count = d.numPages
      if (fitMode) zoom = fitZoom()
      current = 1
      status = 'ready'
    } catch (e) {
      if (myGen !== gen) return
      const code = typeof e === 'object' && e && 'code' in e ? asFsError(e).code : ''
      errMsg = code === 'too_large' ? 'PDF troppo grande per l\'anteprima (oltre 50 MB)' : 'Impossibile aprire il PDF'
      status = 'error'
    }
  }

  function fitZoom() {
    const w = scroller?.clientWidth ?? 800
    return Math.max(0.25, Math.min(2, (w - 48) / base.w))
  }

  $effect(() => {
    void rev
    void load(path)
  })

  // IntersectionObserver: segnala i segnaposto vicini alla vista
  $effect(() => {
    if (!scroller) return
    observer = new IntersectionObserver(
      (entries) => {
        for (const e of entries) {
          if (!e.isIntersecting) continue
          const n = Number((e.target as HTMLElement).dataset.page)
          if (!queue.includes(n) && !(e.target as HTMLElement).querySelector('canvas')) queue.push(n)
        }
        void pump()
      },
      { root: scroller, rootMargin: '800px 0px' },
    )
    els.forEach((el) => observer?.observe(el))
    return () => observer?.disconnect()
  })

  function lazy(node: HTMLDivElement, n: number) {
    node.dataset.page = String(n)
    els.set(n, node)
    observer?.observe(node)
    return {
      destroy() {
        observer?.unobserve(node)
        els.delete(n)
      },
    }
  }

  async function pump() {
    if (rendering) return
    rendering = true
    try {
      while (queue.length && doc && alive) {
        const myGen = gen
        // il più vicino alla pagina corrente per primo
        queue.sort((a, b) => Math.abs(a - current) - Math.abs(b - current))
        const n = queue.shift()!
        const el = els.get(n)
        if (!el || el.querySelector('canvas')) continue
        const d = doc
        try {
          const page = await d.getPage(n)
          const vp = page.getViewport({ scale: zoom })
          const dpr = Math.min(window.devicePixelRatio || 1, 2)
          const out = Math.min(dpr, Math.sqrt(MAX_CANVAS_PIXELS / (vp.width * vp.height)))
          const canvas = document.createElement('canvas')
          canvas.width = Math.max(1, Math.floor(vp.width * out))
          canvas.height = Math.max(1, Math.floor(vp.height * out))
          canvas.style.width = '100%'
          canvas.style.height = '100%'
          const ctx = canvas.getContext('2d')
          if (ctx) {
            await page.render({ canvas, canvasContext: ctx, viewport: page.getViewport({ scale: zoom * out }) }).promise
            if (myGen === gen && els.get(n) === el && !el.querySelector('canvas')) el.appendChild(canvas)
          }
          const s1 = page.getViewport({ scale: 1 })
          sizes.set(n, { w: s1.width, h: s1.height })
          sizesTick++
          page.cleanup()
        } catch {
          // una pagina rovinata non ferma le altre
        }
        await new Promise((r) => requestAnimationFrame(() => r(null))) // lascia respirare l'interfaccia
      }
    } finally {
      rendering = false
    }
  }

  function setZoom(z: number, fit = false) {
    fitMode = fit
    zoom = Math.max(0.25, Math.min(4, z))
    queue = []
    gen++
    els.forEach((el) => el.querySelector('canvas')?.remove())
    // dopo il layout, ri-osserva: la callback iniziale rimette in coda le pagine visibili
    requestAnimationFrame(() => {
      observer?.disconnect()
      els.forEach((el) => observer?.observe(el))
    })
  }

  let raf = 0
  function onScroll() {
    if (raf) return
    raf = requestAnimationFrame(() => {
      raf = 0
      if (!scroller) return
      const mid = scroller.scrollTop + scroller.clientHeight / 3
      let best = 1
      for (const [n, el] of els) if (el.offsetTop <= mid && n >= best) best = n
      current = best
    })
  }

  function key(e: KeyboardEvent) {
    if (!e.metaKey) return
    if (e.key === '+' || e.key === '=') setZoom(zoom * 1.2)
    else if (e.key === '-') setZoom(zoom / 1.2)
    else if (e.key === '0') setZoom(fitZoom(), true)
    else return
    e.preventDefault()
  }

  onDestroy(() => {
    alive = false
    gen++
    cancelAnimationFrame(raf)
    void doc?.loadingTask.destroy()
    doc = null
  })
</script>

<!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
<div class="viewer" tabindex="0" role="application" aria-label="Anteprima PDF" data-no-tooltip="contenitore di scelte da tastiera: i controlli al suo interno hanno il proprio suggerimento" onkeydown={key}>
  <div class="pages" bind:this={scroller} onscroll={onScroll}>
    {#if status === 'loading'}
      <p class="msg">Caricamento…</p>
    {:else if status === 'error'}
      <p class="msg">{errMsg}</p>
    {:else}
      {#each Array.from({ length: count }, (_, i) => i + 1) as n (n)}
        {@const sz = pageSize(n)}
        <div class="page" use:lazy={n} style:width="{sz.w}px" style:height="{sz.h}px" style:margin-bottom="{GAP}px"></div>
      {/each}
    {/if}
  </div>
  <div class="foot">
    <span>{status === 'ready' ? `pagina ${current} di ${count}` : ''}</span>
    <span class="grow"></span>
    <button use:tooltip={{ text: 'Riduci', shortcut: '⌘−' }} onclick={() => setZoom(zoom / 1.2)} aria-label="Riduci">−</button>
    <span class="z">{Math.round(zoom * 100)}%</span>
    <button use:tooltip={{ text: 'Ingrandisci', shortcut: '⌘+' }} onclick={() => setZoom(zoom * 1.2)} aria-label="Ingrandisci">+</button>
    <button class:on={fitMode} use:tooltip={{ text: 'Adatta alla larghezza', shortcut: '⌘0' }} onclick={() => setZoom(fitZoom(), true)}>Adatta</button>
    <button use:tooltip={'Dimensione reale (100%)'} onclick={() => setZoom(1)}>100%</button>
  </div>
</div>

<style>
  .viewer { display: flex; flex-direction: column; height: 100%; outline: none; }
  .pages { flex: 1; min-height: 0; overflow: auto; padding: 16px 24px 4px; background: var(--well); }
  .page { background: var(--paper); margin-left: auto; margin-right: auto; box-shadow: 0 2px 14px var(--shade); flex: none; }
  .msg { color: var(--faint); font-size: 13px; text-align: center; margin: 40px 0; }
  .foot { display: flex; align-items: center; gap: 8px; padding: 5px 12px; font-size: 11px; color: var(--dim); border-top: 1px solid var(--edge-soft); flex: none; }
  .grow { flex: 1; }
  .z { min-width: 40px; text-align: center; }
  button { background: var(--glass); border: 1px solid var(--edge-soft); color: var(--dim); border-radius: 6px; padding: 2px 9px; font-size: 11px; cursor: default; }
  button:hover { color: var(--txt); background: var(--glass2); }
  button.on { color: var(--accent); border-color: var(--accent); }
</style>
