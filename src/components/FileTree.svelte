<script lang="ts">
  import { onMount } from 'svelte'
  import { listenProject } from '../lib/events'
  import { PATH_DRAG_TYPE, relativeToFolders } from '../lib/conv/paths'
  import { tooltip } from '../lib/tooltip'
  import { projectStore } from '../lib/stores/project.svelte'
  import { editor } from '../lib/stores/editor.svelte'
  import { asFsError, fsCreate, fsList, fsRename, fsTrash } from '../lib/ipc'
  import FileIcon from './FileIcon.svelte'
  import { iconFor } from '../lib/fileicons'
  import type { DirEntry, FsChanged } from '../lib/types'

  let children = $state<Record<string, DirEntry[]>>({})
  // cartella non fidata: sola lettura (il core rifiuta comunque ogni modifica)
  const readOnly = $derived(projectStore.untrusted.length > 0)
  let expanded = $state<Record<string, boolean>>({})
  let menu = $state<{ x: number; y: number; entry: DirEntry } | null>(null)
  // inserimento in corso: nuovo file/cartella dentro `parent`, oppure rinomina di `path`
  let editing = $state<{ kind: 'file' | 'dir' | 'rename'; parent?: string; path?: string; value: string } | null>(null)
  let dropTarget = $state<string | null>(null)
  let error = $state<string | null>(null)

  const folders = $derived(projectStore.project?.folders ?? [])
  const parentOf = (p: string) => p.slice(0, p.lastIndexOf('/')) || '/'
  const nameOf = (p: string) => p.slice(p.lastIndexOf('/') + 1)

  async function load(dir: string) {
    try {
      children[dir] = await fsList(dir)
    } catch {
      delete children[dir]
    }
  }

  async function toggle(e: DirEntry) {
    expanded[e.path] = !expanded[e.path]
    if (expanded[e.path] && !children[e.path]) await load(e.path)
  }

  $effect(() => {
    for (const f of folders) {
      if (expanded[f] === undefined) expanded[f] = true
      if (!children[f]) void load(f)
    }
  })

  // eventi del watcher del progetto della finestra (il progetto si carica dopo il montaggio)
  $effect(() => {
    const pid = projectStore.project?.id
    if (!pid) return
    let unlisten: (() => void) | undefined
    let gone = false
    void listenProject<FsChanged>('fs:changed', pid, (ev) => {
      const dirs = new Set(ev.payload.paths.map(parentOf))
      for (const d of dirs) if (children[d]) void load(d)
      // una cartella creata/rimossa cambia anche l'elenco del suo genitore (già coperto da parentOf)
    }).then((u) => (gone ? u() : (unlisten = u)))
    return () => {
      gone = true
      unlisten?.()
    }
  })

  onMount(() => {
    const close = () => (menu = null)
    window.addEventListener('click', close)
    return () => window.removeEventListener('click', close)
  })

  async function run(op: () => Promise<unknown>): Promise<boolean> {
    error = null
    try {
      await op()
      return true
    } catch (e) {
      const err = asFsError(e)
      error = err.code === 'forbidden' ? 'Percorso fuori dal progetto' : err.message
      return false
    }
  }

  function startNew(kind: 'file' | 'dir', dir: string) {
    menu = null
    expanded[dir] = true
    if (!children[dir]) void load(dir)
    editing = { kind, parent: dir, value: '' }
  }

  function startRename(e: DirEntry) {
    menu = null
    editing = { kind: 'rename', path: e.path, value: e.name }
  }

  async function commit() {
    const ed = editing
    editing = null
    const value = ed?.value.trim()
    if (!ed || !value || value.includes('/')) return
    if (ed.kind === 'rename' && ed.path) {
      const to = `${parentOf(ed.path)}/${value}`
      if (to !== ed.path) {
        await run(async () => {
          await fsRename(ed.path!, to)
          editor.repath(ed.path!, to)
        })
      }
    } else if (ed.parent) {
      const path = `${ed.parent}/${value}`
      await run(async () => {
        await fsCreate(path, ed.kind === 'dir')
        await load(ed.parent!)
        if (ed.kind === 'file') await editor.open(path)
      })
    }
    for (const d of Object.keys(children)) void load(d)
  }

  async function trash(e: DirEntry) {
    menu = null
    if (await run(() => fsTrash(e.path))) editor.closeUnder(e.path)
    void load(parentOf(e.path))
  }

  function onDrop(ev: DragEvent, target: DirEntry) {
    ev.preventDefault()
    dropTarget = null
    if (readOnly) return
    const from = ev.dataTransfer?.getData('text/plain')
    if (!from || !target.isDir || from === target.path || target.path.startsWith(from + '/')) return
    const to = `${target.path}/${nameOf(from)}`
    void run(async () => {
      await fsRename(from, to)
      editor.repath(from, to)
      await Promise.all([load(parentOf(from)), load(target.path)])
      expanded[target.path] = true
    })
  }
</script>

{#snippet inputRow(depth: number)}
  <!-- svelte-ignore a11y_autofocus -->
  <input
    class="edit"
    style:padding-left="{8 + depth * 14}px"
    autofocus
    bind:value={editing!.value}
    onkeydown={(e) => {
      if (e.key === 'Enter') void commit()
      else if (e.key === 'Escape') editing = null
    }}
    onblur={() => void commit()}
  />
{/snippet}

{#snippet node(e: DirEntry, depth: number)}
  {#if editing?.kind === 'rename' && editing.path === e.path}
    {@render inputRow(depth)}
  {:else}
    {@const spec = iconFor(e.name, e.isDir, !!expanded[e.path])}
    {@const isRoot = depth === 0 && folders.includes(e.path)}
    <div
      class="row"
      class:ignored={e.ignored}
      class:active={editor.active === e.path}
      class:drop={dropTarget === e.path}
      style:padding-left="{8 + depth * 14}px"
      role="treeitem"
      use:tooltip={isRoot ? e.path : relativeToFolders(e.path, folders)}
      aria-selected={editor.active === e.path}
      aria-expanded={e.isDir ? !!expanded[e.path] : undefined}
      tabindex="0"
      draggable="true"
      ondragstart={(ev) => {
        ev.dataTransfer?.setData('text/plain', e.path)
        ev.dataTransfer?.setData(PATH_DRAG_TYPE, e.path)
      }}
      ondragover={(ev) => {
        if (e.isDir && !readOnly) {
          ev.preventDefault()
          dropTarget = e.path
        }
      }}
      ondragleave={() => (dropTarget = null)}
      ondrop={(ev) => onDrop(ev, e)}
      onclick={() => (e.isDir ? void toggle(e) : void editor.open(e.path))}
      onkeydown={(ev) => ev.key === 'Enter' && (e.isDir ? void toggle(e) : void editor.open(e.path))}
      oncontextmenu={(ev) => {
        ev.preventDefault()
        ev.stopPropagation()
        menu = { x: ev.clientX, y: ev.clientY, entry: e }
      }}
    >
      <span class="chev" class:open={e.isDir && expanded[e.path]} aria-hidden="true">
        {#if e.isDir}
          <svg width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M3.500 2 6.500 5 3.500 8" /></svg>
        {/if}
      </span>
      <span class="ic" class:loading={e.isDir && expanded[e.path] && !children[e.path]}>
        <FileIcon id={spec.id} dim={spec.dim} size={isRoot ? 17 : 16} root={isRoot} />
      </span>
      <span class="name">{e.name}</span>
    </div>
    {#if e.isDir && expanded[e.path]}
      {#if editing && editing.kind !== 'rename' && editing.parent === e.path}
        {@render inputRow(depth + 1)}
      {/if}
      {#each children[e.path] ?? [] as c (c.path)}
        {@render node(c, depth + 1)}
      {/each}
    {/if}
  {/if}
{/snippet}

<div class="tree" role="tree">
  {#each folders as f (f)}
    {@const root: DirEntry = { name: nameOf(f), path: f, isDir: true, ignored: false }}
    {@render node(root, 0)}
  {/each}
  {#if error}<p class="err">{error}</p>{/if}
</div>

{#if menu}
  <div class="menu glass-strong" style:left="{menu.x}px" style:top="{menu.y}px" role="menu">
    {#if menu.entry.isDir}
      <button role="menuitem" aria-disabled={readOnly} use:tooltip={readOnly ? 'Cartella non fidata: sola lettura' : 'Crea un nuovo file in questa cartella'} onclick={() => !readOnly && startNew('file', menu!.entry.path)}>Nuovo file</button>
      <button role="menuitem" aria-disabled={readOnly} use:tooltip={readOnly ? 'Cartella non fidata: sola lettura' : 'Crea una nuova cartella qui dentro'} onclick={() => !readOnly && startNew('dir', menu!.entry.path)}>Nuova cartella</button>
    {/if}
    <button role="menuitem" aria-disabled={readOnly} use:tooltip={readOnly ? 'Cartella non fidata: sola lettura' : 'Cambia il nome'} onclick={() => !readOnly && startRename(menu!.entry)}>Rinomina</button>
    <button role="menuitem" aria-disabled={readOnly} use:tooltip={readOnly ? 'Cartella non fidata: sola lettura' : 'Sposta nel Cestino (si può recuperare)'} onclick={() => !readOnly && void trash(menu!.entry)}>Sposta nel Cestino</button>
  </div>
{/if}

<style>
  .tree { height: 100%; overflow: auto; font-size: 12.5px; user-select: none; }
  .row {
    display: flex; align-items: center; gap: 5px; height: 24px; padding-right: 8px;
    border-radius: 6px; cursor: default; color: var(--txt); white-space: nowrap;
    transition: background var(--dur);
  }
  .row:hover { background: var(--glass); }
  .row.active { background: var(--glass2); }
  .row.drop { outline: 1px solid var(--accent); }
  .row.ignored { opacity: 0.45; }
  .chev { width: 10px; height: 10px; color: var(--faint); flex: none; display: flex; }
  .chev svg { transition: transform 170ms ease; }
  .chev.open svg { transform: rotate(90deg); }
  .row:hover .chev { color: var(--dim); }
  .ic { width: 18px; height: 18px; flex: none; display: flex; align-items: center; justify-content: center; }
  .ic.loading { opacity: 0.55; animation: pulse 1s ease-in-out infinite; }
  @keyframes pulse { 50% { opacity: 0.9; } }
  @media (prefers-reduced-motion: reduce) {
    .chev svg { transition: none; }
    .ic.loading { animation: none; }
  }
  .name { overflow: hidden; text-overflow: ellipsis; }
  .edit {
    width: 100%; box-sizing: border-box; height: 24px; background: var(--glass2); color: var(--txt);
    border: 1px solid var(--accent); border-radius: 6px; font: inherit; outline: none;
  }
  .err { margin: 8px; color: var(--wait); font-size: 12px; }
  .menu {
    position: fixed; z-index: 90; min-width: 170px; padding: 4px; display: flex; flex-direction: column;
    background: rgba(24, 27, 38, 0.96);
  }
  .menu button {
    text-align: left; background: none; border: 0; color: var(--txt); padding: 6px 10px;
    border-radius: 6px; font-size: 12.5px; cursor: default;
  }
  .menu button:hover:not([aria-disabled='true']) { background: var(--glass2); }
  .menu button[aria-disabled='true'] { opacity: 0.4; cursor: default; }
</style>
