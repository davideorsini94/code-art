<script lang="ts">
  import { onMount } from 'svelte'
  import { listen } from '@tauri-apps/api/event'
  import { PATH_DRAG_TYPE } from '../lib/conv/paths'
  import { projectStore } from '../lib/stores/project.svelte'
  import { editor } from '../lib/stores/editor.svelte'
  import { asFsError, fsCreate, fsList, fsRename, fsTrash } from '../lib/ipc'
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

  onMount(() => {
    let unlisten: (() => void) | undefined
    void listen<FsChanged>('fs:changed', (ev) => {
      const dirs = new Set(ev.payload.paths.map(parentOf))
      for (const d of dirs) if (children[d]) void load(d)
      // una cartella creata/rimossa cambia anche l'elenco del suo genitore (già coperto da parentOf)
    }).then((u) => (unlisten = u))
    const close = () => (menu = null)
    window.addEventListener('click', close)
    return () => {
      unlisten?.()
      window.removeEventListener('click', close)
    }
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

  const ICONS: Record<string, [string, string]> = {
    ts: ['TS', '#6aa3ff'], tsx: ['TX', '#6aa3ff'], js: ['JS', '#f2d36b'], jsx: ['JX', '#f2d36b'],
    svelte: ['SV', '#ff8a5c'], rs: ['RS', '#e8a07a'], json: ['{}', '#d9c26b'], md: ['MD', '#9aa3b8'],
    css: ['CS', '#b48cff'], html: ['<>', '#ff8a8a'], py: ['PY', '#6bd0a0'], toml: ['TM', '#9aa3b8'],
    yml: ['YM', '#9aa3b8'], yaml: ['YM', '#9aa3b8'], sh: ['SH', '#6bd0a0'], lock: ['LK', '#667089'],
    png: ['IM', '#5ee0c1'], jpg: ['IM', '#5ee0c1'], svg: ['IM', '#5ee0c1'], gif: ['IM', '#5ee0c1'],
  }
  function icon(e: DirEntry): [string, string] {
    if (e.isDir) return ['▬', 'var(--accent)']
    const ext = e.name.includes('.') ? e.name.split('.').pop()!.toLowerCase() : ''
    return ICONS[ext] ?? ['··', 'var(--faint)']
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
    {@const [glyph, color] = icon(e)}
    <div
      class="row"
      class:ignored={e.ignored}
      class:active={editor.active === e.path}
      class:drop={dropTarget === e.path}
      style:padding-left="{8 + depth * 14}px"
      role="treeitem"
      aria-selected={editor.active === e.path}
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
      <span class="chev">{e.isDir ? (expanded[e.path] ? '▾' : '▸') : ''}</span>
      <span class="ic" style:color>{glyph}</span>
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
      <button role="menuitem" disabled={readOnly} onclick={() => startNew('file', menu!.entry.path)}>Nuovo file</button>
      <button role="menuitem" disabled={readOnly} onclick={() => startNew('dir', menu!.entry.path)}>Nuova cartella</button>
    {/if}
    <button role="menuitem" disabled={readOnly} onclick={() => startRename(menu!.entry)}>Rinomina</button>
    <button role="menuitem" disabled={readOnly} onclick={() => void trash(menu!.entry)}>Sposta nel Cestino</button>
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
  .chev { width: 10px; color: var(--faint); font-size: 10px; flex: none; }
  .ic { width: 18px; flex: none; font-size: 9px; font-weight: 700; text-align: center; letter-spacing: -0.02em; }
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
  .menu button:hover:not(:disabled) { background: var(--glass2); }
  .menu button:disabled { opacity: 0.4; cursor: default; }
</style>
