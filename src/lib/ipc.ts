import { invoke } from '@tauri-apps/api/core'
import type { Project } from './types'

export const projectCurrent = () => invoke<Project>('project_current')
export const projectOpenFolder = (path: string) => invoke<Project>('project_open_folder', { path })
export const projectPickAndOpen = () => invoke<void>('project_pick_and_open')
export const projectRename = (id: string, name: string) => invoke<Project>('project_rename', { id, name })
export const projectAddFolder = (id: string, path: string) => invoke<Project>('project_add_folder', { id, path })
export const projectRelocate = (id: string, old: string, newPath: string) =>
  invoke<Project>('project_relocate', { id, old, new: newPath })
export const projectMissingFolders = (id: string) => invoke<string[]>('project_missing_folders', { id })
export const projectBranch = (id: string) => invoke<string | null>('project_branch', { id })
export const settingsGet = (key: string) => invoke<string | null>('settings_get', { key })
export const settingsSet = (key: string, value: string) => invoke<void>('settings_set', { key, value })

// ---- file system ----
import type { DirEntry, FileContent, FsError } from './types'
export const fsList = (path: string) => invoke<DirEntry[]>('fs_list', { path })
export const fsCreate = (path: string, isDir: boolean) => invoke<void>('fs_create', { path, isDir })
export const fsRename = (from: string, to: string) => invoke<void>('fs_rename', { from, to })
export const fsTrash = (path: string) => invoke<void>('fs_trash', { path })
export const fsRead = (path: string) => invoke<FileContent>('fs_read', { path })
export const fsWrite = (path: string, text: string, expectedMtimeMs: number | null) =>
  invoke<number>('fs_write', { path, text, expectedMtimeMs })

/** Normalizza un errore di comando in `FsError`. */
export function asFsError(e: unknown): FsError {
  if (e && typeof e === 'object' && 'code' in e) return e as FsError
  return { code: 'io', message: String(e) }
}

// ---- terminale ----
export const ptySpawn = (cwd: string, cols: number, rows: number) => invoke<number>('pty_spawn', { cwd, cols, rows })
export const ptyWrite = (id: number, data: string) => invoke<void>('pty_write', { id, data })
export const ptyResize = (id: number, cols: number, rows: number) => invoke<void>('pty_resize', { id, cols, rows })
export const ptyKill = (id: number) => invoke<void>('pty_kill', { id })
