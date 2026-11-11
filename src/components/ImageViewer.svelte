<script lang="ts">
  import { loadBlobUrl, formatSize } from '../lib/blob'
  import { asFsError } from '../lib/ipc'
  import { tooltip } from '../lib/tooltip'

  let { path, name, rev, onAsText }: { path: string; name: string; rev: number; onAsText?: () => void } = $props()

  let url = $state<string | null>(null)
  let size = $state(0)
  let failed = $state<string | null>(null)
  let nat = $state<{ w: number; h: number; synthetic?: boolean } | null>(null)
  let imgEl = $state<HTMLImageElement>()
  let zoom = $state<'fit' | number>('fit')
  let root = $state<HTMLDivElement>()
  const isSvg = $derived(name.toLowerCase().endsWith('.svg'))

  // carica (e ricarica a ogni modifica su disco); l'URL precedente si revoca
  $effect(() => {
    void rev
    const p = path
    const n = name
    let dead = false
    let mine: string | null = null
    failed = null
    loadBlobUrl(p, n)
      .then((r) => {
        if (dead) return URL.revokeObjectURL(r.url)
        mine = r.url
        url = r.url
        size = r.size
      })
      .catch((e) => {
        if (!dead) failed = asFsError(e).code === 'too_large' ? 'Immagine troppo grande per l\'anteprima (oltre 50 MB)' : 'Impossibile leggere l\'immagine'
      })
    return () => {
      dead = true
      if (mine) URL.revokeObjectURL(mine)
    }
  })

  const STEPS = [0.1, 0.25, 0.5, 0.75, 1, 1.5, 2, 3, 4, 8]
  // da "Adatta" si parte dalla scala davvero mostrata, non da 1
  const cur = () => {
    if (zoom !== 'fit') return zoom
    const w = imgEl?.getBoundingClientRect().width
    return nat && w ? w / nat.w : 1
  }
  function step(dir: 1 | -1) {
    const c = cur()
    const next = dir > 0 ? STEPS.find((s) => s > c + 1e-6) : [...STEPS].reverse().find((s) => s < c - 1e-6)
    if (next) zoom = next
  }
  function key(e: KeyboardEvent) {
    if (!e.metaKey) return
    if (e.key === '+' || e.key === '=') step(1)
    else if (e.key === '-') step(-1)
    else if (e.key === '0') zoom = 'fit'
    else return
    e.preventDefault()
  }
  $effect(() => root?.focus())
</script>

<!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
<div class="viewer" bind:this={root} tabindex="0" role="application" aria-label="Anteprima immagine" data-no-tooltip="contenitore di scelte da tastiera: i controlli al suo interno hanno il proprio suggerimento" onkeydown={key}>
  <div class="stage" class:fit={zoom === 'fit'}>
    {#if failed}
      <p class="msg">{failed}</p>
    {:else if url}
      <img
        src={url}
        alt={name}
        draggable="false"
        bind:this={imgEl}
        style:width={nat?.synthetic ? `${nat.w * (zoom === 'fit' ? 1 : zoom)}px` : zoom !== 'fit' && nat ? `${nat.w * zoom}px` : undefined}
        onload={(e) => {
          const i = e.currentTarget as HTMLImageElement
          // senza dimensioni proprie (es. SVG senza viewBox) si usa un riquadro predefinito di 512 px
          nat = i.naturalWidth > 0 && i.naturalHeight > 0 ? { w: i.naturalWidth, h: i.naturalHeight } : { w: 512, h: 512, synthetic: true }
        }}
      />
    {:else}
      <p class="msg">Caricamento…</p>
    {/if}
  </div>
  <div class="foot">
    <span>{nat && !nat.synthetic ? `${nat.w} × ${nat.h} px` : '—'}</span>
    <span class="sep">·</span>
    <span>{formatSize(size)}</span>
    <span class="grow"></span>
    {#if isSvg && onAsText}<button use:tooltip={'Apri il file SVG come testo nell\'editor'} onclick={onAsText}>Apri come testo</button>{/if}
    <button use:tooltip={{ text: 'Riduci', shortcut: '⌘−' }} onclick={() => step(-1)} aria-label="Riduci">−</button>
    <span class="z">{zoom === 'fit' ? 'Adatta' : `${Math.round(zoom * 100)}%`}</span>
    <button use:tooltip={{ text: 'Ingrandisci', shortcut: '⌘+' }} onclick={() => step(1)} aria-label="Ingrandisci">+</button>
    <button class:on={zoom === 'fit'} use:tooltip={{ text: 'Adatta alla finestra', shortcut: '⌘0' }} onclick={() => (zoom = 'fit')}>Adatta</button>
    <button class:on={zoom === 1} use:tooltip={'Dimensione reale (100%)'} onclick={() => (zoom = 1)}>100%</button>
  </div>
</div>

<style>
  .viewer { display: flex; flex-direction: column; height: 100%; outline: none; }
  .stage { flex: 1; min-height: 0; overflow: auto; display: grid; place-items: center; padding: 16px; background: var(--well); }
  .stage.fit { overflow: hidden; }
  .stage.fit img { max-width: 100%; max-height: 100%; object-fit: contain; }
  .stage:not(.fit) { place-items: safe center; }
  img { display: block; image-rendering: auto; }
  .msg { color: var(--faint); font-size: 13px; margin: 0; }
  .foot { display: flex; align-items: center; gap: 8px; padding: 5px 12px; font-size: 11px; color: var(--dim); border-top: 1px solid var(--edge-soft); flex: none; }
  .sep { color: var(--faint); }
  .grow { flex: 1; }
  .z { min-width: 46px; text-align: center; }
  button { background: var(--glass); border: 1px solid var(--edge-soft); color: var(--dim); border-radius: 6px; padding: 2px 9px; font-size: 11px; cursor: default; }
  button:hover { color: var(--txt); background: var(--glass2); }
  button.on { color: var(--accent); border-color: var(--accent); }
</style>
