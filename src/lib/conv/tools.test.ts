import { describe, expect, it } from 'vitest'
import { capText, diffAction, isEditTool, isSubagent, toolSummary } from './tools'

describe('tools helpers', () => {
  it('summarises by main argument', () => {
    expect(toolSummary('Read', { file_path: '/Users/dev/p/a.txt' })).toBe('/Users/dev/p/a.txt')
    expect(toolSummary('Bash', { command: 'ls', description: 'x' })).toBe('ls')
    expect(toolSummary('Agent', { description: 'Leggi a.txt', prompt: 'lungo' })).toBe('Leggi a.txt')
    expect(toolSummary('Bash', { command: 'x'.repeat(300) }).length).toBe(141)
    expect(toolSummary('Foo', null)).toBe('')
  })
  it('classifies tools', () => {
    expect(isSubagent('Agent') && isSubagent('Task') && !isSubagent('Read')).toBe(true)
    expect(['Edit', 'Write', 'MultiEdit'].every(isEditTool) && !isEditTool('Read')).toBe(true)
  })
  it('Edit gives a diff of the fragments; Write opens the file', () => {
    expect(diffAction('Edit', { file_path: '/p/a.txt', old_string: 'red', new_string: 'blue' }, '/p/a.txt')).toEqual({ type: 'diff', path: '/p/a.txt', before: 'red', after: 'blue' })
    expect(diffAction('Edit', { file_path: '/p/a.txt', old_string: 'red', new_string: '' }, '/p/a.txt')).toMatchObject({ type: 'diff', after: '' })
    expect(diffAction('Write', { file_path: '/p/a.txt', content: 'hi' }, '/p/a.txt')).toEqual({ type: 'open', path: '/p/a.txt' })
    expect(diffAction('Edit', { file_path: '/p/a.txt' }, null)).toEqual({ type: 'open', path: '/p/a.txt' })
    expect(diffAction('MultiEdit', { file_path: '/p/a', edits: [{ old_string: 'a', new_string: 'b' }, { old_string: 'c', new_string: 'd' }] }, '/p/a')).toEqual({ type: 'diff', path: '/p/a', before: 'a\nc', after: 'b\nd' })
    expect(diffAction('Read', { file_path: '/p/a' }, '/p/a')).toBeNull()
    expect(diffAction('Edit', {}, null)).toBeNull()
  })
  it('caps long text', () => {
    expect(capText('abc', 5)).toEqual({ text: 'abc', truncated: false })
    expect(capText('abcdef', 5)).toEqual({ text: 'abcde', truncated: true })
  })
})
