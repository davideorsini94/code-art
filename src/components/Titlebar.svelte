<script lang="ts">
  import { projectStore } from '../lib/stores/project.svelte'

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
    <div class="proj" ondblclick={startEdit} role="presentation" title="Doppio click per rinominare">
      {projectStore.project?.name ?? 'Code Art'}
    </div>
  {/if}
  {#if projectStore.branch}
    <div class="branch" data-tauri-drag-region>⎇ {projectStore.branch}</div>
  {/if}
  <div class="pal" data-tauri-drag-region>⌘K &nbsp; Cerca file, conversazioni, comandi…</div>
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
  .pal {
    margin: 0 auto;
    width: 340px;
    padding: 6px 14px;
    border-radius: 999px;
    color: var(--faint);
    background: var(--glass);
    border: 1px solid var(--edge-soft);
    text-align: center;
    font-size: 12px;
  }
</style>
