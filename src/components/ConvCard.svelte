<script lang="ts">
  import { untrack } from 'svelte'
  import type { Conversation } from '../lib/types'
  import { relativeTime } from '../lib/conv/time'
  import { tooltip } from '../lib/tooltip'
  import { STATUS_TIP } from '../lib/conv/labels'

  type Props = {
    conv: Conversation
    active?: boolean
    now: number
    renaming?: boolean
    onselect?: () => void
    onstartrename?: () => void
    onrename?: (title: string) => void
    oncancelrename?: () => void
    onunlock?: () => void
    oncontextmenu?: (ev: MouseEvent) => void
  }
  let { conv, active = false, now, renaming = false, onselect, onstartrename, onrename, oncancelrename, onunlock, oncontextmenu }: Props = $props()

  const LABEL = { waiting: 'aspetta te', working: 'al lavoro', idle: '' } as const
  const label = $derived(LABEL[conv.status])
  const statusTip = $derived(STATUS_TIP[conv.status])

  let draft = $state('')
  let finished = false

  function commit() {
    if (finished) return
    finished = true
    const t = draft.trim()
    if (t && t !== conv.title) onrename?.(t)
    else oncancelrename?.()
  }
  function cancel() {
    finished = true
    oncancelrename?.()
  }
  // La bozza si inizializza solo alla comparsa del campo: gli upsert di `conv` durante la digitazione non la toccano.
  function focusSelect(node: HTMLInputElement) {
    untrack(() => {
      draft = conv.title
      finished = false
    })
    node.value = draft
    node.focus()
    node.select()
  }
</script>

<div
  class="conv"
  class:on={active}
  role="button"
  tabindex="0"
  use:tooltip={conv.title}
  aria-pressed={active}
  data-status={conv.status}
  onclick={() => onselect?.()}
  onkeydown={(e) => {
    if (e.target === e.currentTarget && (e.key === 'Enter' || e.key === ' ')) {
      e.preventDefault()
      onselect?.()
    }
  }}
  {oncontextmenu}
>
  <div class="t">
    <span class="dot {conv.status}" class:pulse={conv.status === 'working'} data-testid="dot" aria-hidden="true" use:tooltip={statusTip}></span>
    {#if renaming}
      <input
        class="rename"
        aria-label="Titolo della conversazione"
        maxlength="80"
        bind:value={draft}
        use:focusSelect
        onclick={(e) => e.stopPropagation()}
        onblur={commit}
        onkeydown={(e) => {
          e.stopPropagation()
          if (e.key === 'Enter') commit()
          else if (e.key === 'Escape') cancel()
        }}
      />
    {:else}
      <span class="title" role="presentation" data-no-tooltip="il tooltip della scheda (genitore) mostra già il titolo completo" ondblclick={(e) => { e.stopPropagation(); onstartrename?.() }}>{conv.title}</span>
    {/if}
    {#if conv.titleLocked}
      <button class="lock" use:tooltip={'Titolo bloccato: clic per sbloccare'} aria-label="Sblocca il titolo" onclick={(e) => { e.stopPropagation(); onunlock?.() }}>🔒</button>
    {/if}
    {#if conv.maybeOpenElsewhere}
      <span class="else" use:tooltip={'Forse aperta in un altro programma'} aria-label="Forse aperta in un altro programma">⧉</span>
    {/if}
  </div>
  {#if conv.prevTitle}
    <div class="prev" use:tooltip={`Titolo precedente: ${conv.prevTitle}`}>era: {conv.prevTitle}</div>
  {/if}
  {#if conv.summary}
    <div class="s" class:one={!!conv.prevTitle} use:tooltip={conv.summary}>{conv.summary}</div>
  {/if}
  <div class="m">
    {#if label}<span class="badge {conv.status}" use:tooltip={statusTip}>{label}</span>{:else}<span></span>{/if}
    <span class="time" use:tooltip={conv.lastActivity ? new Date(conv.lastActivity).toLocaleString('it-IT') : ''}>{relativeTime(conv.lastActivity, now)}</span>
  </div>
</div>

<style>
  .conv {
    padding: 10px 11px; border-radius: 12px; border: 1px solid transparent; cursor: default;
    height: 100%; box-sizing: border-box; overflow: hidden; outline: none;
    transition: background var(--dur), border-color var(--dur);
  }
  .conv:hover { background: var(--glass); }
  .conv:focus-visible { border-color: var(--accent); }
  .conv.on {
    background: var(--glass2); border-color: var(--edge);
    box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.2), 0 6px 18px rgba(0, 0, 0, 0.25);
  }
  .t { display: flex; gap: 8px; align-items: center; font-weight: 600; font-size: 13px; min-width: 0; }
  .title { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .dot { width: 8px; height: 8px; border-radius: 50%; flex: none; }
  .dot.working { background: var(--work); box-shadow: 0 0 10px var(--work); }
  .dot.waiting { background: var(--wait); box-shadow: 0 0 10px var(--wait); }
  .dot.idle { background: var(--done); }
  .dot.pulse { animation: p 1.6s ease-in-out infinite; }
  @keyframes p { 50% { opacity: 0.35; } }
  .rename {
    flex: 1; min-width: 0; font: inherit; color: var(--txt); background: var(--glass2);
    border: 1px solid var(--accent); border-radius: 6px; padding: 1px 5px; outline: none;
  }
  .lock, .else { flex: none; font-size: 10px; line-height: 1; opacity: 0.7; }
  .lock { background: none; border: 0; padding: 0; color: inherit; cursor: pointer; }
  .lock:hover { opacity: 1; }
  .prev { margin: 2px 0 0 16px; font-size: 10.5px; color: var(--faint); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .s {
    color: var(--dim); font-size: 11.5px; margin: 3px 0 0 16px; overflow: hidden;
    display: -webkit-box; -webkit-line-clamp: 2; line-clamp: 2; -webkit-box-orient: vertical;
  }
  .s.one { -webkit-line-clamp: 1; line-clamp: 1; }
  .m { color: var(--faint); font-size: 10.5px; margin: 4px 0 0 16px; display: flex; justify-content: space-between; align-items: center; }
  .badge { font-size: 10px; padding: 1px 7px; border-radius: 999px; border: 1px solid currentColor; }
  .badge.waiting { color: var(--wait); }
  .badge.working { color: var(--work); }
</style>
