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
