<script lang="ts">
  import type { ChatItem } from '../lib/types'
  import { chat } from '../lib/stores/chat.svelte'
  import { renderMarkdown } from '../lib/safe-markdown'
  import { links } from '../lib/conv/links'

  type Ask = Extract<ChatItem, { kind: 'ask' }>
  let { item, sessionId }: { item: Ask; sessionId: string } = $props()

  let busy = $state(false)
  let error = $state<string | null>(null)
  let free = $state('')

  async function answer(choice: string) {
    if (busy || item.answered || !choice.trim()) return
    busy = true
    error = null
    try {
      await chat.answer(sessionId, item.requestId, choice)
    } catch (e) {
      error = String(e)
    } finally {
      busy = false
    }
  }

  const title = $derived(item.askType === 'plan' ? 'Claude propone un piano' : item.askType === 'question' ? 'Come procedo?' : 'Claude chiede il permesso')
</script>

<div class="ask" class:done={item.answered} role="group" aria-label="Claude ha bisogno di te">
  <b>{title}</b>
  {#if item.answered}<span class="tag">risposto</span>{:else}<div class="sub">Serve una tua scelta per continuare.</div>{/if}

  {#if item.askType === 'plan'}
    <div class="md plan" use:links>{@html renderMarkdown(item.prompt)}</div>
  {:else}
    <p class="prompt">{item.prompt}</p>
  {/if}

  {#if !item.answered}
    <div class="btns">
      {#if item.askType === 'permission'}
        <button class="btn p" disabled={busy} onclick={() => answer('allow')}>Consenti</button>
        <button class="btn" disabled={busy} onclick={() => answer('deny')}>Nega</button>
      {:else if item.askType === 'plan'}
        <button class="btn p" disabled={busy} onclick={() => answer('approve')}>Approva piano</button>
        <button class="btn" disabled={busy} onclick={() => answer('reject')}>Continua a pianificare</button>
      {:else}
        {#each item.options as opt, i (i)}
          <button class="btn" class:p={i === 0} disabled={busy} onclick={() => answer(opt)}>{opt}</button>
        {/each}
      {/if}
    </div>
    {#if item.askType === 'question'}
      <form class="free" onsubmit={(e) => { e.preventDefault(); void answer(free) }}>
        <input type="text" placeholder="Oppure scrivi una risposta…" bind:value={free} disabled={busy} aria-label="Risposta libera" />
        <button class="btn" type="submit" disabled={busy || !free.trim()}>Invia</button>
      </form>
    {/if}
  {/if}
  {#if error}<p class="err" role="alert">{error}</p>{/if}
</div>

<style>
  .ask { border: 1px solid rgba(255, 180, 84, 0.4); background: rgba(255, 180, 84, 0.09); border-radius: 13px; padding: 10px 12px; }
  .ask.done { opacity: 0.6; border-color: var(--edge-soft); background: var(--glass); }
  .sub { color: var(--dim); }
  .tag { color: var(--faint); font-size: 11px; margin-left: 8px; }
  .prompt { margin: 6px 0 0; white-space: pre-wrap; word-break: break-word; color: var(--txt); }
  .plan { margin-top: 6px; max-height: 320px; overflow: auto; }
  .btns { display: flex; flex-wrap: wrap; gap: 6px; margin-top: 8px; }
  .btn { padding: 4px 12px; border-radius: 999px; background: var(--glass2); border: 1px solid var(--edge); color: inherit; font-size: 11.5px; cursor: pointer; }
  .btn.p { background: rgba(255, 180, 84, 0.25); border-color: rgba(255, 180, 84, 0.5); }
  .btn:disabled { opacity: 0.5; cursor: default; }
  .free { display: flex; gap: 6px; margin-top: 8px; }
  .free input { flex: 1; min-width: 0; padding: 4px 10px; border-radius: 999px; border: 1px solid var(--edge-soft); background: rgba(0, 0, 0, 0.28); color: inherit; font: inherit; font-size: 12px; }
  .err { color: #ff8f8f; font-size: 11.5px; margin: 6px 0 0; }
  @media (prefers-reduced-motion: no-preference) { .ask { transition: opacity var(--dur); } }
</style>
