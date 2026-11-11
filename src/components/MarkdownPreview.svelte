<script lang="ts">
  // Anteprima markdown: l'HTML nasce solo da renderMarkdown (sanificato). Le immagini locali (percorsi relativi
  // al file, dentro il progetto) si caricano con fs_read_bytes e diventano URL blob: la rete non entra mai.
  import { onDestroy } from 'svelte'
  import { renderMarkdown } from '../lib/safe-markdown'
  import { links } from '../lib/conv/links'
  import { fsReadBytes } from '../lib/ipc'
  import { ImageLoader } from '../lib/image-budget'
  import { projectStore } from '../lib/stores/project.svelte'
  import { mimeFor, resolveMarkdownImage } from '../lib/viewers'

  let { text, path }: { text: string; path: string } = $props()

  const PENDING = 'blob:codeart-pending/' // segnaposto: la pagina lo sostituisce, non viene mai richiesto
  const ready = new Map<string, string>() // percorso assoluto -> URL blob già caricato
  let alive = true
  let host = $state<HTMLElement>()

  // al massimo 4 caricamenti alla volta, 100 immagini, 10 MB l'una e 60 MB per documento (vedi image-budget.ts)
  const loader = new ImageLoader(
    async (abs, maxBytes) => {
      const bytes = await fsReadBytes(abs, maxBytes)
      if (!alive) throw new Error('smontato')
      const url = URL.createObjectURL(new Blob([bytes as BlobPart], { type: mimeFor(abs) ?? 'application/octet-stream' }))
      return { url, size: bytes.byteLength }
    },
    (url) => URL.revokeObjectURL(url),
  )

  const opts = $derived({
    resolveImage: (href: string) => {
      const abs = resolveMarkdownImage(href, path, projectStore.project?.folders ?? [])
      return abs ? (ready.get(abs) ?? PENDING + encodeURIComponent(abs)) : null
    },
  })

  function missing(img: HTMLImageElement) {
    const span = document.createElement('span')
    span.className = 'imgmiss'
    span.textContent = `immagine non caricata${img.alt ? `: ${img.alt}` : ''}`
    img.replaceWith(span)
  }

  function fill(img: HTMLImageElement, abs: string) {
    void loader.request(abs).then((url) => {
      if (!alive || !img.isConnected) return
      if (url) {
        ready.set(abs, url)
        img.setAttribute('src', url)
      } else missing(img)
    })
  }

  // dopo ogni render: i segnaposto entrano in coda solo quando si avvicinano alla vista
  $effect(() => {
    void text
    void path
    const el = host
    if (!el) return
    const io =
      typeof IntersectionObserver === 'undefined'
        ? null
        : new IntersectionObserver(
            (entries) => {
              for (const e of entries) {
                if (!e.isIntersecting) continue
                io?.unobserve(e.target)
                const img = e.target as HTMLImageElement
                const abs = (img as HTMLImageElement & { _abs?: string })._abs
                if (abs) fill(img, abs)
              }
            },
            { root: el.parentElement, rootMargin: '600px 0px' },
          )
    for (const img of el.querySelectorAll('img')) {
      const src = img.getAttribute('src') ?? ''
      if (!src.startsWith(PENDING)) continue
      let abs: string
      try {
        abs = decodeURIComponent(src.slice(PENDING.length))
      } catch {
        continue
      }
      if (io) {
        ;(img as HTMLImageElement & { _abs?: string })._abs = abs
        io.observe(img)
      } else fill(img, abs)
    }
    return () => io?.disconnect()
  })

  onDestroy(() => {
    alive = false
    loader.dispose()
    ready.clear()
  })
</script>

<div class="preview" data-testid="md-preview">
  <article class="md" bind:this={host} use:links>{@html renderMarkdown(text, opts)}</article>
</div>

<style>
  .preview { height: 100%; overflow: auto; padding: 22px 28px 48px; box-sizing: border-box; }
  .md { max-width: 820px; margin: 0 auto; font-size: 14px; line-height: 1.7; color: var(--txt); overflow-wrap: anywhere; user-select: text; }
  .md :global(h1), .md :global(h2), .md :global(h3), .md :global(h4), .md :global(h5), .md :global(h6) { line-height: 1.3; margin: 1.6em 0 0.6em; font-weight: 650; }
  .md :global(h1) { font-size: 1.9em; padding-bottom: 0.3em; border-bottom: 1px solid var(--edge-soft); margin-top: 0.4em; }
  .md :global(h2) { font-size: 1.5em; padding-bottom: 0.25em; border-bottom: 1px solid var(--edge-soft); }
  .md :global(h3) { font-size: 1.2em; }
  .md :global(h4), .md :global(h5), .md :global(h6) { font-size: 1em; color: var(--dim); }
  .md :global(p), .md :global(ul), .md :global(ol), .md :global(blockquote), .md :global(pre), .md :global(table) { margin: 0 0 1em; }
  .md :global(ul), .md :global(ol) { padding-left: 1.6em; }
  .md :global(li + li) { margin-top: 0.25em; }
  .md :global(a) { color: var(--accent); text-decoration: none; }
  .md :global(a:hover) { text-decoration: underline; }
  .md :global(blockquote) { padding: 0.2em 1em; color: var(--dim); border-left: 3px solid var(--edge); background: var(--glass); border-radius: 0 6px 6px 0; }
  .md :global(blockquote > :last-child) { margin-bottom: 0; }
  .md :global(code) { font-family: ui-monospace, Menlo, monospace; font-size: 0.88em; background: var(--glass2); padding: 0.15em 0.4em; border-radius: 5px; }
  .md :global(pre) { background: var(--well); border: 1px solid var(--edge-soft); border-radius: 10px; padding: 12px 14px; overflow: auto; line-height: 1.5; }
  .md :global(pre code) { background: none; padding: 0; font-size: 12.5px; }
  .md :global(table) { border-collapse: collapse; display: block; overflow-x: auto; max-width: 100%; }
  .md :global(th), .md :global(td) { border: 1px solid var(--edge-soft); padding: 6px 12px; }
  .md :global(th) { background: var(--glass2); text-align: left; }
  .md :global(tr:nth-child(even) td) { background: var(--glass); }
  .md :global(hr) { border: 0; border-top: 1px solid var(--edge); margin: 1.6em 0; }
  .md :global(.imgmiss) { color: var(--faint); font-size: 12px; font-style: italic; }
  .md :global(img) { max-width: 100%; height: auto; border-radius: 6px; }
</style>
