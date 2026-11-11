<script lang="ts">
  // Icona decorativa: un solo <svg>, forme statiche scelte dall'id (mai markup da dati esterni).
  let { id, color = 'var(--dim)', size = 16, root = false }: { id: string; color?: string; size?: number; root?: boolean } = $props()

  const isFolder = $derived(id.startsWith('folder'))
  const open = $derived(id.endsWith('-open'))
  const badge = $derived(isFolder ? id.replace(/-open$/, '').slice(7) : '')

  // glifi dei file: tracciati 16x16 disegnati con tratto scuro dentro una tessera colorata
  const GLYPHS: Record<string, string> = {
    ts: 'M4.6 6.2h4M6.6 6.2v5M10 6.6c.5-.5 1.9-.6 2.2.2.3.9-2.4 1.1-2.2 2.1.2 1 1.8.9 2.3.3',
    js: 'M7 5.8v4.2c0 1-.4 1.4-1.2 1.4M10 6.6c.5-.5 1.9-.6 2.2.2.3.9-2.4 1.1-2.2 2.1.2 1 1.8.9 2.3.3',
    json: 'M6.4 4.8c-1 0-1.2.6-1.2 1.4v1.1c0 .5-.3.7-.7.7.4 0 .7.2.7.7v1.1c0 .8.2 1.4 1.2 1.4M9.6 4.8c1 0 1.2.6 1.2 1.4v1.1c0 .5.3.7.7.7-.4 0-.7.2-.7.7v1.1c0 .8-.2 1.4-1.2 1.4',
    html: 'M6.2 5.8 3.8 8l2.4 2.2M9.8 5.8 12.2 8l-2.4 2.2M8.8 4.8l-1.6 6.4',
    css: 'M6.3 4.8 5.7 11.2M10.3 4.8 9.7 11.2M4.4 6.8h7.4M4.2 9.2h7.4',
    svelte: 'M10.6 5.6c-1-1-3.4-.8-4 .6-.5 1.4 1 1.6 2.4 1.9 1.5.3 2.2.9 1.7 2.1-.7 1.3-3.2 1.3-4.2.2',
    vue: 'M4.4 5 8 11.4 11.6 5M6.2 5 8 8.2 9.8 5',
    md: 'M4.2 10.8V5.4l1.9 2.6 1.9-2.6v5.4M10.8 5.4v4.2M9.4 8.4l1.4 1.6 1.4-1.6',
    python: 'M6 11V5.2h2.6a1.6 1.6 0 0 1 0 3.2H6',
    rust: 'M6 11V5.2h2.6a1.5 1.5 0 0 1 0 3H6M8.4 8.3 10.4 11',
    go: 'M10.6 6.2c-.4-.8-1.2-1.2-2.2-1.2C6.800 5 5.500 6.300 5.500 8s1.300 3 2.900 3c1.300 0 2.200-.7 2.200-1.900H8.600',
    java: 'M9.4 5v4.4c0 1.100-.7 1.700-1.700 1.700-.8 0-1.300-.3-1.700-.9',
    c: 'M10.4 6.2C9.900 5.400 9.100 5 8.200 5 6.600 5 5.400 6.300 5.400 8s1.200 3 2.800 3c.9 0 1.700-.4 2.200-1.200',
    php: 'M3.8 8c0-1.400 1.900-2.600 4.200-2.600s4.200 1.200 4.200 2.600-1.900 2.600-4.200 2.600S3.800 9.400 3.800 8Z',
    ruby: 'M5.200 6.200 6.800 4.800h2.400l1.600 1.400L8 11.400Z',
    swift: 'M10.500 5.500C9.800 5 8.800 4.800 7.800 5.100 6.300 5.600 6 7 7 7.700 8.100 8.400 10 8.700 10 9.900c0 1-1.500 1.500-3 1.100',
    shell: 'M5 6.200 7.300 8 5 9.800M8.600 10.600h2.600',
    sql: 'M4.800 6.200c0-.8 1.500-1.400 3.200-1.400s3.200.6 3.200 1.400-1.500 1.400-3.200 1.400-3.200-.6-3.200-1.400ZM4.800 6.200v3.600c0 .8 1.500 1.400 3.200 1.400s3.200-.6 3.200-1.400V6.200',
    config: 'M4.500 6h7M4.500 10h7M6.500 4.800v2.400M9.500 8.800v2.400',
    docker: 'M4.200 8h2v2h-2ZM6.200 8h2v2h-2ZM8.200 8h2v2h-2ZM6.200 6h2v2h-2Z',
    image: 'M4 11 6.600 7.800 8.600 10 9.800 8.600 12 11ZM10.500 5.600v.1',
    pdf: 'M5 6h6M5 8.200h6M5 10.400h3.600',
    archive: 'M8 4.200v1.400M8 6.600v1.400M8 9v1.400M6.600 11.200h2.800',
    lock: 'M5.600 7.600h4.800v3.600H5.600ZM6.600 7.600V6.500a1.400 1.400 0 0 1 2.800 0v1.100',
    git: 'M6 4.600v6.800M6 9c0-1.800 4-.4 4-2.800M6 4.600v.1M10 6.200v.1M6 11.400v.1',
    readme: 'M4.600 5.200h3v6.200h-3ZM8.400 5.200h3v6.200h-3Z',
    license: 'M8 4.600v6.800M5 6.200h6M5 6.200 4.200 9h1.600ZM11 6.200 10.200 9h1.600Z',
  }
  const BADGES: Record<string, string> = {
    git: 'M6.500 8.200v3.400M6.500 10c0-1.300 3-.4 3-1.800',
    node: 'M6 9.500h4M8 8v3',
    src: 'M6.800 9 5.600 10.200 6.800 11.400M9.400 9l1.200 1.200-1.200 1.200',
    docs: 'M6 9.300h4.400M6 11h3',
    test: 'M6 10.200l1.500 1.500 2.800-3',
    dist: 'M8.200 8.200v3.400M6.600 10.200l1.600 1.500 1.600-1.500',
    dot: 'M8.200 10.200v.1',
  }
</script>

<svg
  class="fi"
  class:root
  class:folder={isFolder}
  class:open
  width={size}
  height={size}
  viewBox="0 0 16 16"
  fill="none"
  stroke-linecap="round"
  stroke-linejoin="round"
  aria-hidden="true"
  focusable="false"
  style:color
>
  {#if isFolder}
    {#if open}
      <path d="M1.5 12.3V4.3a1.2 1.2 0 0 1 1.2-1.2h3l1.4 1.6h5.3a1.2 1.2 0 0 1 1.2 1.2v1.3" stroke="currentColor" stroke-width="1.3" />
      <path d="M3.5 7.2h11a.6.6 0 0 1 .6.8l-1.500 4.600a1 1 0 0 1-1 .7H2.700a1.200 1.200 0 0 1-1.200-1.200L2.600 8a1 1 0 0 1 .9-.8Z" fill="currentColor" />
    {:else}
      <path d="M1.500 4.300a1.200 1.200 0 0 1 1.200-1.200h3.100l1.400 1.600h6.100a1.200 1.200 0 0 1 1.200 1.200v6.300a1.200 1.200 0 0 1-1.200 1.200H2.700a1.200 1.200 0 0 1-1.200-1.200Z" fill="currentColor" />
    {/if}
    {#if BADGES[badge]}
      <path d={BADGES[badge]} stroke="rgba(10,12,22,.75)" stroke-width="1.3" transform={open ? 'translate(.6 .4)' : ''} />
    {/if}
  {:else if id === 'file'}
    <path d="M4 1.800h5.200L12.700 5.300v8.400a.8.8 0 0 1-.8.8H4a.8.8 0 0 1-.8-.8V2.600a.8.8 0 0 1 .8-.8ZM9.200 1.800v3.500h3.500" stroke="currentColor" stroke-width="1.2" />
    <path d="M5.500 9h5M5.500 11h3.200" stroke="currentColor" stroke-width="1.2" opacity=".7" />
  {:else}
    <rect x="1.500" y="1.500" width="13" height="13" rx="3.200" fill="currentColor" />
    <path d={GLYPHS[id] ?? ''} stroke="rgba(10,12,22,.8)" stroke-width="1.400" />
  {/if}
</svg>

<style>
  .fi { flex: none; display: block; overflow: visible; }
  .folder.root { filter: drop-shadow(0 0 2px color-mix(in srgb, currentColor 55%, transparent)); }
  .folder.open { filter: brightness(1.15); }
  .folder.root.open { filter: brightness(1.15) drop-shadow(0 0 2px color-mix(in srgb, currentColor 55%, transparent)); }
</style>
