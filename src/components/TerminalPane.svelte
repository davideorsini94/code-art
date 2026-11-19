<script lang="ts">
  import Icon from './Icon.svelte'
  import { onDestroy, onMount, tick, untrack } from 'svelte'
  import { listenProject } from '../lib/events'
  import { openExternal } from '../lib/external'
  import { projectBranch, ptyKill, ptyPickCwd, ptyResize, ptySpawn, ptyWrite } from '../lib/ipc'
  import { UNTRUSTED } from '../lib/errors'
  import { PtyInbox } from '../lib/pty-inbox'
  import { xtermTheme } from '../lib/theme'
  import { projectStore } from '../lib/stores/project.svelte'
  import { git } from '../lib/stores/git.svelte'
  import { editor } from '../lib/stores/editor.svelte'
  import { terminals } from '../lib/stores/terminals.svelte'
  import { relativeToFolders } from '../lib/conv/paths'
  import { folderLabel, moveTab, tabTitle } from '../lib/terminal/tabs'
  import { buildNewMenu, normalizeCustom, onlyPrimary, slotToIndex, type NewMenuItem } from '../lib/terminal/menu'
  import { tooltip } from '../lib/tooltip'

  type XTerm = import('@xterm/xterm').Terminal
  type Fit = import('@xterm/addon-fit').FitAddon
  type Mods = { Terminal: typeof import('@xterm/xterm').Terminal; FitAddon: typeof import('@xterm/addon-fit').FitAddon; WebLinksAddon: typeof import('@xterm/addon-web-links').WebLinksAddon }

  type Tab = { key: number; cwd: string; repo: string | null; openBranch: string | null; custom: string | null; shell: string; id: number | null; exited: boolean }
  type Live = { term: XTerm; fit: Fit; el: HTMLDivElement; id: number | null }

  let { visible }: { visible: boolean } = $props()

  let tabs = $state<Tab[]>([])
  let active = $state<number | null>(null)
  let host = $state<HTMLDivElement>()
  const live = new Map<number, Live>() // per chiave di scheda
  const byPty = new Map<number, number>() // id PTY -> chiave
  const inbox = new PtyInbox() // dati arrivati prima che spawn() restituisse l'id (scartati per i PTY chiusi)
  let mods: Mods | undefined
  let loading: Promise<Mods> | undefined
  let nextKey = 1
  let busy = 0 // schede in apertura: evita che la scheda di default si sommi a quelle richieste
  let chain: Promise<void> = Promise.resolve()
  let unlisten: (() => void)[] = []
  let listening: Promise<void> | undefined
  const pendingExit = new Set<number>() // pty:exit arrivato prima che spawn() restituisse l'id

  function decode(b64: string): Uint8Array {
    const bin = atob(b64)
    const out = new Uint8Array(bin.length)
    for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i)
    return out
  }

  function load(): Promise<Mods> {
    loading ??= Promise.all([import('@xterm/xterm'), import('@xterm/addon-fit'), import('@xterm/addon-web-links'), import('@xterm/xterm/css/xterm.css')]).then(
      ([x, f, w]) => (mods = { Terminal: x.Terminal, FitAddon: f.FitAddon, WebLinksAddon: w.WebLinksAddon }),
    )
    return loading
  }

  function listenOnce(): Promise<void> {
    listening ??= register()
    return listening
  }

  async function register() {
    const pid = projectStore.project?.id
    if (!pid) return
    unlisten = await Promise.all([
      listenProject<{ id: number; data: string }>('pty:data', pid, ({ payload }) => {
        const key = byPty.get(payload.id)
        const bytes = decode(payload.data)
        if (key === undefined) {
          inbox.hold(payload.id, bytes)
        } else {
          live.get(key)?.term.write(bytes)
        }
      }),
      listenProject<{ id: number }>('pty:exit', pid, ({ payload }) => {
        if (!byPty.has(payload.id)) pendingExit.add(payload.id)
        else markExited(payload.id)
      }),
    ])
  }

  function markExited(id: number) {
    const key = byPty.get(id)
    if (key === undefined) return
    byPty.delete(id)
    inbox.close(id)
    const l = live.get(key)
    if (l) l.id = null
    const t = tabs.find((x) => x.key === key)
    if (t) {
      t.exited = true
      t.id = null
    }
    l?.term.write('\r\n\x1b[2m[processo terminato]\x1b[0m\r\n')
  }

  const folders = $derived(projectStore.project?.folders ?? [])

  /** Il branch segue il repository (da git.status), con il nome letto all'apertura come ripiego. */
  const branchOf = (t: Tab) => (t.repo ? (git.status[t.repo]?.branch ?? t.openBranch) : t.openBranch)
  const titleOf = (t: Tab) => tabTitle({ cwd: t.cwd, branch: branchOf(t), custom: t.custom }, folders)
  const hint = (t: Tab) => `${relativeToFolders(t.cwd, folders)}${t.shell ? ` · ${t.shell}` : ''}${t.exited ? ' (terminato)' : ''}`

  /** Accoda l'apertura di schede, una dopo l'altra. */
  function enqueue(cwds: (string | undefined)[]) {
    busy++
    chain = chain
      .then(async () => {
        for (const c of cwds) await newTab(c)
      })
      .finally(() => busy--)
  }

  async function newTab(where?: string) {
    const cwd = where ?? projectStore.project?.folders[0]
    if (!cwd || !host) return
    void git.ensure().catch(() => {})
    const m = await load()
    await listenOnce()
    if (!host) return // il pannello è stato smontato durante il caricamento
    const key = nextKey++
    const el = document.createElement('div')
    el.className = 'term'
    host.appendChild(el)
    const term = new m.Terminal({
      theme: xtermTheme,
      allowTransparency: true,
      fontFamily: 'ui-monospace, Menlo, monospace',
      fontSize: 12,
      cursorBlink: true,
      scrollback: 5000,
      macOptionIsMeta: true,
    })
    const fit = new m.FitAddon()
    term.loadAddon(fit)
    term.loadAddon(
      // solo http(s), sempre nel browser di sistema: mai navigare la WebView
      new m.WebLinksAddon((_e, uri) => void openExternal(uri)),
    )
    // ⌃` è del pannello, non della shell
    term.attachCustomKeyEventHandler((e) => !(e.ctrlKey && !e.metaKey && (e.key === '`' || e.code === 'Backquote')))
    term.open(el)
    const l: Live = { term, fit, el, id: null }
    live.set(key, l)
    tabs.push({ key, cwd, repo: null, openBranch: null, custom: null, shell: '', id: null, exited: false })
    void projectBranch(cwd)
      .then((r) => {
        const t = tabs.find((x) => x.key === key)
        if (t && r) {
          t.repo = r.repo
          t.openBranch = r.name
        }
      })
      .catch(() => {})
    select(key)
    await tick()
    fit.fit()
    try {
      const { id, shell } = await ptySpawn(cwd, term.cols, term.rows)
      l.id = id
      const t = tabs.find((x) => x.key === key)
      if (t) {
        t.id = id
        t.shell = shell
      }
      byPty.set(id, key)
      for (const b of inbox.take(id)) term.write(b)
      if (pendingExit.delete(id)) markExited(id)
      term.onData((d) => void ptyWrite(id, d).catch(() => {}))
      term.onResize(({ cols, rows }) => void ptyResize(id, cols, rows).catch(() => {}))
    } catch (e) {
      // il messaggio del core (cartella inesistente o fuori dal progetto) si mostra così com'è
      term.write(
        String(e) === UNTRUSTED
          ? '\x1b[33mCartella non fidata: terminale disattivato\x1b[0m\r\n'
          : typeof e === 'string' && e
            ? `\x1b[31m${e}\x1b[0m\r\n`
            : '\x1b[31mImpossibile avviare la shell\x1b[0m\r\n',
      )
      const t = tabs.find((x) => x.key === key)
      if (t) t.exited = true
    }
  }

  function select(key: number) {
    active = key
    void tick().then(() => refit())
  }

  function refit() {
    const l = active === null ? undefined : live.get(active)
    if (!l || !visible || l.el.clientWidth === 0) return
    l.fit.fit()
    l.term.focus()
  }

  async function close(key: number) {
    const l = live.get(key)
    if (l?.id != null) {
      byPty.delete(l.id)
      inbox.close(l.id)
      void ptyKill(l.id).catch(() => {})
    }
    l?.term.dispose()
    l?.el.remove()
    live.delete(key)
    const i = tabs.findIndex((t) => t.key === key)
    tabs.splice(i, 1)
    if (tabs.length === 0) active = null
    else if (active === key) select(tabs[Math.min(i, tabs.length - 1)].key)
  }

  // ogni richiesta dello store apre una scheda nella sua cartella (prima della scheda di default)
  $effect(() => {
    if (!host || terminals.requests.length === 0) return
    const reqs = untrack(() => terminals.take())
    enqueue(reqs.map((r) => r.cwd))
  })

  // xterm si carica e la prima scheda parte alla prima apertura del pannello
  $effect(() => {
    if (visible && host && tabs.length === 0 && busy === 0 && terminals.requests.length === 0) enqueue([undefined])
    else if (visible) void tick().then(refit)
  })

  // le schede non attive restano montate ma nascoste
  $effect(() => {
    for (const [key, l] of live) l.el.style.display = key === active ? '' : 'none'
  })

  $effect(() => {
    if (!host) return
    const ro = new ResizeObserver(() => refit())
    ro.observe(host)
    return () => ro.disconnect()
  })

  // ---- rinomina ----
  let renaming = $state<{ key: number; text: string } | null>(null)

  function startRename(t: Tab) {
    renaming = { key: t.key, text: titleOf(t) }
  }

  function commitRename() {
    if (!renaming) return
    const t = tabs.find((x) => x.key === renaming!.key)
    if (t) t.custom = normalizeCustom(renaming.text)
    renaming = null
    void tick().then(refit)
  }

  function focusSelect(el: HTMLInputElement) {
    el.focus()
    el.select()
  }

  // ---- riordino ----
  let dragKey = $state<number | null>(null)
  let dropSlot = $state<number | null>(null)

  function dragOver(e: DragEvent, i: number) {
    if (dragKey === null) return
    e.preventDefault()
    if (e.dataTransfer) e.dataTransfer.dropEffect = 'move'
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect()
    dropSlot = i + (e.clientX > r.left + r.width / 2 ? 1 : 0)
  }

  function drop(e: DragEvent) {
    e.preventDefault()
    const from = tabs.findIndex((t) => t.key === dragKey)
    if (from >= 0 && dropSlot !== null) tabs = moveTab(tabs, from, slotToIndex(dropSlot, from))
    dragKey = dropSlot = null
  }

  // ---- menu ＋ ----
  let menu = $state<{ x: number; y: number; items: NewMenuItem[] } | null>(null)
  let pickError = $state<string | null>(null)
  const MENU_W = 240

  const activeFile = $derived(editor.activeTab && !editor.activeTab.diff ? editor.activeTab.path : null)
  const menuItems = () => buildNewMenu({ folders, repos: git.repos, activeFile })

  function plusClick(e: MouseEvent) {
    e.stopPropagation()
    pickError = null
    const items = menuItems()
    if (e.metaKey || onlyPrimary(items)) {
      menu = null
      enqueue([undefined])
      return
    }
    void git.ensure().catch(() => {})
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect()
    const h = items.length * 30 + 12
    const y = r.bottom + h > window.innerHeight ? Math.max(8, r.top - h) : r.bottom
    menu = { x: Math.max(8, Math.min(r.left, window.innerWidth - MENU_W - 8)), y, items }
  }

  async function choose(it: NewMenuItem) {
    menu = null
    if (it.kind !== 'other') return void enqueue([it.path])
    try {
      const picked = await ptyPickCwd()
      if (picked) enqueue([picked])
    } catch (e) {
      pickError = typeof e === 'string' && e ? e : 'Impossibile scegliere la cartella'
    }
  }

  function itemLabel(it: NewMenuItem): string {
    if (it.kind === 'other') return 'Altra cartella…'
    const name = folderLabel(it.path!, folders)
    if (it.kind === 'file') return `Cartella del file aperto · ${name}`
    return it.kind === 'folder' && it.path === folders[0] ? `${name} · principale` : name
  }

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

  onDestroy(() => {
    unlisten.forEach((u) => u())
    for (const l of live.values()) {
      if (l.id != null) void ptyKill(l.id).catch(() => {})
      l.term.dispose()
    }
  })
</script>

<div class="term-pane">
  <div class="tabs" role="tablist">
    {#each tabs as t, i (t.key)}
      <div class="tab" class:on={t.key === active} class:dead={t.exited} class:drop-before={dropSlot === i} class:drop-after={dropSlot === tabs.length && i === tabs.length - 1} class:dragging={dragKey === t.key}
        role="tab" use:tooltip={hint(t)} aria-selected={t.key === active} tabindex="0" draggable={renaming?.key !== t.key}
        onclick={() => select(t.key)} onkeydown={(e) => e.target === e.currentTarget && e.key === 'Enter' && select(t.key)}
        onmousedown={(e) => e.button === 1 && e.preventDefault()} onauxclick={(e) => { if (e.button === 1) { e.preventDefault(); void close(t.key) } }}
        ondblclick={() => startRename(t)}
        ondragstart={(e) => { dragKey = t.key; e.dataTransfer?.setData('application/x-codeart-tab', String(t.key)); if (e.dataTransfer) e.dataTransfer.effectAllowed = 'move' }}
        ondragover={(e) => dragOver(e, i)} ondrop={drop} ondragend={() => (dragKey = dropSlot = null)}>
        {#if renaming?.key === t.key}
          <input class="rename" aria-label="Nome del terminale" bind:value={renaming.text} use:focusSelect
            onclick={(e) => e.stopPropagation()} ondblclick={(e) => e.stopPropagation()}
            onkeydown={(e) => { e.stopPropagation(); if (e.key === 'Enter') commitRename(); else if (e.key === 'Escape') renaming = null }}
            onblur={commitRename} />
        {:else}
          <span>{titleOf(t)}</span>
        {/if}
        <button class="x" aria-label="Chiudi {titleOf(t)}" use:tooltip={'Chiudi terminale'} onclick={(e) => { e.stopPropagation(); void close(t.key) }}><Icon name="x" size={14} /></button>
      </div>
    {/each}
    <button class="plus" aria-label="Nuovo terminale" aria-haspopup="menu" use:tooltip={'Nuovo terminale (⌘ per la cartella principale)'} onclick={plusClick}><Icon name="plus" size={14} /></button>
  </div>
  {#if pickError}
    <div class="err" role="alert">{pickError}</div>
  {/if}
  <div class="host" bind:this={host}></div>
</div>

{#if menu}
  <div class="menu glass-strong" style:left="{menu.x}px" style:top="{menu.y}px" role="menu">
    {#each menu.items as it, i (i)}
      <button role="menuitem" use:tooltip={it.path ?? 'Scegli una cartella del progetto'} onclick={() => void choose(it)}>{itemLabel(it)}</button>
    {/each}
  </div>
{/if}

<style>
  .term-pane { display: flex; flex-direction: column; height: 100%; min-height: 0; border-top: 1px solid var(--edge-soft); }
  .tabs { display: flex; align-items: center; gap: 2px; padding: 4px 6px 0; flex: none; overflow-x: auto; }
  .tab {
    display: flex; align-items: center; gap: 6px; padding: 4px 8px 4px 12px; border-radius: 8px 8px 0 0;
    font-size: 12px; color: var(--dim); cursor: default; white-space: nowrap; transition: background var(--dur);
  }
  .tab:hover { background: var(--glass); }
  .tab.on { background: var(--glass2); color: var(--txt); }
  .tab.dead { color: var(--faint); }
  .x, .plus { background: none; border: 0; color: var(--faint); font-size: 14px; line-height: 1; padding: 2px 6px; border-radius: 4px; cursor: default; }
  .x { padding: 0 3px; }
  .x:hover, .plus:hover { background: var(--glass2); color: var(--txt); }
  .host { position: relative; flex: 1; min-height: 0; padding: 4px 8px 6px; }
  .host :global(.term) { position: absolute; inset: 4px 8px 6px; }
  .tab.dragging { opacity: 0.5; }
  .tab.drop-before { box-shadow: inset 2px 0 0 var(--accent); }
  .tab.drop-after { box-shadow: inset -2px 0 0 var(--accent); }
  .rename { width: 120px; height: 20px; box-sizing: border-box; background: var(--glass2); color: var(--txt); border: 1px solid var(--accent); border-radius: 5px; font: inherit; outline: none; padding: 0 4px; }
  .err { margin: 4px 10px; color: var(--wait); font-size: 12px; }
  .menu { position: fixed; z-index: 90; min-width: 200px; max-width: 320px; padding: 4px; display: flex; flex-direction: column; }
  .menu button { text-align: left; background: none; border: 0; color: var(--txt); padding: 6px 10px; border-radius: 6px; font-size: 12.5px; cursor: default; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .menu button:hover { background: var(--glass2); }
  .x, .plus { display: inline-flex; align-items: center; justify-content: center; }
</style>
