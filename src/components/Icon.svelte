<script lang="ts" module>
  /** Icone dell'interfaccia: tratti su griglia 24×24, linee arrotondate, nessuna dipendenza esterna.
   *  Ogni icona è un elenco di tracciati SVG (`d`), disegnati senza `{@html}`. */

  /** Cerchio come tracciato. */
  const c = (cx: number, cy: number, r: number) => `M${cx - r} ${cy}a${r} ${r} 0 1 0 ${2 * r} 0a${r} ${r} 0 1 0 ${-2 * r} 0`
  /** Rettangolo con angoli arrotondati come tracciato. */
  const r = (x: number, y: number, w: number, h: number, k: number) =>
    `M${x + k} ${y}h${w - 2 * k}a${k} ${k} 0 0 1 ${k} ${k}v${h - 2 * k}a${k} ${k} 0 0 1 ${-k} ${k}h${-(w - 2 * k)}a${k} ${k} 0 0 1 ${-k} ${-k}v${-(h - 2 * k)}a${k} ${k} 0 0 1 ${k} ${-k}z`

  type Shape = string | { d: string; fill: true }

  export const ICONS = {
    // barra laterale
    files: ['M4 5.5A1.5 1.5 0 0 1 5.5 4H10l2 2.5h6.5A1.5 1.5 0 0 1 20 8v10.5a1.5 1.5 0 0 1-1.5 1.5h-13A1.5 1.5 0 0 1 4 18.5z', 'M4 10h16'],
    search: [c(11, 11, 6.5), 'm20 20-4.2-4.2'],
    branch: [c(6, 5, 2), c(6, 19, 2), c(18, 7, 2), 'M6 7v10', 'M18 9c0 5-6 4-11.2 8.6'],
    chats: [
      'M14 9a2 2 0 0 0-2-2H5a2 2 0 0 0-2 2v6a2 2 0 0 0 2 2h1v3l3.5-3H12a2 2 0 0 0 2-2z',
      'M10 7V5a2 2 0 0 1 2-2h7a2 2 0 0 1 2 2v6a2 2 0 0 1-2 2h-1v3l-3-2.5',
    ],
    chat: ['M20 15a2 2 0 0 1-2 2H8l-4 3.5V6a2 2 0 0 1 2-2h12a2 2 0 0 1 2 2z', 'M8 9h8', 'M8 12.5h5'],
    settings: ['M20 7h-9', 'M14 17H4', 'M4 7h3', 'M20 17h-3', c(9, 7, 2.5), c(15, 17, 2.5)],
    terminal: [r(3, 4, 18, 16, 2), 'm7 9 3 3-3 3', 'M13 15h4'],
    // azioni
    x: ['M6 6l12 12M18 6 6 18'],
    plus: ['M12 5v14M5 12h14'],
    minus: ['M5 12h14'],
    undo: ['M9 14 4 9l5-5', 'M4 9h10.5a5.5 5.5 0 0 1 0 11H11'],
    refresh: ['M20 11a8 8 0 0 0-14.3-4.9L4 8', 'M4 3v5h5', 'M4 13a8 8 0 0 0 14.3 4.9L20 16', 'M20 21v-5h-5'],
    open: ['M14 4h6v6', 'M20 4 11 13', 'M18 14v5a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V7a1 1 0 0 1 1-1h5'],
    zoomIn: [c(11, 11, 6.5), 'm20 20-4.2-4.2', 'M11 8.5v5M8.5 11h5'],
    zoomOut: [c(11, 11, 6.5), 'm20 20-4.2-4.2', 'M8.5 11h5'],
    // direzioni
    chevronDown: ['m6 9 6 6 6-6'],
    chevronRight: ['m9 6 6 6-6 6'],
    chevronUp: ['m6 15 6-6 6 6'],
    arrowUp: ['M12 19V5', 'm6 11 6-6 6 6'],
    arrowDown: ['M12 5v14', 'm6 13 6 6 6-6'],
    // stati
    check: ['m5 12.5 4.5 4.5L19 7.5'],
    box: [r(4, 4, 16, 16, 3)],
    boxCheck: [r(4, 4, 16, 16, 3), 'm8 12 3 3 5-6'],
    circle: [c(12, 12, 8)],
    circleHalf: [c(12, 12, 8), { d: 'M12 4a8 8 0 0 1 0 16z', fill: true }],
    circleCheck: [c(12, 12, 8), 'm8.5 12 2.5 2.5 4.5-5'],
    loader: ['M12 3a9 9 0 1 0 9 9'],
    elsewhere: [r(3, 5, 14, 11, 1.5), 'M7 19h12a2 2 0 0 0 2-2V9'],
  } satisfies Record<string, Shape[]>

  export type IconName = keyof typeof ICONS
</script>

<script lang="ts">
  let { name, size = 16, stroke = 1.75, label }: { name: IconName; size?: number; stroke?: number; label?: string } = $props()
  const shapes = $derived(ICONS[name] as readonly Shape[])
</script>

<svg
  class="icon"
  width={size}
  height={size}
  viewBox="0 0 24 24"
  fill="none"
  stroke="currentColor"
  stroke-width={stroke}
  stroke-linecap="round"
  stroke-linejoin="round"
  role={label ? 'img' : undefined}
  aria-label={label}
  aria-hidden={label ? undefined : 'true'}
  focusable="false"
>
  {#each shapes as s, i (i)}
    {#if typeof s === 'string'}<path d={s} />{:else}<path d={s.d} fill="currentColor" stroke="none" />{/if}
  {/each}
</svg>

<style>
  .icon { display: block; flex: none; }
</style>
