<script lang="ts">
  import Icon from './Icon.svelte'
  import { tick } from 'svelte'
  import { conversations } from '../lib/stores/conversations.svelte'
  import { projectStore } from '../lib/stores/project.svelte'
  import { STATUS_TIP, UNTRUSTED_HINT, convTitle } from '../lib/conv/labels'
  import { tooltip } from '../lib/tooltip'

  const untrusted = $derived(projectStore.untrusted.length > 0)
  let bar = $state<HTMLDivElement>()

  // la scheda attiva resta in vista anche quando le schede non ci stanno
  $effect(() => {
    const id = conversations.activeId
    if (!id || !bar) return
    void tick().then(() => {
      const el = bar?.querySelector<HTMLElement>(`[data-id="${CSS.escape(id)}"]`)
      el?.scrollIntoView?.({ block: 'nearest', inline: 'nearest' })
    })
  })
</script>

<div class="tabs" role="tablist" aria-label="Conversazioni aperte" bind:this={bar}>
  {#each conversations.openConvs as c (c.sessionId)}
    {@const on = c.sessionId === conversations.activeId}
    <div
      class="tab"
      class:on
      data-id={c.sessionId}
      role="tab"
      aria-selected={on}
      tabindex="0"
      use:tooltip={convTitle(c.title)}
      onclick={() => void conversations.select(c.sessionId)}
      onkeydown={(e) => e.key === 'Enter' && void conversations.select(c.sessionId)}
      onauxclick={(e) => {
        if (e.button === 1) {
          e.preventDefault()
          conversations.close(c.sessionId)
        }
      }}
    >
      <span class="dot {c.status}" class:pulse={c.status === 'working'} role="img" aria-label={STATUS_TIP[c.status]} use:tooltip={STATUS_TIP[c.status]}></span>
      <span class="name">{convTitle(c.title)}</span>
      <button
        class="x"
        aria-label="Chiudi {convTitle(c.title)}"
        use:tooltip={'Chiudi la scheda (la conversazione resta nell\'elenco)'}
        onclick={(e) => {
          e.stopPropagation()
          conversations.close(c.sessionId)
        }}><Icon name="x" size={14} /></button
      >
    </div>
  {/each}
  <button class="new" aria-label="Nuova conversazione" aria-disabled={untrusted} use:tooltip={untrusted ? UNTRUSTED_HINT : 'Nuova conversazione'} onclick={() => !untrusted && void conversations.create()}><Icon name="plus" size={14} /></button>
</div>

<style>
  .tabs {
    display: flex; align-items: flex-end; gap: 2px; overflow-x: auto; flex: none; scrollbar-width: none;
    margin: -12px -12px 10px; padding: 6px 6px 0; border-bottom: 1px solid var(--edge-soft);
  }
  .tabs::-webkit-scrollbar { display: none; }
  .tab {
    display: flex; align-items: center; gap: 7px; padding: 6px 6px 6px 10px; border-radius: 8px 8px 0 0;
    font-size: 12px; color: var(--dim); cursor: default; white-space: nowrap; flex: none; max-width: 190px;
    transition: background var(--dur);
  }
  .tab:hover { background: var(--glass); }
  .tab.on { background: var(--glass2); color: var(--txt); box-shadow: inset 0 1px 0 var(--hi); }
  .name { min-width: 0; overflow: hidden; text-overflow: ellipsis; }
  .dot { width: 7px; height: 7px; border-radius: 50%; flex: none; background: var(--done); }
  .dot.working { background: var(--work); box-shadow: 0 0 8px var(--work); }
  .dot.waiting { background: var(--wait); box-shadow: 0 0 8px var(--wait); }
  .dot.pulse { animation: p 1.6s ease-in-out infinite; }
  @keyframes p { 50% { opacity: 0.35; } }
  .x {
    background: none; border: 0; color: var(--faint); font-size: 14px; line-height: 1; padding: 0 3px; border-radius: 4px;
    cursor: default; visibility: hidden; flex: none;
  }
  .tab:hover .x, .tab.on .x, .x:focus-visible { visibility: visible; }
  .x:hover { background: var(--glass2); color: var(--txt); }
  .new {
    background: none; border: 0; color: var(--faint); font-size: 13px; padding: 4px 8px; margin-bottom: 2px;
    border-radius: 6px; cursor: default; flex: none;
  }
  .new:hover { color: var(--accent); background: var(--glass); }
  .new[aria-disabled='true'] { opacity: 0.45; }
  .x, .new { display: inline-flex; align-items: center; justify-content: center; }
</style>
