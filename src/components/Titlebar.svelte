<script lang="ts">
  import { projectStore } from '../lib/stores/project.svelte'
  import { conversations } from '../lib/stores/conversations.svelte'
  import { activeLabel, waitingLabel } from '../lib/conv/order'
  import { palette } from '../lib/palette/state.svelte'
  import { hooks } from '../lib/palette/search'
  import { onMount } from 'svelte'
  import { tooltip } from '../lib/tooltip'

  let editing = $state(false)
  let draft = $state('')

  function startEdit() {
    if (!projectStore.project) return
    draft = projectStore.project.name
    editing = true
  }

  async function commit() {
    if (!editing) return
    editing = false
    const name = draft.trim()
    if (name && name !== projectStore.project?.name) await projectStore.rename(name)
  }

  onMount(() => {
    hooks.renameProject = startEdit
  })

  function focusSelect(node: HTMLInputElement) {
    node.focus()
    node.select()
  }
</script>

<header class="bar" data-tauri-drag-region>
  <div class="traffic-space" data-tauri-drag-region></div>
  {#if editing}
    <input
      class="rename"
      aria-label="Nome del progetto"
      bind:value={draft}
      use:focusSelect
      onblur={commit}
      onkeydown={(e) => {
        if (e.key === 'Enter') void commit()
        else if (e.key === 'Escape') editing = false
      }}
    />
  {:else}
    <div class="proj" ondblclick={startEdit} role="presentation" use:tooltip={'Rinomina progetto (doppio clic)'}>
      {projectStore.project?.name ?? 'Code Art'}
    </div>
  {/if}
  {#if projectStore.branch}
    <div class="branch" data-tauri-drag-region use:tooltip={`Ramo Git: ${projectStore.branch}`}>⎇ {projectStore.branch}</div>
  {/if}
  {#if conversations.active > 0}
    <div class="active" data-tauri-drag-region>{activeLabel(conversations.active)}</div>
  {/if}
  <button class="pal" use:tooltip={{ text: 'Cerca file, conversazioni e comandi', shortcut: '⌘K' }} onclick={() => palette.show('all')}>⌘K &nbsp; Cerca file, conversazioni, comandi…</button>
  {#if conversations.waiting > 0}
    <span class="chip" use:tooltip={'Conversazioni che aspettano una tua risposta'}><span class="dot"></span>{waitingLabel(conversations.waiting)}</span>
  {/if}
</header>

<style>
  .bar {
    display: flex;
    align-items: center;
    gap: 14px;
    height: 44px;
    padding: 0 14px;
    border-bottom: 1px solid var(--edge-soft);
  }
  .traffic-space { width: 70px; flex: none; height: 100%; }
  .proj { font-size: 15px; font-weight: 650; letter-spacing: -0.01em; }
  .rename {
    font: inherit;
    font-size: 15px;
    font-weight: 650;
    color: inherit;
    width: 200px;
    padding: 2px 6px;
    border-radius: 6px;
    border: 1px solid var(--edge-soft);
    background: var(--glass);
    outline: none;
  }
  .branch { font-size: 12px; color: var(--faint); font-family: ui-monospace, Menlo, monospace; }
  .active { font-size: 12px; color: var(--faint); white-space: nowrap; }
  .chip {
    display: inline-flex; align-items: center; gap: 7px; padding: 4px 11px; border-radius: 999px;
    background: var(--glass); border: 1px solid var(--edge-soft); font-size: 12px; white-space: nowrap;
  }
  .chip .dot { width: 8px; height: 8px; border-radius: 50%; background: var(--wait); box-shadow: 0 0 10px var(--wait); }
  .pal {
    margin: 0 auto;
    width: 340px;
    padding: 6px 14px;
    border-radius: 999px;
    color: var(--faint);
    background: var(--glass);
    border: 1px solid var(--edge-soft);
    text-align: center;
    font: inherit;
    font-size: 12px;
    cursor: pointer;
  }
</style>
