<script lang="ts">
  import FileTree from './FileTree.svelte'
  import SearchPanel from './SearchPanel.svelte'
  import SshPanel from './ssh/SshPanel.svelte'
  import ConvList from './ConvList.svelte'
  import { sidebar } from '../lib/stores/sidebar.svelte'
</script>

{#if sidebar.view === 'files'}
  <FileTree />
{:else if sidebar.view === 'search'}
  <SearchPanel />
{:else if sidebar.view === 'ssh'}
  <SshPanel />
{:else if sidebar.view === 'convs'}
  <ConvList />
{:else if sidebar.view === 'db'}
  {#await import('./db/DbPanel.svelte') then m}
    <m.default />
  {:catch}
    <p class="empty">Vista Database non caricata</p>
  {/await}
{:else}
  {#await import('./git/GitPanel.svelte') then m}
    <m.default />
  {:catch}
    <p class="empty">Vista Git non caricata</p>
  {/await}
{/if}

<style>
  .empty { margin: 0; padding: 16px; color: var(--faint); font-size: 13px; text-align: center; }
</style>
