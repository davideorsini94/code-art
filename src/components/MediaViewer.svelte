<script lang="ts">
  import { loadBlobUrl, formatSize } from '../lib/blob'
  import { asFsError } from '../lib/ipc'
  import { isVideo } from '../lib/viewers'

  let { path, name, rev }: { path: string; name: string; rev: number } = $props()
  let url = $state<string | null>(null)
  let size = $state(0)
  let failed = $state<string | null>(null)
  const video = $derived(isVideo(name))

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
        if (!dead) failed = asFsError(e).code === 'too_large' ? 'File troppo grande per l\'anteprima (oltre 50 MB)' : 'Impossibile leggere il file'
      })
    return () => {
      dead = true
      if (mine) URL.revokeObjectURL(mine)
    }
  })
</script>

<div class="viewer">
  <div class="stage">
    {#if failed}
      <p class="msg">{failed}</p>
    {:else if !url}
      <p class="msg">Caricamento…</p>
    {:else if video}
      <!-- svelte-ignore a11y_media_has_caption -->
      <video controls src={url}></video>
    {:else}
      <div class="audio">
        <p class="aname">{name}</p>
        <audio controls src={url}></audio>
      </div>
    {/if}
  </div>
  <div class="foot"><span>{formatSize(size)}</span></div>
</div>

<style>
  .viewer { display: flex; flex-direction: column; height: 100%; }
  .stage { flex: 1; min-height: 0; display: grid; place-items: center; padding: 16px; background: rgba(0, 0, 0, 0.28); }
  video { max-width: 100%; max-height: 100%; border-radius: 8px; }
  .audio { display: flex; flex-direction: column; align-items: center; gap: 14px; }
  .aname { margin: 0; color: var(--dim); font-size: 13px; }
  audio { width: min(480px, 90vw); }
  .msg { color: var(--faint); font-size: 13px; margin: 0; }
  .foot { padding: 5px 12px; font-size: 11px; color: var(--dim); border-top: 1px solid var(--edge-soft); flex: none; }
</style>
