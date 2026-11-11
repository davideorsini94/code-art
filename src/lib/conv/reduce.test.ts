import { describe, expect, it } from 'vitest'
import { answerAsk, appendUser, buildRows, initialState, reconcile, reduce, reduceAll } from './reduce'
import type { ChatItem, ClaudeEvent } from '../types'

const run = (evs: ClaudeEvent[], history: ChatItem[] = []) => reduceAll(initialState(history), evs)
const tool = (id: string, name = 'Read', parentId: string | null = null, input: unknown = { file_path: '/Users/dev/p/a.txt' }): ClaudeEvent => ({
  kind: 'toolStart', id, name, input, parentId,
})

describe('reduce', () => {
  it('text_deltas_merge_by_id', () => {
    const s = run([
      { kind: 'textDelta', id: 'm:0', text: 'ciao ' },
      { kind: 'textDelta', id: 'm:0', text: 'mondo' },
      { kind: 'textDelta', id: 'm:1', text: 'altro' },
    ])
    expect(s.items).toEqual([
      { kind: 'assistant', id: 'm:0', text: 'ciao mondo' },
      { kind: 'assistant', id: 'm:1', text: 'altro' },
    ])
  })

  it('thinking_accumulates_by_id_even_when_empty', () => {
    const s = run([{ kind: 'thinking', id: 't', text: '' }, { kind: 'thinking', id: 't', text: 'hmm' }])
    expect(s.items).toEqual([{ kind: 'thinking', id: 't', text: 'hmm' }])
  })

  it('tool_end_completes_matching_tool', () => {
    const s = run([tool('a'), tool('b'), { kind: 'toolEnd', id: 'a', ok: true, result: 'fatto' }])
    const [a, b] = s.items
    expect(a).toMatchObject({ kind: 'tool', id: 'a', ok: true, result: 'fatto', filePath: '/Users/dev/p/a.txt' })
    expect(b).toMatchObject({ kind: 'tool', id: 'b', ok: null, result: null })
  })

  it('tool_end_for_missing_id_is_ignored', () => {
    const before = run([tool('a')])
    const after = reduce(before, { kind: 'toolEnd', id: 'zzz', ok: false, result: 'x' })
    expect(after.items).toEqual(before.items)
  })

  it('tool_start_duplicate_id_is_ignored (storia + evento)', () => {
    const s = run([tool('a')], [
      { kind: 'tool', id: 'a', name: 'Read', input: {}, result: 'r', ok: true, parentId: null, filePath: null },
    ])
    expect(s.items).toHaveLength(1)
    expect(s.items[0]).toMatchObject({ result: 'r', ok: true })
  })

  it('ask_then_answer_marks_answered', () => {
    const s = run([{ kind: 'ask', requestId: 'r1', askType: 'permission', prompt: 'Bash: ls', options: [], questions: [], toolName: 'Bash', input: {} }])
    expect(s.items[0]).toMatchObject({ kind: 'ask', requestId: 'r1', answered: false })
    const s2 = answerAsk(s, 'r1')
    expect(s2.items[0]).toMatchObject({ answered: true })
    expect(s.items[0]).toMatchObject({ answered: false }) // immutabile
  })

  it('ask_cancelled_marks_answered_and_unknown_request_is_ignored', () => {
    let s = run([{ kind: 'ask', requestId: 'r1', askType: 'plan', prompt: '# Piano', options: [], questions: [], toolName: 'ExitPlanMode', input: {} }])
    s = reduce(s, { kind: 'askCancelled', requestId: 'nope' })
    expect(s.items[0]).toMatchObject({ answered: false })
    s = reduce(s, { kind: 'askCancelled', requestId: 'r1' })
    expect(s.items[0]).toMatchObject({ answered: true })
  })

  it('todo_deltas_merge_by_id', () => {
    let s = run([
      { kind: 'todoDelta', id: '1', text: 'Scrivi intro', status: 'pending' },
      { kind: 'todoDelta', id: '2', text: 'Test', status: 'pending' },
      { kind: 'todoDelta', id: '1', text: null, status: 'in_progress' },
    ])
    expect(s.todos).toEqual([
      { id: '1', text: 'Scrivi intro', status: 'in_progress' },
      { id: '2', text: 'Test', status: 'pending' },
    ])
    s = reduce(s, { kind: 'todoDelta', id: '2', text: null, status: 'completed' })
    expect(s.todos[1].status).toBe('completed')
    // stato sconosciuto -> pending; "deleted" rimuove
    s = reduce(s, { kind: 'todoDelta', id: '3', text: 'x', status: 'boh' })
    expect(s.todos[2].status).toBe('pending')
    s = reduce(s, { kind: 'todoDelta', id: '3', text: null, status: 'deleted' })
    expect(s.todos.map((t) => t.id)).toEqual(['1', '2'])
  })

  it('unknown_becomes_raw_item', () => {
    const s = run([{ kind: 'unknown', label: 'system/foo', json: '{"a":1}' }, { kind: 'unknown', label: 'system/foo', json: '{"a":2}' }])
    expect(s.items).toHaveLength(2)
    expect(s.items[0]).toMatchObject({ kind: 'raw', label: 'system/foo', json: '{"a":1}' })
    expect(new Set(s.items.map((i) => i.id)).size).toBe(2)
  })

  it('turn_end_sets_context_and_init_and_control_response_are_ignored', () => {
    const s = run([
      { kind: 'init', sessionId: 's', model: 'm', slashCommands: [], tools: [], permissionMode: 'default' },
      { kind: 'controlResponse', requestId: 'x', ok: true, payload: {} },
      { kind: 'turnEnd', contextUsedPct: 41.5, isError: false },
    ])
    expect(s.items).toEqual([])
    expect(s.contextUsedPct).toBe(41.5)
  })

  it('replaying a simple turn yields one user and one assistant', () => {
    const s = reduceAll(
      appendUser(initialState([]), 'u1', 'Di ciao', []),
      [
        { kind: 'init', sessionId: 's', model: 'm', slashCommands: [], tools: [], permissionMode: 'default' },
        { kind: 'textDelta', id: 'msg:0', text: 'cia' },
        { kind: 'textDelta', id: 'msg:0', text: 'o' },
        { kind: 'turnEnd', contextUsedPct: 3, isError: false },
      ],
    )
    expect(s.items.map((i) => i.kind)).toEqual(['user', 'assistant'])
    expect(s.items[1]).toMatchObject({ text: 'ciao' })
  })

  it('reduce does not mutate its input', () => {
    const s0 = initialState([])
    const snap = JSON.stringify(s0)
    reduce(s0, { kind: 'textDelta', id: 'a', text: 'x' })
    expect(JSON.stringify(s0)).toBe(snap)
  })
})

describe('buildRows', () => {
  it('nests children under their parent and keeps orphans at top level', () => {
    const s = run([
      tool('p', 'Agent', null, { description: 'd' }),
      tool('c1', 'Read', 'p'),
      { kind: 'textDelta', id: 'x', text: 'ok' },
      tool('o', 'Read', 'missing'),
    ])
    const rows = buildRows(s.items)
    expect(rows.map((r) => r.item.id)).toEqual(['p', 'x', 'o'])
    expect(rows[0].children.map((c) => c.id)).toEqual(['c1'])
  })
})

describe('reconcile', () => {
  const hist: ChatItem[] = [
    { kind: 'user', id: 'u', text: 'ciao', images: [] },
    { kind: 'assistant', id: 'uuid-1', text: 'ciao a te' },
  ]
  const live = () =>
    reduceAll(initialState([]), [
      { kind: 'textDelta', id: 'msg:0', text: 'ciao a' }, // taglio: la storia ha il testo completo
      { kind: 'todoDelta', id: '1', text: 'Fai', status: 'pending' },
      { kind: 'ask', requestId: 'r1', askType: 'permission', prompt: 'x', options: [], questions: [], toolName: 'Bash', input: {} },
      { kind: 'turnEnd', contextUsedPct: 12, isError: false },
    ])

  it('replaces items with history, no duplicated or cut text', () => {
    const r = reconcile(live(), hist)
    expect(r.items.filter((i) => i.kind === 'assistant')).toEqual([{ kind: 'assistant', id: 'uuid-1', text: 'ciao a te' }])
  })
  it('keeps todos, context and unanswered asks', () => {
    const r = reconcile(live(), hist)
    expect(r.todos).toHaveLength(1)
    expect(r.contextUsedPct).toBe(12)
    expect(r.items.filter((i) => i.kind === 'ask')).toHaveLength(1)
  })
  it('drops answered asks and does not duplicate asks already in history', () => {
    const answered = answerAsk(live(), 'r1')
    expect(reconcile(answered, hist).items.some((i) => i.kind === 'ask')).toBe(false)
    const withAsk: ChatItem[] = [...hist, { kind: 'ask', id: 'h', requestId: 'r1', askType: 'permission', prompt: 'x', options: [], questions: [], answered: false }]
    expect(reconcile(live(), withAsk).items.filter((i) => i.kind === 'ask')).toHaveLength(1)
  })
  it('pending asks applied afterwards are deduplicated by requestId', () => {
    const ask: ClaudeEvent = { kind: 'ask', requestId: 'r1', askType: 'permission', prompt: 'x', options: [], questions: [], toolName: 'Bash', input: {} }
    const r = reduceAll(reconcile(live(), hist), [ask])
    expect(r.items.filter((i) => i.kind === 'ask')).toHaveLength(1)
  })
})

describe('error notices', () => {
  const err = (json: string): ClaudeEvent => ({ kind: 'unknown', label: 'Errore di Claude', json })
  it('shown once even if the CLI already printed the same text as a synthetic message', () => {
    const s = run([{ kind: 'textDelta', id: 'synthetic:m', text: 'API Error: 529' }, err('API Error: 529'), err('Altro errore'), err('Altro errore')])
    expect(s.items.map((i) => i.kind)).toEqual(['assistant', 'raw'])
  })
  it('survive the end-of-turn reconcile (they are not in the session file)', () => {
    const s = run([err('Not logged in')])
    expect(reconcile(s, [{ kind: 'user', id: 'u', text: 'ciao', images: [] }]).items.map((i) => i.kind)).toEqual(['user', 'raw'])
    expect(reconcile(s, [{ kind: 'assistant', id: 'a', text: 'Not logged in' }]).items.map((i) => i.kind)).toEqual(['assistant'])
  })
  it('asks keep every question', () => {
    const q = [{ question: 'A?', header: 'A', options: [], multiSelect: false }, { question: 'B?', header: 'B', options: [], multiSelect: true }]
    const s = run([{ kind: 'ask', requestId: 'r', askType: 'question', prompt: 'A?', options: [], questions: q, toolName: 'AskUserQuestion', input: {} }])
    expect((s.items[0] as Extract<ChatItem, { kind: 'ask' }>).questions).toEqual(q)
  })
})
