import { readdirSync, readFileSync, statSync } from 'node:fs'
import { join } from 'node:path'
import { describe, expect, it, vi } from 'vitest'

const winListen = vi.fn(async () => () => {})
vi.mock('@tauri-apps/api/webviewWindow', () => ({ getCurrentWebviewWindow: () => ({ listen: winListen }) }))
const globalListen = vi.fn()
vi.mock('@tauri-apps/api/event', () => ({ listen: globalListen }))

import { listenProject, projectEventName } from './events'

describe('listenProject', () => {
  it('subscribes on the current window to the name suffixed with the project id', async () => {
    const cb = () => {}
    await listenProject('conv:event', 'p-123', cb)
    expect(winListen).toHaveBeenCalledWith('conv:event:p-123', cb)
    expect(globalListen).not.toHaveBeenCalled()
  })
  it('builds the same names as projects::project_event', () => {
    for (const n of ['conv:event', 'conv:upsert', 'conv:removed', 'conv:focus', 'fs:changed', 'pty:data', 'pty:exit'] as const) {
      expect(projectEventName(n, 'abc')).toBe(`${n}:abc`)
    }
  })
})

describe('no global listeners', () => {
  const ROOT = join(__dirname, '..')
  const sources = (dir: string): string[] =>
    readdirSync(dir).flatMap((n) => {
      const p = join(dir, n)
      return statSync(p).isDirectory() ? sources(p) : /\.(svelte|ts)$/.test(n) && !/\.test\.ts$/.test(n) ? [p] : []
    })
  it('no source imports the global `listen` (events reach every window)', () => {
    for (const f of sources(ROOT)) {
      const text = readFileSync(f, 'utf8')
      expect(/import\s*\{[^}]*\blisten\b[^}]*\}\s*from\s*'@tauri-apps\/api\/event'/.test(text), f).toBe(false)
    }
  })
})
