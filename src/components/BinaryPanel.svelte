<script lang="ts">
  import { fsReadBytes } from '../lib/ipc'
  import { formatSize } from '../lib/blob'

  let { path, name }: { path: string; name: string } = $props()
  let size = $state<number | null>(null)
  let big = $state(false)

  // la dimensione si ricava leggendo i byte (tetto 50 MB del core); oltre il tetto lo diciamo e basta
  $effect(() => {
    const p = path
    let dead = false
    size = null
    big = false
    fsReadBytes(p)
      .then((b) => !dead && (size = b.byteLength))
      .catch((e) => !dead && (big = !!e && typeof e === 'object' && 'code' in e && e.code === 'too_large'))
    return () => (dead = true)
  })
</script>

<div class="panel">
  <p class="title">Anteprima non disponibile per questo tipo di file</p>
  <p class="meta">{name}</p>
  <p class="meta">{big ? 'oltre 50 MB' : size === null ? '' : formatSize(size)}</p>
</div>

<style>
  .panel { height: 100%; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 6px; }
  .title { margin: 0 0 6px; color: var(--dim); font-size: 14px; }
  .meta { margin: 0; color: var(--faint); font-size: 12px; overflow-wrap: anywhere; max-width: 80%; text-align: center; }
</style>
