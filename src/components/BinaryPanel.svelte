<script lang="ts">
  import { fsStat } from '../lib/ipc'
  import { formatSize } from '../lib/blob'

  let { path, name }: { path: string; name: string } = $props()
  let size = $state<number | null>(null)

  // la dimensione viene dai metadati (fs_stat): il file non si legge
  $effect(() => {
    const p = path
    let dead = false
    size = null
    fsStat(p)
      .then((st) => !dead && (size = st.size))
      .catch(() => {})
    return () => (dead = true)
  })
</script>

<div class="panel">
  <p class="title">Anteprima non disponibile per questo tipo di file</p>
  <p class="meta">{name}</p>
  <p class="meta">{size === null ? '' : formatSize(size)}</p>
</div>

<style>
  .panel { height: 100%; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 6px; }
  .title { margin: 0 0 6px; color: var(--dim); font-size: 14px; }
  .meta { margin: 0; color: var(--faint); font-size: 12px; overflow-wrap: anywhere; max-width: 80%; text-align: center; }
</style>
