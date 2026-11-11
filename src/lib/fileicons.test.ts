import { describe, expect, it } from 'vitest'
import { iconFor } from './fileicons'

const id = (name: string, isDir = false, open = false) => iconFor(name, isDir, open).id

describe('iconFor file', () => {
  const cases: [string, string][] = [
    ['a.ts', 'ts'], ['a.tsx', 'ts'], ['a.js', 'js'], ['a.jsx', 'js'], ['a.mjs', 'js'], ['a.cjs', 'js'],
    ['a.json', 'json'], ['a.html', 'html'], ['a.css', 'css'], ['a.scss', 'css'],
    ['A.svelte', 'svelte'], ['A.vue', 'vue'], ['a.md', 'md'], ['a.py', 'python'], ['a.rs', 'rust'],
    ['a.go', 'go'], ['A.java', 'java'], ['A.kt', 'java'], ['a.c', 'c'], ['a.cpp', 'c'], ['a.h', 'c'], ['A.cs', 'c'],
    ['a.php', 'php'], ['a.rb', 'ruby'], ['a.swift', 'swift'],
    ['a.sh', 'shell'], ['a.zsh', 'shell'], ['a.bash', 'shell'], ['a.sql', 'sql'],
    ['a.yaml', 'config'], ['a.yml', 'config'], ['a.toml', 'config'], ['a.ini', 'config'], ['.env', 'config'], ['.env.local', 'config'],
    ['Dockerfile', 'docker'], ['docker-compose.yml', 'docker'],
    ['a.png', 'image'], ['a.jpg', 'image'], ['a.gif', 'image'], ['a.svg', 'image'], ['a.webp', 'image'], ['a.ico', 'image'],
    ['a.pdf', 'pdf'], ['a.zip', 'archive'], ['a.tgz', 'archive'], ['Cargo.lock', 'lock'], ['pnpm-lock.yaml', 'lock'],
    ['package-lock.json', 'lock'], ['.gitignore', 'git'], ['.gitattributes', 'git'],
    ['README.md', 'readme'], ['readme', 'readme'], ['LICENSE', 'license'], ['LICENSE-MIT', 'license'],
    ['notes.txt', 'file'], ['weird.zzz', 'file'],
  ]
  it.each(cases)('%s -> %s', (name, expected) => expect(id(name)).toBe(expected))

  it('is case-insensitive', () => {
    expect(id('MAIN.TS')).toBe('ts')
    expect(id('DOCKERFILE')).toBe('docker')
    expect(id('Readme.MD')).toBe('readme')
  })
  it('handles multi-dot names', () => {
    expect(id('docker-compose.prod.yml')).toBe('docker')
    expect(id('foo.test.ts')).toBe('ts')
    expect(id('archive.tar.gz')).toBe('archive')
    expect(id('a.b.c.d.css')).toBe('css')
  })
  it('handles names without extension and odd names', () => {
    expect(id('Makefile')).toBe('file')
    expect(id('trailingdot.')).toBe('file')
    expect(id('')).toBe('file')
    expect(id('__proto__')).toBe('file')
    expect(id('constructor')).toBe('file')
    expect(id('x.constructor')).toBe('file')
  })
  it('never dims files', () => {
    expect(iconFor('.gitignore', false, false).dim).toBe(false)
  })
})

describe('iconFor folder', () => {
  it('distinguishes closed and open', () => {
    expect(id('lib', true, false)).toBe('folder')
    expect(id('lib', true, true)).toBe('folder-open')
  })
  it('recognises well-known folders', () => {
    expect(id('.git', true)).toBe('folder-git')
    expect(id('.github', true, true)).toBe('folder-git-open')
    expect(id('node_modules', true)).toBe('folder-node')
    expect(id('src', true)).toBe('folder-src')
    expect(id('docs', true)).toBe('folder-docs')
    for (const n of ['test', 'tests', '__tests__']) expect(id(n, true)).toBe('folder-test')
    for (const n of ['dist', 'build', 'target']) expect(id(n, true)).toBe('folder-dist')
  })
  it('dims other dot-folders but not known ones', () => {
    for (const n of ['.idea', '.vscode']) {
      expect(iconFor(n, true, false)).toEqual({ id: 'folder', dim: true })
      expect(iconFor(n, true, true)).toEqual({ id: 'folder-open', dim: true })
    }
    for (const n of ['.git', '.github', 'src', 'lib']) expect(iconFor(n, true, false).dim).toBe(false)
  })
  it('is case-insensitive and ignores file extension rules for dirs', () => {
    expect(id('SRC', true)).toBe('folder-src')
    expect(id('foo.ts', true)).toBe('folder')
    expect(id('src', false)).toBe('file')
  })
})
