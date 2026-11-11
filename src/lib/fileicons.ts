// Mappa nome -> icona. I nomi dei file sono testo non fidato: passano solo da questa ricerca
// (Map, mai oggetti con prototipo) e non vengono mai interpretati.
export type FileIconSpec = { id: string; color: string }

const FOLDER = '#8fa8ff'
const FOLDER_OPEN = '#b4c4ff'

const FOLDERS = new Map<string, [string, string]>([
  ['.git', ['git', '#f08a65']],
  ['.github', ['git', '#f08a65']],
  ['node_modules', ['node', '#8bc66a']],
  ['src', ['src', '#6aa3ff']],
  ['docs', ['docs', '#f2d36b']],
  ['test', ['test', '#5ee0c1']],
  ['tests', ['test', '#5ee0c1']],
  ['__tests__', ['test', '#5ee0c1']],
  ['dist', ['dist', '#ffb454']],
  ['build', ['dist', '#ffb454']],
  ['target', ['dist', '#ffb454']],
])

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

const COLORS: Record<string, string> = {
  ts: '#6aa3ff', js: '#f2d36b', json: '#d9c26b', html: '#ff8a8a', css: '#b48cff', svelte: '#ff8a5c',
  vue: '#6bd0a0', md: '#9aa3b8', python: '#6bb5d0', rust: '#e8a07a', go: '#5ec8e0', java: '#e8946b',
  c: '#7aa8e8', php: '#9d9be0', ruby: '#ff7a8a', swift: '#ff9a5c', shell: '#6bd0a0', sql: '#d0a86b',
  config: '#9aa3b8', docker: '#5aa8ff', image: '#5ee0c1', pdf: '#ff7a7a', archive: '#c9a46b',
  lock: '#667089', git: '#f08a65', readme: '#8fa8ff', license: '#d9c26b', file: '#9aa3b8',
}

function fileId(name: string): string {
  const n = name.toLowerCase()
  const exact = NAMES.get(n)
  if (exact) return exact
  if (n.startsWith('readme')) return 'readme'
  if (n.startsWith('license') || n.startsWith('licence')) return 'license'
  if (n.startsWith('docker-compose.')) return 'docker'
  if (n.startsWith('.env.')) return 'config'
  if (n.endsWith('-lock.json') || n.endsWith('-lock.yaml')) return 'lock'
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
    if (known) return { id: `folder-${known[0]}${sfx}`, color: known[1] }
    if (n.startsWith('.')) return { id: `folder-dot${sfx}`, color: 'var(--dim)' }
    return { id: `folder${sfx}`, color: open ? FOLDER_OPEN : FOLDER }
  }
  const id = fileId(name)
  return { id, color: COLORS[id] ?? COLORS.file }
}
