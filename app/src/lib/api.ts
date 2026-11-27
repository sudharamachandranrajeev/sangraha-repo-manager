import { invoke as tauriInvoke } from '@tauri-apps/api/core'

/**
 * Outside the Sangraha window (a plain browser on `npm run dev`), answers come from
 * `app/demo/board.json`: made-up repositories, for screenshots and layout work.
 */
const inTauri = '__TAURI_INTERNALS__' in window

async function demo<T>(cmd: string): Promise<T> {
  if (cmd === 'board' || cmd === 'refresh') {
    const b = (await (await fetch('/demo/board.json')).json()) as Board
    b.checkedAt = Math.floor(Date.now() / 1000) - 180
    return b as T
  }
  if (cmd === 'settings' || cmd === 'set_roots' || cmd === 'set_hidden') return { roots: ['/home/you/Work'], hidden: [] } as T
  if (cmd === 'version') return 'demo' as T
  return undefined as T
}

function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  return inTauri ? tauriInvoke<T>(cmd, args) : demo<T>(cmd)
}

export type Level = 'info' | 'low' | 'medium' | 'high'
export type Ci = 'passing' | 'failing' | 'running' | 'none'
export type ProblemKind =
  | 'ciFailing'
  | 'alerts'
  | 'unpushed'
  | 'uncommitted'
  | 'unreadable'
  | 'localOnly'
  | 'behind'
  | 'untracked'
  | 'stashes'
  | 'detached'
  | 'oldRemoteName'
  | 'notCloned'

export interface Problem {
  kind: ProblemKind
  level: Level
  text: string
  path: string | null
}

export type Action =
  | { kind: 'openUrl'; label: string; url: string }
  | { kind: 'openTerminal'; label: string; path: string }
  | { kind: 'openFolder'; label: string; path: string }

export interface Release {
  tag: string
  prerelease: boolean
  draft: boolean
  publishedAt: string | null
  url: string
}

export interface Alerts {
  total: number
  critical: number
  high: number
  moderate: number
  low: number
}

export interface LocalClone {
  path: string
  github: string | null
  otherRemote: string | null
  branch: string | null
  upstream: string | null
  ahead: number
  behind: number
  unpushed: number
  changed: number
  untracked: number
  stashes: number
  worktree: boolean
  error: string | null
}

export interface Row {
  key: string
  name: string
  fullName: string | null
  url: string | null
  description: string | null
  private: boolean
  archived: boolean
  fork: boolean
  language: string | null
  pushedAt: string | null
  defaultBranch: string | null
  ci: Ci
  release: Release | null
  alerts: Alerts | null
  openIssues: number
  openPrs: number
  foreign: boolean
  clones: LocalClone[]
  problems: Problem[]
  action: Action | null
  score: number
  hidden: boolean
}

export interface Counts {
  all: number
  attention: number
  unpushed: number
  ciRed: number
  alerts: number
  notCloned: number
  localOnly: number
  hidden: number
}

export interface Rate {
  limit: number
  remaining: number
  resetAt: string
}

export interface Board {
  login: string
  checkedAt: number | null
  rate: Rate | null
  platform: string
  roots: string[]
  rows: Row[]
  counts: Counts
  warnings: string[]
}

export interface Config {
  roots: string[]
  hidden: string[]
}

export const api = {
  board: () => invoke<Board>('board'),
  refresh: (fetch: boolean) => invoke<Board>('refresh', { fetch }),
  settings: () => invoke<Config>('settings'),
  setRoots: (roots: string[]) => invoke<Config>('set_roots', { roots }),
  setHidden: (key: string, hidden: boolean) => invoke<Config>('set_hidden', { key, hidden }),
  openUrl: (url: string) => invoke<void>('open_url', { url }),
  openFolder: (path: string) => invoke<void>('open_folder', { path }),
  openTerminal: (path: string) => invoke<void>('open_terminal', { path }),
  version: () => invoke<string>('version'),
}

export function runAction(a: Action): Promise<void> {
  switch (a.kind) {
    case 'openUrl':
      return api.openUrl(a.url)
    case 'openTerminal':
      return api.openTerminal(a.path)
    case 'openFolder':
      return api.openFolder(a.path)
  }
}
