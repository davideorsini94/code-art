<script lang="ts">
  import { open } from '@tauri-apps/plugin-dialog'
  import { projectStore } from '../lib/stores/project.svelte'

  async function choose(old: string) {
    const picked = await open({ directory: true, multiple: false, title: 'Scegli la nuova posizione' })
    if (typeof picked === 'string') await projectStore.relocate(old, picked)
  }
</script>

{#if projectStore.missing.length > 0}
  <div class="cover" role="alertdialog" aria-label="Cartella non trovata">
    <div class="box glass">
      <h2>Cartella non trovata</h2>
      {#each projectStore.missing as folder (folder)}
        <p class="path">{folder}</p>
        <button onclick={() => choose(folder)}>Scegli nuova posizione</button>
      {/each}
    </div>
  </div>
{/if}

<style>
  .cover {
    position: fixed;
    inset: 0;
    z-index: 100;
    display: grid;
    place-items: center;
    background: rgba(10, 12, 18, 0.7);
  }
  .box { padding: 24px 28px; max-width: 460px; display: flex; flex-direction: column; gap: 10px; }
  h2 { margin: 0; font-size: 16px; }
  .path { margin: 0; font-family: ui-monospace, Menlo, monospace; font-size: 12px; color: var(--faint); word-break: break-all; }
  button {
    align-self: flex-start;
    padding: 6px 14px;
    border-radius: 8px;
    border: 1px solid var(--edge-soft);
    background: var(--glass);
    color: inherit;
    cursor: pointer;
  }
  button:hover { border-color: #8fa8ff; }
</style>
