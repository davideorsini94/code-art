// @vitest-environment jsdom
import { cleanup, render, screen } from '@testing-library/svelte'
import { afterEach, expect, it, vi } from 'vitest'

vi.mock('../lib/stores/sidebar.svelte', () => ({ sidebar: { view: 'git' } }))
vi.mock('./FileTree.svelte', () => ({ default: () => {} }))
vi.mock('./SearchPanel.svelte', () => ({ default: () => {} }))
const dbLoaded = vi.hoisted(() => ({ n: 0 }))
vi.mock('./db/DbPanel.svelte', () => {
  dbLoaded.n++
  return { default: () => {} }
})
vi.mock('./git/GitPanel.svelte', () => {
  throw new Error('chunk mancante')
})

import SidebarView from './SidebarView.svelte'

afterEach(cleanup)

it('a Git view that fails to load shows a message instead of a blank sidebar', async () => {
  render(SidebarView)
  expect(await screen.findByText('Vista Git non caricata')).toBeTruthy()
})

it('the Database panel is not loaded until its view is shown', async () => {
  const { sidebar } = await import('../lib/stores/sidebar.svelte')
  render(SidebarView)
  expect(dbLoaded.n).toBe(0)
  sidebar.view = 'db'
  render(SidebarView)
  await vi.waitFor(() => expect(dbLoaded.n).toBe(1))
})
