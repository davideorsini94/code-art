<script lang="ts">
  import FileTree from './FileTree.svelte'
  import SearchPanel from './SearchPanel.svelte'
  import { sidebar } from '../lib/stores/sidebar.svelte'
</script>

{#if sidebar.view === 'files'}
  <FileTree />
{:else if sidebar.view === 'search'}
  <SearchPanel />
{:else}
  <p class="placeholder">Git arriva a breve.</p>
{/if}

<style>
  .placeholder { margin: 8px; font-size: 12px; color: var(--dim); }
</style>
