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
