<script lang="ts">
  import { onDestroy, tick } from 'svelte'
  import { listenProject } from '../lib/events'
  import { openExternal } from '../lib/external'
  import { ptyKill, ptyResize, ptySpawn, ptyWrite } from '../lib/ipc'
  import { UNTRUSTED } from '../lib/errors'
  import { PtyInbox } from '../lib/pty-inbox'
  import { xtermTheme } from '../lib/theme'
  import { projectStore } from '../lib/stores/project.svelte'
  import { tooltip } from '../lib/tooltip'

  type XTerm = import('@xterm/xterm').Terminal
  type Fit = import('@xterm/addon-fit').FitAddon
  type Mods = { Terminal: typeof import('@xterm/xterm').Terminal; FitAddon: typeof import('@xterm/addon-fit').FitAddon; WebLinksAddon: typeof import('@xterm/addon-web-links').WebLinksAddon }

  type Tab = { key: number; title: string; id: number | null; exited: boolean }
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
  let starting = false
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

  async function newTab() {
    const cwd = projectStore.project?.folders[0]
    if (!cwd || !host) return
    const m = await load()
    await listenOnce()
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
    tabs.push({ key, title: `terminale ${key}`, id: null, exited: false })
    select(key)
    await tick()
    fit.fit()
    try {
      const { id, shell } = await ptySpawn(cwd, term.cols, term.rows)
      l.id = id
      const t = tabs.find((x) => x.key === key)
      if (t) {
        t.id = id
        t.title = `${shell} ${key}`
      }
      byPty.set(id, key)
      for (const b of inbox.take(id)) term.write(b)
      if (pendingExit.delete(id)) markExited(id)
      term.onData((d) => void ptyWrite(id, d).catch(() => {}))
      term.onResize(({ cols, rows }) => void ptyResize(id, cols, rows).catch(() => {}))
    } catch (e) {
      term.write(
        String(e) === UNTRUSTED
          ? '\x1b[33mCartella non fidata: terminale disattivato\x1b[0m\r\n'
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

  // xterm si carica e la prima scheda parte alla prima apertura del pannello
  $effect(() => {
    if (visible && host && tabs.length === 0 && !starting) {
      starting = true
      void newTab().finally(() => (starting = false))
    } else if (visible) void tick().then(refit)
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
    {#each tabs as t (t.key)}
      <div class="tab" class:on={t.key === active} class:dead={t.exited} role="tab" use:tooltip={t.exited ? `${t.title} (terminato)` : t.title} aria-selected={t.key === active} tabindex="0"
        onclick={() => select(t.key)} onkeydown={(e) => e.key === 'Enter' && select(t.key)}>
        <span>{t.title}</span>
        <button class="x" aria-label="Chiudi {t.title}" use:tooltip={'Chiudi terminale'} onclick={(e) => { e.stopPropagation(); void close(t.key) }}>×</button>
      </div>
    {/each}
    <button class="plus" aria-label="Nuovo terminale" use:tooltip={'Nuovo terminale'} onclick={() => void newTab()}>+</button>
  </div>
  <div class="host" bind:this={host}></div>
</div>

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
</style>
