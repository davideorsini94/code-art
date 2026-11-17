<script lang="ts">
  import Icon from './Icon.svelte'
  import { tooltip } from '../lib/tooltip'

  /** Riga informativa generica: resta finché non si chiude. */
  let { message, onclose }: { message: string | null; onclose: () => void } = $props()
</script>

{#if message}
  <div class="toast" role="status" aria-live="polite">
    <span class="msg">{message}</span>
    <button class="x" aria-label="Chiudi il messaggio" use:tooltip={'Chiudi il messaggio'} onclick={onclose}><Icon name="x" size={14} /></button>
  </div>
{/if}

<style>
  .toast { position: fixed; right: 16px; bottom: 64px; z-index: 50; display: flex; align-items: flex-start; gap: 10px; max-width: 420px; padding: 8px 12px; border-radius: var(--radius); background: var(--surface-pop); border: 1px solid var(--edge); box-shadow: var(--panel-shadow); font-size: 12px; color: var(--txt); }
  .msg { flex: 1; min-width: 0; white-space: pre-wrap; overflow-wrap: anywhere; }
  .x { padding: 0; border: 0; background: none; color: var(--dim); font: inherit; cursor: pointer; }
  .x:hover { color: var(--txt); }
  @media (prefers-reduced-motion: no-preference) { .toast { animation: toast-in var(--dur) ease-out; } }
  @keyframes toast-in { from { opacity: 0; transform: translateY(6px); } }
  .x { display: inline-flex; align-items: center; justify-content: center; }
</style>
