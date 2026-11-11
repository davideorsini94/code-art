<script lang="ts">
  import { KNOWN_FOLDER_KINDS } from '../lib/fileicons'
  // Icona decorativa. Le immagini (Material Icon Theme, MIT: src/assets/fileicons/LICENSE) sono asset
  // statici del bundle, mostrati con <img src>: nessun {@html}, nessun markup da dati esterni.
  // `id` arriva solo da iconFor() e fa da chiave in una mappa fissa.
  const URLS = import.meta.glob('../assets/fileicons/*.svg', { eager: true, query: '?url', import: 'default' }) as Record<string, string>
  const url = (n: string): string | undefined => URLS[`../assets/fileicons/${n}.svg`]

  const FILES: Record<string, string> = {
    ts: 'typescript', js: 'javascript', json: 'json', html: 'html', css: 'css', svelte: 'svelte', vue: 'vue',
    md: 'markdown', python: 'python', rust: 'rust', go: 'go', java: 'java', c: 'c', php: 'php', ruby: 'ruby',
    swift: 'swift', shell: 'console', sql: 'database', config: 'settings', docker: 'docker', image: 'image',
    pdf: 'pdf', archive: 'zip', lock: 'lock', git: 'git', readme: 'readme', license: 'certificate', file: 'file',
  }

  let { id, dim = false, size = 16, root = false }: { id: string; dim?: boolean; size?: number; root?: boolean } = $props()

  const isFolder = $derived(id.startsWith('folder'))
  const open = $derived(id.endsWith('-open'))
  const src = $derived.by(() => {
    if (!isFolder) return url(FILES[id] ?? 'file') ?? url('file')
    const kind = id.replace(/-open$/, '').slice(7)
    const base = root ? 'folder-root' : KNOWN_FOLDER_KINDS.has(kind) ? `folder-${kind}` : 'folder'
    return url(open ? `${base}-open` : base) ?? url(open ? 'folder-open' : 'folder')
  })
</script>

<img class="fi" class:dim class:root class:open src={src} width={size} height={size} alt="" aria-hidden="true" draggable="false" />

<style>
  .fi { flex: none; display: block; }
  .fi.dim { opacity: 0.55; }
  .fi.open:not(.root) { filter: brightness(1.12); }
</style>
