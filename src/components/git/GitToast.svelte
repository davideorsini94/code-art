<script lang="ts">
  import { tooltip } from '../../lib/tooltip'
  import { git, REMOTE_LABEL } from '../../lib/stores/git.svelte'

  /** Il pannello Git è in vista: mostra già errori ed esiti, il messaggio qui sarebbe doppio. */
  let { panelVisible = false }: { panelVisible?: boolean } = $props()

  const NOTICE_MS = 6000
  const remote = $derived(git.busy && git.busy.op in REMOTE_LABEL ? git.busy : null)
  const show = $derived(!panelVisible && (git.error !== null || git.notice !== null || remote !== null))

  // gli esiti positivi scompaiono da soli; gli errori restano finché non si chiudono (o finché un'operazione riesce)
  $effect(() => {
    const n = git.notice
    if (!n) return
    const t = setTimeout(() => {
      if (git.notice === n) git.notice = null
    }, NOTICE_MS)
    return () => clearTimeout(t)
  })
</script>

{#if show}
  <div class="toast" role="status" aria-live="polite">
    {#if remote}
      <span class="msg">{REMOTE_LABEL[remote.op as keyof typeof REMOTE_LABEL]}{remote.line ? ` · ${remote.line}` : '…'}</span>
      <button class="link" use:tooltip={'Interrompe l’operazione'} onclick={() => void git.cancel()}>Annulla</button>
    {:else if git.error}
      <span class="msg err">{git.error}</span>
      <button class="x" aria-label="Chiudi il messaggio" use:tooltip={'Chiudi il messaggio'} onclick={() => (git.error = null)}>×</button>
    {:else if git.notice}
      <span class="msg">{git.notice}</span>
      <button class="x" aria-label="Chiudi il messaggio" use:tooltip={'Chiudi il messaggio'} onclick={() => (git.notice = null)}>×</button>
    {/if}
  </div>
{/if}

<style>
  .toast { position: fixed; right: 16px; bottom: 16px; z-index: 50; display: flex; align-items: flex-start; gap: 10px; max-width: 420px; padding: 8px 12px; border-radius: var(--radius); background: var(--surface-pop); border: 1px solid var(--edge); box-shadow: var(--panel-shadow); font-size: 12px; color: var(--txt); }
  .msg { flex: 1; min-width: 0; white-space: pre-wrap; overflow-wrap: anywhere; }
  .err { color: var(--err); }
  .link, .x { padding: 0; border: 0; background: none; color: var(--dim); font: inherit; cursor: pointer; }
  .link { color: var(--accent); text-decoration: underline; }
  .x:hover { color: var(--txt); }
  @media (prefers-reduced-motion: no-preference) { .toast { animation: toast-in var(--dur) ease-out; } }
  @keyframes toast-in { from { opacity: 0; transform: translateY(6px); } }
</style>
