// Mappa nome -> icona. I nomi dei file sono testo non fidato: passano solo da questa ricerca
// (Map, mai oggetti con prototipo) e non vengono mai interpretati.
export type FileIconSpec = { id: string; dim: boolean }

// cartelle note -> tipo di icona (ognuna ha un gemello -open nel set)
const FOLDERS = new Map<string, string>([
  ['.git', 'git'], ['.github', 'git'], ['node_modules', 'node'], ['src', 'src'], ['docs', 'docs'],
  ['test', 'test'], ['tests', 'test'], ['__tests__', 'test'],
  ['dist', 'dist'], ['build', 'dist'], ['target', 'dist'],
])
export const KNOWN_FOLDER_KINDS: ReadonlySet<string> = new Set(FOLDERS.values())

const NAMES = new Map<string, string>([
  ['dockerfile', 'docker'], ['.dockerignore', 'docker'],
  ['.gitignore', 'git'], ['.gitattributes', 'git'], ['.gitmodules', 'git'], ['.gitkeep', 'git'],
  ['.env', 'config'], ['.editorconfig', 'config'], ['.npmrc', 'config'], ['.prettierrc', 'config'],
  ['.bashrc', 'shell'], ['.zshrc', 'shell'], ['.profile', 'shell'],
  ['package-lock.json', 'lock'], ['pnpm-lock.yaml', 'lock'], ['yarn.lock', 'lock'],
])

const EXTS = new Map<string, string>()
const family = (id: string, exts: string) => exts.split(' ').forEach((x) => EXTS.set(x, id))
family('ts', 'ts tsx mts cts')
family('js', 'js jsx mjs cjs')
family('json', 'json jsonc json5')
family('html', 'html htm')
family('css', 'css scss sass less')
family('svelte', 'svelte')
family('vue', 'vue')
family('md', 'md mdx markdown')
family('python', 'py pyi')
family('rust', 'rs')
family('go', 'go')
family('java', 'java kt kts')
family('c', 'c h cpp cc cxx hpp cs')
family('php', 'php')
family('ruby', 'rb')
family('swift', 'swift')
family('shell', 'sh zsh bash fish')
family('sql', 'sql')
family('config', 'yaml yml toml ini env conf cfg')
family('image', 'png jpg jpeg gif svg webp ico bmp avif')
family('pdf', 'pdf')
family('archive', 'zip tar gz tgz bz2 xz 7z rar')
family('lock', 'lock')

function fileId(name: string): string {
  const n = name.toLowerCase()
  const exact = NAMES.get(n)
  if (exact) return exact
  if (n.startsWith('readme')) return 'readme'
  if (n.startsWith('license') || n.startsWith('licence')) return 'license'
  if (n.startsWith('docker-compose.')) return 'docker'
  if (n.startsWith('.env.')) return 'config'
  const dot = n.lastIndexOf('.')
  if (dot > 0 && dot < n.length - 1) {
    const byExt = EXTS.get(n.slice(dot + 1))
    if (byExt) return byExt
  }
  return 'file'
}

export function iconFor(name: string, isDir: boolean, open: boolean): FileIconSpec {
  if (isDir) {
    const sfx = open ? '-open' : ''
    const n = name.toLowerCase()
    const known = FOLDERS.get(n)
    if (known) return { id: `folder-${known}${sfx}`, dim: false }
    return { id: `folder${sfx}`, dim: n.startsWith('.') }
  }
  return { id: fileId(name), dim: false }
}
