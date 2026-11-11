import { describe, expect, it, vi } from 'vitest'

const openUrl = vi.fn(async (_u: string) => {})
vi.mock('@tauri-apps/plugin-opener', () => ({ openUrl: (u: string) => openUrl(u) }))

import { openExternal } from './external'

describe('openExternal (also Monaco ⌘-click link opener)', () => {
  it('opens only http(s) in the system browser and always reports handled', async () => {
    expect(openExternal('https://example.com/a')).toBe(true)
    await new Promise((r) => setTimeout(r, 0))
    expect(openUrl).toHaveBeenCalledWith('https://example.com/a')
    openUrl.mockClear()
    for (const bad of ['file:///etc/passwd', 'javascript:alert(1)', 'command:workbench', 'vscode://x', '/relativo']) {
      expect(openExternal(bad)).toBe(true)
    }
    await new Promise((r) => setTimeout(r, 0))
    expect(openUrl).not.toHaveBeenCalled()
  })
  it('monaco.ts registers the opener through openExternal', async () => {
    const { readFileSync } = await import('node:fs')
    const src = readFileSync(new URL('./monaco.ts', import.meta.url), 'utf8')
    expect(src).toMatch(/registerLinkOpener\(\{\s*open:\s*\(uri\)\s*=>\s*openExternal\(/)
  })
})
