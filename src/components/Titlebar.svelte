<script lang="ts">
  import { projectStore } from '../lib/stores/project.svelte'
  import { conversations } from '../lib/stores/conversations.svelte'
  import { activeLabel, waitingLabel } from '../lib/conv/order'
  import { palette } from '../lib/palette/state.svelte'
  import { hooks } from '../lib/palette/search'
  import { onMount } from 'svelte'
  import { tooltip } from '../lib/tooltip'
  import { errorText } from '../lib/errors'
  import { git } from '../lib/stores/git.svelte'
  import BranchPicker from './git/BranchPicker.svelte'

  let editing = $state(false)
  let draft = $state('')
  let renameError = $state<string | null>(null)
  let errorTimer: ReturnType<typeof setTimeout> | undefined

  function startEdit() {
    if (!projectStore.project) return
    draft = projectStore.project.name
    editing = true
  }

  async function commit() {
    if (!editing) return
    editing = false
    const name = draft.trim()
    if (!name || name === projectStore.project?.name) return
    clearTimeout(errorTimer)
    renameError = null
    try {
      await projectStore.rename(name)
    } catch (e) {
      renameError = `Impossibile rinominare: ${errorText(e)}`
      errorTimer = setTimeout(() => (renameError = null), 5000)
    }
  }

  async function openPicker() {
    await git.ensure()
    git.openPicker('switch')
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
  {#if renameError}<span class="err" role="alert">{renameError}</span>{/if}
  {#if projectStore.branch}
    <button class="branch" use:tooltip={'Cambia branch'} onclick={openPicker}>⎇ {projectStore.branch}</button>
  {/if}
  {#if conversations.active > 0}
    <div class="active" data-tauri-drag-region>{activeLabel(conversations.active)}</div>
  {/if}
  {#if projectStore.project}
    <button class="pal" use:tooltip={{ text: 'Cerca file, conversazioni e comandi', shortcut: '⌘K' }} onclick={() => palette.show('all')}>⌘K &nbsp; Cerca file, conversazioni, comandi…</button>
  {/if}
  {#if conversations.waiting > 0}
    <span class="chip" use:tooltip={'Conversazioni che aspettano una tua risposta'}><span class="dot"></span>{waitingLabel(conversations.waiting)}</span>
  {/if}
</header>
{#if git.picker}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="backdrop" data-no-tooltip="copertura invisibile: un clic fuori chiude il selettore" onpointerdown={() => git.closePicker()}></div>
  <div class="pop">
    {#key `${git.picker.repo}:${git.picker.mode}`}
      <BranchPicker repo={git.picker.repo} mode={git.picker.mode} onclose={() => git.closePicker()} />
    {/key}
  </div>
{/if}

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
  .err { font-size: 12px; color: var(--err); }
  .branch { padding: 2px 6px; border: 0; border-radius: 6px; background: none; font: inherit; font-size: 12px; color: var(--faint); font-family: ui-monospace, Menlo, monospace; cursor: pointer; }
  .branch:hover { background: var(--glass); color: var(--txt); }
  .backdrop { position: fixed; inset: 0; z-index: 40; background: none; }
  .pop { position: fixed; top: 46px; left: 90px; z-index: 41; }
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
