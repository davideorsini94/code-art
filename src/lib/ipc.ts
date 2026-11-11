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
