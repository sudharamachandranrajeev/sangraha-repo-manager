<script lang="ts">
  import { onMount } from 'svelte'
  import { api, runAction, type Board, type Row } from './lib/api'
  import { ago, chip, folderName, plural, shortDate } from './lib/format'
  import Detail from './lib/Detail.svelte'
  import RowMenu from './lib/RowMenu.svelte'
  import Settings from './lib/Settings.svelte'

  type Filter = 'all' | 'attention' | 'unpushed' | 'ciRed' | 'alerts' | 'notCloned' | 'localOnly' | 'hidden'
  type Sort = 'worst' | 'name' | 'pushed' | 'release'

  let board = $state<Board | null>(null)
  let filter = $state<Filter>('attention')
  let sort = $state<Sort>('worst')
  let view = $state<'table' | 'grid'>(localStorage.getItem('view') === 'grid' ? 'grid' : 'table')
  let query = $state('')
  let busy = $state<'' | 'refresh' | 'fetch'>('')
  let error = $state('')
  let note = $state('')
  let open = $state<string | null>(null)
  let showSettings = $state(false)
  let showWarnings = $state(false)
  let search: HTMLInputElement
  let tick = $state(0)

  const filters: { id: Filter; label: string }[] = [
    { id: 'all', label: 'All' },
    { id: 'attention', label: 'Needs attention' },
    { id: 'unpushed', label: 'Unpushed' },
    { id: 'ciRed', label: 'Checks failing' },
    { id: 'alerts', label: 'Alerts' },
    { id: 'notCloned', label: 'Not cloned' },
    { id: 'localOnly', label: 'Local only' },
  ]

  function matches(r: Row, f: Filter): boolean {
    if (f === 'hidden') return r.hidden
    if (r.hidden) return false
    const has = (k: string) => r.problems.some((p) => p.kind === k)
    switch (f) {
      case 'all':
        return true
      case 'attention':
        return r.problems.some((p) => p.level === 'high' || p.level === 'medium')
      case 'unpushed':
        return has('unpushed')
      case 'ciRed':
        return has('ciFailing')
      case 'alerts':
        return has('alerts')
      case 'notCloned':
        return has('notCloned')
      case 'localOnly':
        return has('localOnly')
    }
  }

  const rows = $derived.by(() => {
    if (!board) return []
    const q = query.trim().toLowerCase()
    const list = board.rows.filter(
      (r) =>
        matches(r, filter) &&
        (!q ||
          r.name.toLowerCase().includes(q) ||
          (r.description ?? '').toLowerCase().includes(q) ||
          r.clones.some((c) => c.path.toLowerCase().includes(q))),
    )
    const by: Record<Sort, (a: Row, b: Row) => number> = {
      worst: () => 0,
      name: (a, b) => a.name.localeCompare(b.name),
      pushed: (a, b) => (b.pushedAt ?? '').localeCompare(a.pushedAt ?? ''),
      release: (a, b) => (b.release?.publishedAt ?? '').localeCompare(a.release?.publishedAt ?? ''),
    }
    return sort === 'worst' ? list : [...list].sort(by[sort])
  })

  function count(f: Filter): number {
    if (!board) return 0
    const c = board.counts
    return { all: c.all, attention: c.attention, unpushed: c.unpushed, ciRed: c.ciRed, alerts: c.alerts, notCloned: c.notCloned, localOnly: c.localOnly, hidden: c.hidden }[f]
  }

  async function load() {
    try {
      board = await api.board()
      if (board.counts.attention === 0 && filter === 'attention') filter = 'all'
    } catch (e) {
      error = String(e)
    }
  }

  async function refresh(fetch = false) {
    if (busy) return
    busy = fetch ? 'fetch' : 'refresh'
    error = ''
    try {
      board = await api.refresh(fetch)
    } catch (e) {
      error = String(e)
    } finally {
      busy = ''
    }
  }

  async function hide(row: Row, hidden: boolean) {
    await api.setHidden(row.key, hidden)
    await load()
    note = hidden ? `${row.name} is hidden. Settings lists hidden rows.` : `${row.name} is back on the board.`
  }

  async function act(row: Row) {
    if (!row.action) return
    try {
      await runAction(row.action)
    } catch (e) {
      error = String(e)
    }
  }

  function localLine(r: Row): string {
    if (!r.clones.length) return ''
    const c = r.clones[0]
    if (c.error) return 'unreadable'
    const parts = [c.branch ?? 'detached']
    if (c.upstream && (c.ahead || c.behind)) parts.push(`↑${c.ahead} ↓${c.behind}`)
    if (c.changed) parts.push(`${c.changed} changed`)
    return parts.join(' · ')
  }

  function onKey(e: KeyboardEvent) {
    const typing = (e.target as HTMLElement).closest('input, textarea, select')
    if (typing || showSettings) return
    if (e.key === '/') {
      e.preventDefault()
      search?.focus()
    } else if (e.key === 'r' && !e.ctrlKey && !e.metaKey) {
      refresh()
    }
  }

  $effect(() => {
    localStorage.setItem('view', view)
  })

  onMount(() => {
    load().then(() => {
      // Read GitHub at start when the last answer is older than ten minutes.
      if (!board?.checkedAt || Date.now() / 1000 - board.checkedAt > 600) refresh()
    })
    const t = setInterval(() => tick++, 30_000)
    return () => clearInterval(t)
  })
</script>

<svelte:window onkeydown={onKey} />

<div class="app">
  <header class="top">
    <div class="brand">
      <img src="/favicon.svg" alt="" width="22" height="22" />
      <span class="caps word">Sangraha</span>
    </div>
    <nav class="tabs caps" aria-label="Section">
      <span class="tab on" aria-current="page">Repos</span>
    </nav>
    <div class="spacer"></div>
    <div class="seg caps" role="group" aria-label="View">
      <button class:on={view === 'table'} aria-pressed={view === 'table'} onclick={() => (view = 'table')}>Table</button>
      <button class:on={view === 'grid'} aria-pressed={view === 'grid'} onclick={() => (view = 'grid')}>Grid</button>
    </div>
    <button class="btn ghost caps" disabled={!!busy} onclick={() => refresh(true)} title="git fetch in every clone, then read GitHub">
      {busy === 'fetch' ? 'Fetching…' : 'Fetch all'}
    </button>
    <button class="btn ghost icon" class:spin={busy === 'refresh'} disabled={!!busy} aria-label="Read GitHub again (R)" title="Read GitHub again (R)" onclick={() => refresh()}
      ><svg viewBox="0 0 16 16" width="15" height="15" aria-hidden="true"
        ><path d="M13.5 8a5.5 5.5 0 1 1-1.6-3.9M13.5 2.5v3h-3" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" /></svg
      ></button>
    <button class="btn ghost icon" aria-label="Settings" onclick={() => (showSettings = true)}
      ><svg viewBox="0 0 16 16" width="15" height="15" aria-hidden="true"
        ><path d="M2.5 4.5h11M2.5 11.5h11" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" /><circle cx="6" cy="4.5" r="1.8" fill="var(--surface)" stroke="currentColor" stroke-width="1.5" /><circle cx="10.5" cy="11.5" r="1.8" fill="var(--surface)" stroke="currentColor" stroke-width="1.5" /></svg
      ></button>
  </header>

  <div class="bar">
    <div class="filters caps" role="tablist" aria-label="Filter">
      {#each filters as f (f.id)}
        <button role="tab" aria-selected={filter === f.id} class:on={filter === f.id} onclick={() => (filter = f.id)}>
          {f.label} <span class="n" class:hot={f.id !== 'all' && f.id !== 'notCloned' && count(f.id) > 0}>{count(f.id)}</span>
        </button>
      {/each}
      {#if board && board.counts.hidden}
        <button role="tab" aria-selected={filter === 'hidden'} class:on={filter === 'hidden'} onclick={() => (filter = 'hidden')}>
          Hidden <span class="n">{board.counts.hidden}</span>
        </button>
      {/if}
    </div>
    <div class="spacer"></div>
    <label class="caps sortlabel" for="sort">Sort</label>
    <select id="sort" bind:value={sort}>
      <option value="worst">Worst first</option>
      <option value="pushed">Recently pushed</option>
      <option value="release">Latest release</option>
      <option value="name">Name</option>
    </select>
    <input bind:this={search} type="search" placeholder="Search repositories  /" bind:value={query} aria-label="Search repositories" />
  </div>

  {#if error}
    <div class="banner bad" role="alert">
      <span>{error}</span>
      <button class="btn ghost" onclick={() => (error = '')}>Dismiss</button>
    </div>
  {/if}
  {#if note}
    <div class="banner" role="status">
      <span>{note}</span>
      <button class="btn ghost" onclick={() => (note = '')}>OK</button>
    </div>
  {/if}

  <main>
    {#if !board}
      <p class="empty">Looking at your clones…</p>
    {:else if !rows.length}
      <p class="empty">
        {query ? 'No repository matches.' : filter === 'attention' ? 'Nothing needs attention.' : 'Nothing here.'}
        {#if !board.checkedAt}<br />GitHub has not been read yet: press R or the refresh button.{/if}
      </p>
    {:else if view === 'table'}
      <table class="board">
        <thead class="caps">
          <tr>
            <th class="name">Repository</th>
            <th>Status</th>
            <th>Local</th>
            <th>Checks</th>
            <th>Release</th>
            <th>Alerts</th>
            <th>Pushed</th>
            <th class="act"><span class="sr">Actions</span></th>
          </tr>
        </thead>
        <tbody>
          {#each rows as r (r.key)}
            {@const c = chip(r)}
            <tr class="row" class:open={open === r.key} onclick={() => (open = open === r.key ? null : r.key)}>
              <td class="name"><div class="namecell">
                <button class="expand" aria-expanded={open === r.key} aria-label="Details for {r.name}" onclick={(e) => (e.stopPropagation(), (open = open === r.key ? null : r.key))}>
                  {open === r.key ? '▾' : '▸'}
                </button>
                <div>
                  <div class="title">
                    {r.name}
                    {#if r.private}<span class="tag caps">private</span>{/if}
                    {#if r.foreign}<span class="tag caps">upstream</span>{/if}
                    {#if r.clones.length > 1}<span class="tag caps">{r.clones.length} clones</span>{/if}
                  </div>
                  <div class="desc">{r.description ?? (r.clones[0] ? folderName(r.clones[0].path) : '')}</div>
                </div>
              </div></td>
              <td>
                <span class="chip caps tone-{c.tone}">{c.label}</span>
                {#if r.problems.length > 1}<span class="more mono">+{r.problems.length - 1}</span>{/if}
              </td>
              <td class="mono small">{localLine(r) || '—'}</td>
              <td><span class="dot {r.ci}" aria-hidden="true"></span><span class="small">{{ passing: 'Passing', failing: 'Failing', running: 'Running', none: '—' }[r.ci]}</span></td>
              <td class="mono small">
                {#if r.release}
                  <span class:pre={r.release.prerelease} title={r.release.prerelease ? 'Pre-release' : 'Release'}>{r.release.tag}</span>
                  <div class="desc">{shortDate(r.release.publishedAt)}</div>
                {:else}—{/if}
              </td>
              <td class="mono small">
                {#if r.alerts && r.alerts.total}
                  <span class={r.alerts.critical + r.alerts.high ? 'bad' : 'warn'}>{r.alerts.total}</span>
                  {#if r.alerts.critical + r.alerts.high}<span class="desc"> {r.alerts.critical + r.alerts.high} serious</span>{/if}
                {:else}—{/if}
              </td>
              <td class="small muted" data-tick={tick}>{ago(r.pushedAt)}</td>
              <td class="act" onclick={(e) => e.stopPropagation()}><div class="actcell">
                {#if r.action}
                  <button class="btn caps primary" class:quiet={!r.problems.length || r.problems[0].level === 'info'} onclick={() => act(r)}>{r.action.label}</button>
                {/if}
                <RowMenu row={r} onhide={(h) => hide(r, h)} onerror={(m) => (error = m)} />
              </div></td>
            </tr>
            {#if open === r.key}
              <tr class="detailrow"><td colspan="8"><Detail row={r} roots={board.roots} /></td></tr>
            {/if}
          {/each}
        </tbody>
      </table>
    {:else}
      <div class="grid">
        {#each rows as r (r.key)}
          {@const c = chip(r)}
          <article class="card">
            <div class="cardtop caps">
              <span class="chip tone-{c.tone}">{c.label}</span>
              {#if r.problems.length > 1}<span class="more">+{r.problems.length - 1}</span>{/if}
              <span class="spacer"></span>
              {#if r.private}<span class="tag">private</span>{/if}
            </div>
            <h3>{r.name}</h3>
            <p class="desc clamp">{r.description ?? (r.clones[0] ? folderName(r.clones[0].path) : 'No description')}</p>
            <p class="facts small">
              <span class="dot {r.ci}" aria-hidden="true"></span>{{ passing: 'Checks passing', failing: 'Checks failing', running: 'Checks running', none: 'No checks' }[r.ci]}
              · {r.release ? `${r.release.tag}${r.release.prerelease ? ' (pre)' : ''}` : 'no release'}
              {#if r.clones.length} · <span class="mono">{localLine(r)}</span>{/if}
            </p>
            {#if r.problems[0]}<p class="first small">{r.problems[0].text}</p>{/if}
            <div class="cardact">
              {#if r.action}
                <button class="btn caps primary" class:quiet={!r.problems.length || r.problems[0].level === 'info'} onclick={() => act(r)}>{r.action.label}</button>
              {/if}
              <RowMenu row={r} onhide={(h) => hide(r, h)} onerror={(m) => (error = m)} />
            </div>
          </article>
        {/each}
      </div>
    {/if}
  </main>

  <footer class="foot caps" data-tick={tick}>
    <span>Checked {board?.checkedAt ? ago(board.checkedAt) : 'never'}{board?.login ? ` · ${board.login}` : ''}</span>
    {#if board?.rate}
      <span class:lowbudget={board.rate.remaining < 500}>GitHub budget {board.rate.remaining.toLocaleString()}/{board.rate.limit.toLocaleString()}</span>
    {/if}
    {#if board?.warnings.length}
      <button class="linkish caps" onclick={() => (showWarnings = !showWarnings)}>{plural(board.warnings.length, 'note', 'notes')}</button>
    {/if}
    <span class="spacer"></span>
    <span>{rows.length} shown</span>
    <span>{board?.platform ?? ''}</span>
  </footer>
  {#if showWarnings && board?.warnings.length}
    <ul class="warnings small">
      {#each board.warnings as w, i (i)}<li>{w}</li>{/each}
    </ul>
  {/if}
</div>

{#if showSettings}
  <Settings onclose={() => (showSettings = false)} onchange={load} />
{/if}

<style>
  .app {
    height: 100%;
    display: flex;
    flex-direction: column;
  }
  .spacer {
    flex: 1;
  }
  .top {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 10px 16px;
    border-bottom: 1px solid var(--line);
    background: var(--surface);
  }
  .brand {
    display: flex;
    align-items: center;
    gap: 10px;
    padding-right: 16px;
    border-right: 1px solid var(--line);
  }
  .word {
    font-size: 14px;
    font-weight: 700;
    letter-spacing: 0.16em;
  }
  .tabs {
    display: flex;
    gap: 6px;
  }
  .tab {
    padding: 5px 8px;
    color: var(--muted);
  }
  .tab.on {
    background: var(--ink);
    color: var(--ink-text);
  }
  .seg {
    display: flex;
    border: 1px solid var(--line-strong);
    border-radius: 2px;
  }
  .seg button {
    border: 0;
    background: none;
    padding: 6px 10px;
    color: var(--muted);
  }
  .seg button.on {
    background: var(--ink);
    color: var(--ink-text);
  }
  .spin svg {
    animation: spin 900ms linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }

  .bar {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 12px 16px;
    flex-wrap: wrap;
  }
  .filters {
    display: flex;
    border: 1px solid var(--line-strong);
    border-radius: 2px;
    flex-wrap: wrap;
  }
  .filters button {
    border: 0;
    border-right: 1px solid var(--line);
    background: none;
    padding: 8px 11px;
    color: var(--muted);
  }
  .filters button:last-child {
    border-right: 0;
  }
  .filters button.on {
    background: var(--ink);
    color: var(--ink-text);
  }
  .n {
    margin-left: 4px;
    color: var(--faint);
  }
  .n.hot {
    color: var(--warn);
  }
  .filters button.on .n {
    color: inherit;
    opacity: 0.75;
  }
  .sortlabel {
    color: var(--faint);
  }
  select,
  input[type='search'] {
    height: 34px;
    padding: 0 10px;
    border: 1px solid var(--line-strong);
    border-radius: 2px;
    background: var(--surface);
  }
  input[type='search'] {
    width: 240px;
  }

  .banner {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    margin: 0 16px 10px;
    padding: 8px 8px 8px 14px;
    border: 1px solid var(--line-strong);
    background: var(--surface);
  }
  .banner.bad {
    border-color: var(--bad);
    background: var(--bad-bg);
  }

  main {
    flex: 1;
    overflow: auto;
    padding: 0 16px 16px;
  }
  .empty {
    margin: 60px auto;
    text-align: center;
    color: var(--muted);
  }

  .board {
    width: 100%;
    border-collapse: collapse;
    background: var(--surface);
    border: 1px solid var(--line);
  }
  .board thead th {
    position: sticky;
    top: 0;
    z-index: 1;
    background: var(--surface);
    text-align: left;
    font-weight: 400;
    color: var(--faint);
    padding: 10px 12px;
    border-bottom: 1px solid var(--line);
  }
  .board td {
    white-space: nowrap;
    padding: 10px 12px;
    border-bottom: 1px solid var(--line);
    vertical-align: middle;
  }
  .row {
    cursor: pointer;
  }
  .row:hover td {
    background: var(--surface-2);
  }
  .name {
    min-width: 260px;
    white-space: normal !important;
  }
  .namecell {
    display: flex;
    gap: 8px;
    align-items: flex-start;
  }
  .expand {
    border: 0;
    background: none;
    color: var(--faint);
    width: 18px;
    padding: 2px 0 0;
  }
  .title {
    font-weight: 600;
  }
  .tag {
    margin-left: 6px;
    padding: 1px 5px;
    border: 1px solid var(--line-strong);
    border-radius: 2px;
    color: var(--muted);
    font-size: 10px;
    font-weight: 400;
    vertical-align: 1px;
  }
  .desc {
    color: var(--muted);
    font-size: 12.5px;
    max-width: 380px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .small {
    font-size: 12.5px;
  }
  .muted {
    color: var(--muted);
  }
  .more {
    margin-left: 6px;
    color: var(--faint);
    font-size: 12px;
  }
  .pre::after {
    content: ' pre';
    color: var(--faint);
    font-size: 11px;
  }
  .bad {
    color: var(--bad);
  }
  .warn {
    color: var(--warn);
  }
  .dot {
    display: inline-block;
    width: 8px;
    height: 8px;
    margin-right: 6px;
    border-radius: 50%;
    background: var(--line-strong);
  }
  .dot.passing {
    background: var(--ok);
  }
  .dot.failing {
    background: var(--bad);
  }
  .dot.running {
    background: var(--warn);
  }
  .act {
    text-align: right;
    white-space: nowrap;
  }
  .actcell {
    display: flex;
    justify-content: flex-end;
    gap: 6px;
  }
  .primary {
    min-width: 132px;
  }
  .primary.quiet {
    background: transparent;
    color: var(--text);
    border-color: var(--line-strong);
  }
  .detailrow td {
    background: var(--bg);
    padding: 0;
  }
  .sr {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip: rect(0 0 0 0);
  }

  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(320px, 1fr));
    border-top: 1px solid var(--line);
    border-left: 1px solid var(--line);
  }
  .card {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 14px 16px;
    background: var(--surface);
    border-right: 1px solid var(--line);
    border-bottom: 1px solid var(--line);
  }
  .cardtop {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .card h3 {
    margin: 4px 0 0;
    font-size: 18px;
  }
  .clamp {
    white-space: normal;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    min-height: 2.6em;
    max-width: none;
  }
  .facts,
  .first {
    margin: 0;
    color: var(--muted);
  }
  .first {
    color: var(--text);
  }
  .cardact {
    margin-top: auto;
    display: flex;
    gap: 6px;
  }
  .cardact .primary {
    flex: 1;
  }

  .foot {
    display: flex;
    gap: 16px;
    align-items: center;
    padding: 8px 16px;
    border-top: 1px solid var(--line);
    background: var(--surface);
    color: var(--faint);
  }
  .lowbudget {
    color: var(--bad);
  }
  .linkish {
    border: 0;
    background: none;
    padding: 0;
    color: var(--warn);
    text-decoration: underline;
  }
  .warnings {
    margin: 0;
    padding: 8px 16px 12px 32px;
    max-height: 160px;
    overflow: auto;
    background: var(--surface);
    color: var(--muted);
  }
</style>
