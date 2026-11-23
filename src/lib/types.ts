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

// ---- ricerca nel progetto (specchio di src-tauri/src/search.rs) ----
export type SearchQuery = { text: string; regex: boolean; caseSensitive: boolean; wholeWord: boolean; include: string[]; exclude: string[] }
/** `col` e `previewCol`/`previewLen`: unità UTF-16, come le stringhe JS (`preview.slice(previewCol, previewCol + previewLen)`). */
export type SearchHit = { path: string; line: number; col: number; len: number; preview: string; previewCol: number; previewLen: number }
export type SearchSummary = { files: number; hits: number; truncated: boolean; cancelled: boolean; error: string | null }
export type ReplaceOutcome = { files: number; replacements: number; skipped: string[]; cancelled: boolean }

// ---- git (specchio di src-tauri/src/git/) ----
/** Voce di `git_status`: lettere di stato nell'indice (`staged`) e nella cartella di lavoro (`unstaged`). */
export type GitStatusEntry = {
  path: string
  origPath: string | null
  staged: string | null
  unstaged: string | null
  conflict: boolean
  untracked: boolean
}
export type GitRepoStatus = {
  root: string
  branch: string | null
  upstream: string | null
  ahead: number
  behind: number
  detached: boolean
  merging: boolean
  entries: GitStatusEntry[]
  truncated: boolean
}
export type GitDiffText = { before: string; after: string; binary: boolean }
export type GitBranch = { name: string; remote: boolean; current: boolean; upstream: string | null }
export type MergeOutcome = { kind: 'fast_forward' | 'merged' | 'conflicts' | 'up_to_date'; conflicts: string[] }
export type GitRemoteOp = 'fetch' | 'pull' | 'push'
/** Payload degli eventi `git:changed`, `git:progress` e `git:op_done`. */
export type GitChangedEvent = { repo: string }
export type GitProgressEvent = { id: number; line: string }
export type GitOpDoneEvent = { id: number; ok: boolean; message: string }
export type GitCommit = { hash: string; short: string; subject: string; author: string; dateIso: string }

// ---- GitHub (specchio di src-tauri/src/github.rs) ----
export type GhAccount = { login: string; active: boolean }
/** Remoto `origin`: se esiste e il suo `proprietario/nome` se è su github.com. */
export type GhRemote = { origin: boolean; github: string | null }
export type RemoteRepo = { nameWithOwner: string; description: string; private: boolean; updatedAt: string }
export type ChecksSummary = { passing: number; failing: number; pending: number }
export type PrState = 'open' | 'closed' | 'all'
export type Pr = {
  number: number
  title: string
  author: string
  head: string
  base: string
  state: 'open' | 'closed' | 'merged'
  isDraft: boolean
  url: string
  checks: ChecksSummary
}
export type PrCheck = { name: string; state: 'passing' | 'failing' | 'pending' | 'skipped'; url: string }
export type MergeMethod = 'merge' | 'squash' | 'rebase'

// ---- Language server (specchio di src-tauri/src/lsp/mod.rs) ----
export type LspLang = 'python' | 'java' | 'go' | 'rust'
export type LspStart = { id: number; rootUri: string }
/** Payload degli eventi `lsp:message` (messaggio JSON-RPC del server, come testo) e `lsp:exit` (uscita inattesa). */
export type LspMessageEvent = { id: number; message: string }
export type LspExitEvent = { id: number; code: number }

/** Branch mostrato nella barra del titolo e repository a cui appartiene. */
export type ProjectBranch = { repo: string; name: string }

// ---- Database (specchio di src-tauri/src/db/mod.rs) ----
export type DbKind = 'postgres' | 'mongo' | 'oracle'
export type DbTls = 'prefer' | 'require' | 'disable'
export type DbConnection = {
  id: string
  kind: DbKind
  name: string
  host: string
  port: number
  user: string | null
  database: string | null
  tls: DbTls
  acceptInvalidCerts: boolean
  /** Server SSH (salvato o alias di `~/.ssh/config`) da cui passa il tunnel. */
  sshServerId: string | null
  hasPassword: boolean
}
/** `id` assente = nuova connessione; `port` assente = quella predefinita del tipo (5432 / 27017). */
export type DbConnectionInput = Omit<DbConnection, 'id' | 'port' | 'hasPassword'> & { id?: string; port?: number | null }
export type OracleClientStatus = { dir: string | null; loaded: boolean; detected: string[]; error: string | null }
export type DbHistoryEntry = { id: number; text: string; atMs: number }
/** Evento `db:state` e voce di `db_states` (come `SshStateEvent`). */
export type DbStateEvent = { id: string; state: 'connecting' | 'connected' | 'disconnected' | 'error'; message?: string }

// ---- SSH (specchio di src-tauri/src/ssh/mod.rs) ----
export type ServerSource = 'saved' | 'config'
export type Server = {
  id: string
  name: string
  host: string
  user: string | null
  port: number | null
  identityFile: string | null
  startDir: string | null
  source: ServerSource
  hasPassword: boolean
}
export type SshConnState = 'connecting' | 'connected' | 'disconnected' | 'error'
/** Voce di una cartella sul server; un link a una cartella ha `isDir` e `isLink`. */
export type RemoteEntry = { name: string; path: string; isDir: boolean; isLink: boolean }
/** Cartelle prima, poi per nome; `truncated` oltre 10 000 voci. */
export type RemoteListing = { entries: RemoteEntry[]; truncated: boolean }
/** `version` = `size:mtime` del file, da ridare a `sshWrite`. */
export type RemoteContent = { text: string; version: string }
/** Evento `ssh:state` e voce di `sshStates` (specchio di `ssh::conn::StateEvent`). */
export type SshStateEvent = { id: string; state: SshConnState; message?: string }
/** Evento `ssh:transfer` (specchio di `ssh::transfer::TransferEvent`). `errors` e `message` solo nell'ultimo
 *  (stato diverso da `running`); i file già completati restano a destinazione anche se `cancelled`. */
export type TransferState = 'running' | 'done' | 'cancelled' | 'error'
export type TransferError = { path: string; message: string }
export type TransferEvent = {
  transferId: string
  done: number
  total: number
  file: string
  state: TransferState
  errors?: TransferError[]
  message?: string
}
/** Campi modificabili di un server salvato; `id` assente = nuovo server. */
export type ServerInput = {
  id?: string | null
  name: string
  host: string
  user?: string | null
  port?: number | null
  identityFile?: string | null
  startDir?: string | null
}

// ---- Postgres e Oracle: albero e risultati (Oracle riusa queste forme: `schemas` = proprietari, `editableTable.schema` = proprietario) ----
export type PgTable = { name: string; kind: 'table' | 'view' }
export type PgSchema = { name: string; tables: PgTable[] }
export type PgTree = { schemas: PgSchema[] }
export type PgColumn = { name: string; type: string; nullable: boolean; isPk: boolean }
/** Colonna e valore testuale (`null` = NULL, distinto da ''); i tipi li decide il core dal catalogo, mai la WebView. */
export type PgCell = [column: string, value: string | null]
export type PgRowEdit =
  | { kind: 'update'; pk: PgCell[]; set: PgCell[] }
  | { kind: 'insert'; values: PgCell[] }
  | { kind: 'delete'; pk: PgCell[] }
/** Il protocollo semplice non dà il tipo delle colonne: solo il nome. */
export type DbQueryColumn = { name: string }
/**
 * Una pagina di righe: valori già testo, `null` = NULL (distinto da ''). Le celle oltre 64 KB arrivano troncate e
 * sono elencate in `truncated` come coppie `[riga della pagina, colonna]`.
 */
export type DbQueryPage = {
  rows: (string | null)[][]
  hasMore: boolean
  truncated?: [number, number][]
  notices?: string[]
  /**
   * Solo MongoDB: il documento intero (JSON esteso canonico, `null` oltre 1 MB: non modificabile) e l'`_id` (JSON esteso
   * canonico, testo opaco da ridare così com'è al core: mai `JSON.parse`) di ogni riga, e le colonne di questa pagina.
   */
  docs?: (string | null)[]
  ids?: (string | null)[]
  columns?: DbQueryColumn[]
}
/** Tabella letta da una query modificabile in griglia (nomi esatti del catalogo), decisa dal core. */
export type DbEditableTable = { schema: string; table: string }
/**
 * `resultId` c'è per ogni risultato con righe (tenuto dal core, anche finito: serve a `dbExport`) fino a `dbQueryClose` o a
 * una nuova query della connessione (anche da un'altra scheda: un risultato per connessione); `affected` = conteggio di ogni
 * istruzione (senza cursore); `editableTable` solo per una lettura semplice di una tabella (`SELECT * FROM t [WHERE …]`).
 */
export type DbQueryResult = DbQueryPage & { resultId?: number; columns: DbQueryColumn[]; affected?: number[]; editableTable?: DbEditableTable | null }

/** Esito di `dbExport`: percorso scelto nel selettore nativo e righe scritte (il risultato è poi chiuso). */
export type DbExported = { path: string; rows: number }

/** Un database MongoDB con le sue collezioni (in ordine alfabetico). */
export type MongoDb = { database: string; collections: string[] }
