<script lang="ts">
  // Anteprima markdown: l'HTML nasce solo da renderMarkdown (sanificato). Le immagini locali (percorsi relativi
  // al file, dentro il progetto) si caricano con fs_read_bytes e diventano URL blob: la rete non entra mai.
  import { onDestroy } from 'svelte'
  import { renderMarkdown } from '../lib/safe-markdown'
  import { links } from '../lib/conv/links'
  import { fsReadBytes } from '../lib/ipc'
  import { projectStore } from '../lib/stores/project.svelte'
  import { mimeFor, resolveMarkdownImage } from '../lib/viewers'

  let { text, path }: { text: string; path: string } = $props()

  const PENDING = 'blob:codeart-pending/' // segnaposto: la pagina lo sostituisce subito, non viene mai richiesto
  const cache = new Map<string, string>() // percorso assoluto -> URL blob
  const inflight = new Map<string, Promise<string | null>>()
  let host = $state<HTMLElement>()

  const opts = $derived({
    resolveImage: (href: string) => {
      const abs = resolveMarkdownImage(href, path, projectStore.project?.folders ?? [])
      return abs ? (cache.get(abs) ?? PENDING + encodeURIComponent(abs)) : null
    },
  })

  function blobFor(abs: string): Promise<string | null> {
    const hit = cache.get(abs)
    if (hit) return Promise.resolve(hit)
    let p = inflight.get(abs)
    if (!p) {
      p = fsReadBytes(abs)
        .then((bytes) => {
          const url = URL.createObjectURL(new Blob([bytes as BlobPart], { type: mimeFor(abs) ?? 'application/octet-stream' }))
          cache.set(abs, url)
          return url
        })
        .catch(() => null)
        .finally(() => inflight.delete(abs))
      inflight.set(abs, p)
    }
    return p
  }

  // dopo ogni render: sostituisce i segnaposto con gli URL blob
  $effect(() => {
    void text
    void path
    const el = host
    if (!el) return
    for (const img of el.querySelectorAll('img')) {
      const src = img.getAttribute('src') ?? ''
      if (!src.startsWith(PENDING)) continue
      let abs: string
      try {
        abs = decodeURIComponent(src.slice(PENDING.length))
      } catch {
        continue
      }
      void blobFor(abs).then((url) => {
        if (url && img.isConnected) img.setAttribute('src', url)
        else if (img.isConnected) img.replaceWith(document.createTextNode(img.getAttribute('alt') ?? ''))
      })
    }
  })

  onDestroy(() => {
    for (const url of cache.values()) URL.revokeObjectURL(url)
    cache.clear()
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
  .md :global(pre) { background: rgba(0, 0, 0, 0.32); border: 1px solid var(--edge-soft); border-radius: 10px; padding: 12px 14px; overflow: auto; line-height: 1.5; }
  .md :global(pre code) { background: none; padding: 0; font-size: 12.5px; }
  .md :global(table) { border-collapse: collapse; display: block; overflow-x: auto; max-width: 100%; }
  .md :global(th), .md :global(td) { border: 1px solid var(--edge-soft); padding: 6px 12px; }
  .md :global(th) { background: var(--glass2); text-align: left; }
  .md :global(tr:nth-child(even) td) { background: var(--glass); }
  .md :global(hr) { border: 0; border-top: 1px solid var(--edge); margin: 1.6em 0; }
  .md :global(img) { max-width: 100%; height: auto; border-radius: 6px; }
</style>
