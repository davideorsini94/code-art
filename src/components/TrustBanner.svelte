<script lang="ts">
  import { projectStore } from '../lib/stores/project.svelte'
</script>

{#if projectStore.project && projectStore.untrusted.length > 0}
  <div class="trust" role="status">
    <span>Cartella non fidata — Claude e terminale disattivati</span>
    <span aria-hidden="true">·</span>
    <button onclick={() => void projectStore.acceptTrust()}>Mi fido</button>
  </div>
{/if}

<style>
  .trust {
    display: flex;
    align-items: center;
    gap: 8px;
    margin: 0 10px;
    padding: 4px 12px;
    border-radius: 8px;
    font-size: 12px;
    color: var(--wait);
    background: rgba(255, 180, 84, 0.1);
    border: 1px solid rgba(255, 180, 84, 0.3);
  }
  button {
    padding: 0;
    border: none;
    background: none;
    color: inherit;
    font: inherit;
    text-decoration: underline;
    cursor: pointer;
  }
</style>
