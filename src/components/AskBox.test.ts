// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from '@testing-library/svelte'
import { afterEach, describe, expect, it, vi } from 'vitest'

const answer = vi.fn(async () => {})
vi.mock('../lib/stores/chat.svelte', () => ({ chat: { answer: (...a: unknown[]) => answer(...(a as [])) } }))
vi.mock('@tauri-apps/plugin-opener', () => ({ openUrl: vi.fn() }))

import AskBox from './AskBox.svelte'
import type { ChatItem } from '../lib/types'

type Ask = Extract<ChatItem, { kind: 'ask' }>
const ask = (over: Partial<Ask>): Ask => ({ kind: 'ask', id: 'ask:r', requestId: 'r', askType: 'permission', prompt: 'Bash: ls', options: [], answered: false, ...over })
afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

describe('AskBox', () => {
  it('permission: Consenti / Nega', async () => {
    render(AskBox, { item: ask({}), sessionId: 's' })
    await fireEvent.click(screen.getByText('Consenti'))
    expect(answer).toHaveBeenCalledWith('s', 'r', 'allow')
    await fireEvent.click(screen.getByText('Nega'))
    expect(answer).toHaveBeenLastCalledWith('s', 'r', 'deny')
  })
  it('plan: renders sanitised markdown and approve / reject', async () => {
    const { container } = render(AskBox, { item: ask({ askType: 'plan', prompt: '# Piano\n\n<script>x</script>\n\n- uno' }), sessionId: 's' })
    expect(container.innerHTML).not.toContain('<script')
    expect(screen.getByText('Piano')).toBeTruthy()
    await fireEvent.click(screen.getByText('Approva piano'))
    expect(answer).toHaveBeenCalledWith('s', 'r', 'approve')
    await fireEvent.click(screen.getByText('Continua a pianificare'))
    expect(answer).toHaveBeenLastCalledWith('s', 'r', 'reject')
  })
  it('question: one button per option plus free text', async () => {
    render(AskBox, { item: ask({ askType: 'question', prompt: 'Come procedo?', options: ['Sequence Postgres', 'Lock sulla riga'] }), sessionId: 's' })
    await fireEvent.click(screen.getByText('Lock sulla riga'))
    expect(answer).toHaveBeenCalledWith('s', 'r', 'Lock sulla riga')
    await fireEvent.input(screen.getByLabelText('Risposta libera'), { target: { value: 'Altro' } })
    await fireEvent.click(screen.getByText('Invia'))
    expect(answer).toHaveBeenLastCalledWith('s', 'r', 'Altro')
  })
  it('answered: no buttons', () => {
    render(AskBox, { item: ask({ answered: true }), sessionId: 's' })
    expect(screen.queryByText('Consenti')).toBeNull()
  })
})
