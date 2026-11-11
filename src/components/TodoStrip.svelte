<script lang="ts">
  import { tooltip } from '../lib/tooltip'
  import type { TodoItem } from '../lib/conv/reduce'

  let { todos }: { todos: TodoItem[] } = $props()
  let open = $state(false)

  const done = $derived(todos.filter((t) => t.status === 'completed').length)
  const current = $derived(todos.find((t) => t.status === 'in_progress') ?? todos.find((t) => t.status === 'pending'))
  const MARK = { pending: '○', in_progress: '◐', completed: '●' } as const
  const label = (t: TodoItem) => t.text || `Attività ${t.id}`
</script>

{#if todos.length > 0}
  <div class="strip" aria-label="Attività">
    <button class="head" use:tooltip={`${open ? 'Comprimi' : 'Espandi'} le attività (${done}/${todos.length}): ${current ? label(current) : 'tutto completato'}`} aria-expanded={open} onclick={() => (open = !open)}>
      <span class="count">{done}/{todos.length}</span>
      <span class="cur">{current ? label(current) : 'Tutto completato'}</span>
      <span class="caret">{open ? '▾' : '▸'}</span>
    </button>
    {#if open}
      <ul>
        {#each todos as t (t.id)}
          <li class={t.status}><span class="mk" aria-hidden="true">{MARK[t.status]}</span>{label(t)}</li>
        {/each}
      </ul>
    {/if}
  </div>
{/if}

<style>
  .strip { border: 1px solid var(--edge-soft); background: var(--well); border-radius: 11px; padding: 4px 10px; }
  .head { display: flex; width: 100%; gap: 8px; align-items: center; background: none; border: 0; color: var(--dim); font: inherit; font-size: 11.5px; cursor: pointer; padding: 3px 0; text-align: left; }
  .count { color: var(--work); font-variant-numeric: tabular-nums; }
  .cur { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  ul { list-style: none; padding: 2px 0 4px; max-height: 140px; overflow: auto; }
  li { font-size: 11.5px; color: var(--dim); padding: 1px 0; }
  li.in_progress { color: var(--txt); }
  li.completed { color: var(--faint); text-decoration: line-through; }
  .mk { display: inline-block; width: 16px; color: var(--work); }
</style>
