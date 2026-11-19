<script lang="ts">
  import { onMount } from 'svelte'
  import { tooltip } from '../../lib/tooltip'
  import { errorText } from '../../lib/errors'
  import { asFsError, sshList, sshMkdir, sshRename, sshWrite, sshDelete } from '../../lib/ipc'
  import { ssh } from '../../lib/stores/ssh.svelte'
  import { terminals } from '../../lib/stores/terminals.svelte'
  import { editor } from '../../lib/stores/editor.svelte'
  import { LOCAL_DRAG, REMOTE_DRAG } from '../../lib/drag'
  import FileIcon from '../FileIcon.svelte'
  import { iconFor } from '../../lib/fileicons'
  import type { RemoteEntry } from '../../lib/types'

  /** Albero di una cartella del server. `disabled` = connessione caduta: l'albero resta visibile, senza azioni. */
  let { serverId, root, disabled = false }: { serverId: string; root: string; disabled?: boolean } = $props()

  const DRAG = REMOTE_DRAG
  let children = $state<Record<string, RemoteEntry[]>>({})
  let truncated = $state<Record<string, boolean>>({})
  let expanded = $state<Record<string, boolean>>({})
  let selected = $state<string | null>(null)
  let menu = $state<{ x: number; y: number; entry: RemoteEntry } | null>(null)
  let editing = $state<{ kind: 'file' | 'dir' | 'rename'; parent?: string; path?: string; value: string } | null>(null)
  let dropTarget = $state<string | null>(null)
  let error = $state<string | null>(null)

  const parentOf = (p: string) => p.slice(0, p.lastIndexOf('/')) || '/'
  const nameOf = (p: string) => p.slice(p.lastIndexOf('/') + 1)
  const join = (dir: string, name: string) => (dir === '/' ? `/${name}` : `${dir}/${name}`)
  // i comandi dei file remoti rifiutano con `FsError` (`{ code, message }`), non con una stringa
  const failure = (e: unknown) => errorText(asFsError(e).message)
  const rootEntry = $derived<RemoteEntry>({ name: root, path: root, isDir: true, isLink: false })

  async function load(dir: string) {
    try {
      const l = await sshList(serverId, dir)
      children[dir] = l.entries
      truncated[dir] = !!l.truncated
    } catch (e) {
      delete children[dir]
      error = failure(e)
    }
  }

  async function toggle(e: RemoteEntry) {
    expanded[e.path] = !expanded[e.path]
    if (expanded[e.path] && !children[e.path]) await load(e.path)
  }

  $effect(() => {
    const r = root
    expanded[r] = true
    if (!children[r]) void load(r)
  })

  // trasferimento finito verso una cartella di questo server: si rilegge l'elenco (se era già stato caricato)
  $effect(() => {
    const r = ssh.reloadDir
    if (r && r.serverId === serverId && children[r.dir]) void load(r.dir)
  })

  onMount(() => {
    const close = () => (menu = null)
    const esc = (e: KeyboardEvent) => e.key === 'Escape' && (menu = null)
    window.addEventListener('click', close)
    window.addEventListener('keydown', esc)
    window.addEventListener('blur', close)
    return () => {
      window.removeEventListener('click', close)
      window.removeEventListener('keydown', esc)
      window.removeEventListener('blur', close)
    }
  })

  const MENU_W = 190
  const MENU_H = 220
  const menuAt = (x: number, y: number) => ({
    x: Math.max(4, Math.min(x, window.innerWidth - MENU_W - 4)),
    y: Math.max(4, Math.min(y, window.innerHeight - MENU_H - 4)),
  })

  async function run(op: () => Promise<unknown>): Promise<boolean> {
    error = null
    try {
      await op()
      return true
    } catch (e) {
      error = failure(e)
      return false
    }
  }

  const reloadAll = () => Promise.all(Object.keys(children).map(load))

  function startNew(kind: 'file' | 'dir', dir: string) {
    menu = null
    expanded[dir] = true
    if (!children[dir]) void load(dir)
    editing = { kind, parent: dir, value: '' }
  }

  function startRename(e: RemoteEntry) {
    menu = null
    editing = { kind: 'rename', path: e.path, value: e.name }
  }

  async function commit() {
    const ed = editing
    editing = null
    const value = ed?.value.trim()
    if (!ed || !value || value.includes('/')) return
    if (ed.kind === 'rename' && ed.path) {
      const from = ed.path
      const to = join(parentOf(from), value)
      if (to !== from && (await run(() => sshRename(serverId, from, to)))) editor.repathRemote(serverId, from, to)
    } else if (ed.parent) {
      const path = join(ed.parent, value)
      await run(() => (ed.kind === 'dir' ? sshMkdir(serverId, path) : sshWrite(serverId, path, '', '')))
    }
    await reloadAll()
  }

  async function remove(e: RemoteEntry) {
    menu = null
    // la conferma è nativa, del core: annullata = errore, e va bene così
    if (await run(() => sshDelete(serverId, e.path))) {
      selected = null
      editor.markRemoteRemoved(serverId, e.path)
    }
    await reloadAll()
  }

  function onDrop(ev: DragEvent, target: RemoteEntry) {
    ev.preventDefault()
    dropTarget = null
    if (disabled || !target.isDir) return
    // dal progetto: upload nella cartella di destinazione
    const local = ev.dataTransfer?.getData(LOCAL_DRAG)
    if (local) {
      try {
        const paths = JSON.parse(local)
        if (Array.isArray(paths) && paths.length && paths.every((p) => typeof p === 'string')) void ssh.upload(serverId, paths, target.path)
      } catch {
        /* dati non validi: niente */
      }
      return
    }
    let from: string | null = null
    try {
      const d = JSON.parse(ev.dataTransfer?.getData(DRAG) || 'null')
      if (d && d.serverId === serverId && typeof d.path === 'string') from = d.path
    } catch {
      /* non è un trascinamento di questo albero */
    }
    if (!from || from === root || from === target.path || target.path.startsWith(from + '/') || parentOf(from) === target.path) return
    const to = join(target.path, nameOf(from))
    void run(async () => {
      await sshRename(serverId, from!, to)
      editor.repathRemote(serverId, from!, to)
      expanded[target.path] = true
      await reloadAll()
    })
  }
</script>

{#snippet inputRow(depth: number)}
  <!-- svelte-ignore a11y_autofocus -->
  <input
    class="edit"
    aria-label="Nome"
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

{#snippet node(e: RemoteEntry, depth: number)}
  {#if editing?.kind === 'rename' && editing.path === e.path}
    {@render inputRow(depth)}
  {:else}
    {@const spec = iconFor(e.name, e.isDir, !!expanded[e.path])}
    {@const isRoot = depth === 0}
    <div
      class="row"
      class:active={selected === e.path}
      class:drop={dropTarget === e.path}
      style:padding-left="{8 + depth * 14}px"
      role="treeitem"
      use:tooltip={e.isLink ? `${e.path} (collegamento)` : e.path}
      aria-selected={selected === e.path}
      aria-expanded={e.isDir ? !!expanded[e.path] : undefined}
      tabindex="0"
      draggable={isRoot || disabled ? 'false' : 'true'}
      ondragstart={(ev) => {
        if (isRoot || disabled) return ev.preventDefault()
        ev.dataTransfer?.setData(DRAG, JSON.stringify({ serverId, path: e.path }))
      }}
      ondragover={(ev) => {
        if (e.isDir && !disabled) {
          ev.preventDefault()
          dropTarget = e.path
        }
      }}
      ondragleave={() => (dropTarget = null)}
      ondrop={(ev) => onDrop(ev, e)}
      onclick={() => {
        selected = e.path
        if (e.isDir) void toggle(e)
      }}
      ondblclick={() => {
        if (!e.isDir && !disabled) void editor.openRemote(serverId, e.path)
      }}
      onkeydown={(ev) => {
        if (ev.key !== 'Enter') return
        selected = e.path
        if (e.isDir) void toggle(e)
        else if (!disabled) void editor.openRemote(serverId, e.path)
      }}
      oncontextmenu={(ev) => {
        ev.preventDefault()
        ev.stopPropagation()
        selected = e.path
        if (!disabled) menu = { ...menuAt(ev.clientX, ev.clientY), entry: e }
      }}
    >
      <span class="chev" class:open={e.isDir && expanded[e.path]} aria-hidden="true">
        {#if e.isDir}
          <svg width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M3.500 2 6.500 5 3.500 8" /></svg>
        {/if}
      </span>
      <span class="ic" class:loading={e.isDir && expanded[e.path] && !children[e.path]}>
        <FileIcon id={spec.id} dim={spec.dim} size={16} root={isRoot} />
      </span>
      <span class="name">{e.name}</span>
    </div>
    {#if e.isDir && expanded[e.path]}
      {#if editing && editing.kind !== 'rename' && editing.parent === e.path}
        {@render inputRow(depth + 1)}
      {/if}
      {#each children[e.path] ?? [] as c, i (`${i}:${c.path}`)}
        {@render node(c, depth + 1)}
      {/each}
      {#if truncated[e.path]}<p class="more" style:padding-left="{8 + (depth + 1) * 14}px">Elenco troncato (oltre 10 000 voci)</p>{/if}
    {/if}
  {/if}
{/snippet}

<div class="tree" class:off={disabled} role="tree" aria-label="File sul server">
  {@render node(rootEntry, 0)}
  {#if error}<p class="err">{error}</p>{/if}
</div>

{#if menu}
  <div class="menu glass-strong" style:left="{menu.x}px" style:top="{menu.y}px" role="menu">
    {#if menu.entry.isDir}
      <button role="menuitem" use:tooltip={'Crea un nuovo file sul server in questa cartella'} onclick={() => startNew('file', menu!.entry.path)}>Nuovo file</button>
      <button role="menuitem" use:tooltip={'Crea una nuova cartella sul server qui dentro'} onclick={() => startNew('dir', menu!.entry.path)}>Nuova cartella</button>
    {/if}
    {#if menu.entry.path !== root}
      <button role="menuitem" use:tooltip={'Cambia il nome'} onclick={() => startRename(menu!.entry)}>Rinomina</button>
      <button role="menuitem" use:tooltip={'Elimina sul server (chiede conferma)'} onclick={() => void remove(menu!.entry)}>Elimina</button>
    {/if}
    <button role="menuitem" use:tooltip={'Apre un terminale sul server in questa cartella'} onclick={() => { const e = menu!.entry; menu = null; terminals.openRemote(serverId, e.isDir ? e.path : parentOf(e.path)) }}>Apri terminale qui</button>
    <button role="menuitem" use:tooltip={'Copia nella prima cartella del progetto'} onclick={() => { const p = menu!.entry.path; menu = null; void ssh.download(serverId, p) }}>Scarica nel progetto…</button>
  </div>
{/if}

<style>
  .tree { overflow: auto; font-size: 12.5px; user-select: none; }
  .tree.off { opacity: 0.45; }
  .row {
    display: flex; align-items: center; gap: 5px; height: 24px; padding-right: 8px;
    border-radius: 6px; cursor: default; color: var(--txt); white-space: nowrap;
    transition: background var(--dur);
  }
  .row:hover { background: var(--glass); }
  .row.active { background: var(--glass2); }
  .row.drop { outline: 1px solid var(--accent); }
  .chev { width: 10px; height: 10px; color: var(--faint); flex: none; display: flex; }
  .chev svg { transition: transform 170ms ease; }
  .chev.open svg { transform: rotate(90deg); }
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
  .err { margin: 8px; color: var(--wait); font-size: 12px; overflow-wrap: anywhere; }
  .more { margin: 2px 0; color: var(--faint); font-size: 11.5px; }
  .menu { position: fixed; z-index: 90; min-width: 180px; padding: 4px; display: flex; flex-direction: column; }
  .menu button {
    text-align: left; background: none; border: 0; color: var(--txt); padding: 6px 10px;
    border-radius: 6px; font-size: 12.5px; cursor: default;
  }
  .menu button:hover { background: var(--glass2); }
</style>
