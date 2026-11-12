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
  {#await import('./git/GitPanel.svelte') then m}
    <m.default />
  {/await}
{/if}
