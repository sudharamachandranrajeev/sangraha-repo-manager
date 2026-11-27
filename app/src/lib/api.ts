import { invoke as tauriInvoke } from '@tauri-apps/api/core'

/**
 * Outside the Sangraha window (a plain browser on `npm run dev`), answers come from
 * `app/demo/board.json`: made-up repositories, for screenshots and layout work.
 */
const inTauri = '__TAURI_INTERNALS__' in window

async function demo<T>(cmd: string): Promise<T> {
  if (cmd === 'board' || cmd === 'refresh' || cmd.endsWith('_board') || cmd.endsWith('_refresh')) {
    const file = cmd.startsWith('apps') ? 'apps' : cmd.startsWith('security') ? 'security' : 'board'
    const b = (await (await fetch(`/demo/${file}.json`)).json()) as { checkedAt: number | null }
    b.checkedAt = Math.floor(Date.now() / 1000) - 180
    return b as T
  }
  if (cmd === 'set_hook') return 'Demo: nothing was changed' as T
  if (cmd.startsWith('set') || cmd.startsWith('save') || cmd === 'settings')
    return {
      roots: ['/home/you/Work'],
      hidden: [],
      appsDir: null,
      extraApps: ['someone/their-app'],
      hiddenApps: [],
      prereleases: true,
      delayDays: 0,
      assetRules: {},
      termsFile: null,
      allowedEmails: [],
      requireSigned: false,
    } as T
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
  appsDir: string | null
  extraApps: string[]
  hiddenApps: string[]
  prereleases: boolean
  delayDays: number
  assetRules: Record<string, string>
  termsFile: string | null
  allowedEmails: string[]
  requireSigned: boolean
}

export type Severity = 'low' | 'moderate' | 'high' | 'critical'
export type Scope = 'runtime' | 'development' | 'unknown'

export interface SecurityAlert {
  repo: string
  number: number
  package: string
  ecosystem: string
  severity: Severity
  scope: Scope
  manifest: string | null
  requirement: string | null
  patched: string | null
  advisory: string
  summary: string
  url: string
}

export interface PackageGroup {
  ecosystem: string
  package: string
  severity: Severity
  shipped: boolean
  repos: string[]
  fix: string | null
  unfixed: number
  alerts: SecurityAlert[]
}

export interface RepoScan {
  repo: string
  secrets: number | null
  code: number | null
  codeSerious: number
}

export type HookState = 'none' | 'sangraha' | 'other' | 'shared'

export interface HookRow {
  path: string
  github: string | null
  state: HookState
}

export interface SecurityBoard {
  checkedAt: number | null
  rate: { limit: number; remaining: number; resetAt: string } | null
  groups: PackageGroup[]
  scans: RepoScan[]
  counts: {
    critical: number
    high: number
    moderate: number
    low: number
    shipped: number
    development: number
    secrets: number
    code: number
    secretsOff: number
    codeOff: number
  }
  hooks: HookRow[]
  cli: string | null
  warnings: string[]
}

export type AppStatus = 'update' | 'unknown' | 'waiting' | 'upToDate' | 'notInstalled' | 'noBuild'
export type Kind = 'archive' | 'appImage' | 'exe' | 'setup' | 'systemPackage' | 'apk' | 'mac'

export interface Offer {
  tag: string
  prerelease: boolean
  publishedAt: string | null
  url: string
  asset: string
  size: number
  kind: Kind
  checksum: boolean
}

export interface Here {
  version: string | null
  path: string
  launcher: string | null
  bySangraha: boolean
  previous: string | null
}

export interface AppRow {
  key: string
  name: string
  repo: string
  description: string | null
  url: string
  own: boolean
  status: AppStatus
  offer: Offer | null
  newest: string | null
  here: Here | null
  android: string | null
  notes: string[]
  hidden: boolean
}

export interface AppCounts {
  all: number
  installed: number
  updates: number
  notInstalled: number
  hidden: number
}

export interface AppsBoard {
  checkedAt: number | null
  rate: Rate | null
  appsDir: string
  rows: AppRow[]
  counts: AppCounts
  warnings: string[]
}

export interface InstallFailure {
  unverified: boolean
  message: string
}

export interface InstallProgress {
  key: string
  done: number
  total: number
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
  appsBoard: () => invoke<AppsBoard>('apps_board'),
  appsRefresh: () => invoke<AppsBoard>('apps_refresh'),
  installApp: (key: string, allowUnverified: boolean) => invoke<string>('install_app', { key, allowUnverified }),
  rollbackApp: (key: string) => invoke<string>('rollback_app', { key }),
  launchApp: (key: string) => invoke<void>('launch_app', { key }),
  setAppHidden: (key: string, hidden: boolean) => invoke<Config>('set_app_hidden', { key, hidden }),
  saveAppsSettings: (s: { appsDir: string; prereleases: boolean; delayDays: number; extraApps: string[]; rules: string[] }) =>
    invoke<Config>('save_apps_settings', s),
  securityBoard: () => invoke<SecurityBoard>('security_board'),
  securityRefresh: () => invoke<SecurityBoard>('security_refresh'),
  setHook: (path: string, on: boolean) => invoke<string>('set_hook', { path, on }),
  saveLeakSettings: (s: { termsFile: string; allowedEmails: string[]; requireSigned: boolean }) =>
    invoke<Config>('save_leak_settings', s),
}

/** Download progress while an app installs. Does nothing outside the Sangraha window. */
export async function onInstallProgress(f: (p: InstallProgress) => void): Promise<() => void> {
  if (!inTauri) return () => {}
  const { listen } = await import('@tauri-apps/api/event')
  return listen<InstallProgress>('install-progress', (e) => f(e.payload))
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
