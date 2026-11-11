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
  // solo lato interfaccia: lo crea il riduttore dagli eventi `ask` (la storia letta da disco non lo contiene)
  | { kind: 'ask'; id: string; requestId: string; askType: 'permission' | 'question' | 'plan'; prompt: string; options: string[]; questions: AskQuestion[]; answered: boolean }
  | { kind: 'raw'; id: string; label: string; json: string }
export type Todo = { text: string; status: 'pending' | 'in_progress' | 'completed' }

export type AskType = 'permission' | 'question' | 'plan'
/** Una domanda di `AskUserQuestion` (specchio di `AskQuestion` in protocol.rs). */
export type AskQuestion = { question: string; header: string; options: { label: string; description: string }[]; multiSelect: boolean }
/** Etichetta degli eventi `unknown` che riportano un errore della CLI o dell'API (`ERROR_LABEL` in protocol.rs). */
export const ERROR_LABEL = 'Errore di Claude'
/** Etichetta del marcatore di turno rimasto a metà al riavvio (`INTERRUPTED_LABEL` in engine.rs). */
export const INTERRUPTED_LABEL = 'Turno interrotto — Riprendi'
/** Evento del protocollo Claude (`conv:event`); specchio di `ClaudeEvent` in src-tauri/src/claude/protocol.rs. */
export type ClaudeEvent =
  | { kind: 'init'; sessionId: string; model: string; slashCommands: string[]; tools: string[]; permissionMode: string }
  | { kind: 'textDelta'; id: string; text: string }
  | { kind: 'thinking'; id: string; text: string }
  | { kind: 'toolStart'; id: string; name: string; input: unknown; parentId: string | null }
  | { kind: 'toolEnd'; id: string; ok: boolean; result: string }
  | { kind: 'ask'; requestId: string; askType: AskType; prompt: string; options: string[]; questions: AskQuestion[]; toolName: string; input: unknown }
  | { kind: 'askCancelled'; requestId: string }
  | { kind: 'todoDelta'; id: string; text: string | null; status: string | null }
  | { kind: 'turnEnd'; contextUsedPct: number | null; isError: boolean }
  | { kind: 'controlResponse'; requestId: string; ok: boolean; payload: unknown }
  | { kind: 'unknown'; label: string; json: string }

/** Esito di `claude_check`. */
export type ClaudeCheck = { found: boolean; version: string | null; path: string | null }
/** Opzioni di `conv_start`: senza `permissionMode` vale l'impostazione `default_permission_mode`. */
export type ConvStartOpts = { model?: string; permissionMode?: string }
/** Payload dell'evento `conv:event`. */
export type ConvEventPayload = { sessionId: string; event: ClaudeEvent }

/** Esito di `conv_delete`: `busy` = c'è già una conferma aperta (da ignorare in silenzio). */
export type ConvDeleteOutcome =
  | { kind: 'deleted' }
  | { kind: 'deletedWithWarning'; message: string }
  | { kind: 'cancelled' }
  | { kind: 'busy' }
