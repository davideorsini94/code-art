// @vitest-environment jsdom
import { cleanup, render, screen } from '@testing-library/svelte'
import { afterEach, expect, it, vi } from 'vitest'

vi.mock('../lib/stores/sidebar.svelte', () => ({ sidebar: { view: 'git' } }))
vi.mock('./FileTree.svelte', () => ({ default: () => {} }))
vi.mock('./SearchPanel.svelte', () => ({ default: () => {} }))
vi.mock('./git/GitPanel.svelte', () => {
  throw new Error('chunk mancante')
})

import SidebarView from './SidebarView.svelte'

afterEach(cleanup)

it('a Git view that fails to load shows a message instead of a blank sidebar', async () => {
  render(SidebarView)
  expect(await screen.findByText('Vista Git non caricata')).toBeTruthy()
})
