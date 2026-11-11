export type Project = { id: string; name: string; folders: string[] } // folders[0] = principale
export type DirEntry = { name: string; path: string; isDir: boolean; ignored: boolean }
export type FileContent = { text: string; mtimeMs: number }
export type ConvStatus = 'working' | 'waiting' | 'idle'
export type Conversation = {
  sessionId: string
  projectId: string
  cwd: string
  title: string
  prevTitle: string | null
  titleChangedAt: number | null
  titleLocked: boolean
  summary: string
  status: ConvStatus
  owned: boolean
  archived: boolean
  lastActivity: number
  turnCount: number
  model: string | null
  permissionMode: string
  contextUsedPct: number | null
  maybeOpenElsewhere: boolean
}
export type FsError = { code: 'conflict' | 'forbidden' | 'too_large' | 'not_text' | 'io'; message: string }
export type FsChanged = { paths: string[] }
export type ChatItem =
  | { kind: 'user'; id: string; text: string; images: string[] }
  | { kind: 'assistant'; id: string; text: string }
  | { kind: 'thinking'; id: string; text: string }
  | { kind: 'tool'; id: string; name: string; input: unknown; result: string | null; ok: boolean | null; parentId: string | null; filePath: string | null }
  | { kind: 'ask'; id: string; requestId: string; askType: 'permission' | 'question' | 'plan'; prompt: string; options: string[]; answered: boolean }
  | { kind: 'raw'; id: string; label: string; json: string }
export type Todo = { text: string; status: 'pending' | 'in_progress' | 'completed' }

export type AskType = 'permission' | 'question' | 'plan'
/** Evento del protocollo Claude (`conv:event`); specchio di `ClaudeEvent` in src-tauri/src/claude/protocol.rs. */
export type ClaudeEvent =
  | { kind: 'init'; sessionId: string; model: string; slashCommands: string[]; tools: string[]; permissionMode: string }
  | { kind: 'textDelta'; id: string; text: string }
  | { kind: 'thinking'; id: string; text: string }
  | { kind: 'toolStart'; id: string; name: string; input: unknown; parentId: string | null }
  | { kind: 'toolEnd'; id: string; ok: boolean; result: string }
  | { kind: 'ask'; requestId: string; askType: AskType; prompt: string; options: string[]; toolName: string; input: unknown }
  | { kind: 'askCancelled'; requestId: string }
  | { kind: 'todos'; todos: Todo[] }
  | { kind: 'todoDelta'; id: string; text: string | null; status: string | null }
  | { kind: 'turnEnd'; contextUsedPct: number | null; isError: boolean }
  | { kind: 'controlResponse'; requestId: string; ok: boolean; payload: unknown }
  | { kind: 'unknown'; label: string; json: string }
