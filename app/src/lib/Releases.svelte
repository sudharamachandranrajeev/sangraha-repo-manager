<script lang="ts">
  import { onMount } from 'svelte'
  import { api, type ReleaseBoard, type ReleaseDetails, type ReleaseRow, type Subtree, type Verdict } from './api'
  import { ago, plural, size } from './format'

  let { active, busy = $bindable('') }: { active: boolean; busy?: '' | 'refresh' } = $props()

  type Filter = 'attention' | 'all'

  let board = $state<ReleaseBoard | null>(null)
  let filter = $state<Filter>('attention')
  let query = $state('')
  let error = $state('')
  let note = $state('')
  let search: HTMLInputElement
  let tick = $state(0)
  let showWarnings = $state(false)

  // The row opened in the release sheet, with what is read for it.
  let open = $state<ReleaseRow | null>(null)
  let draft = $state('')
  let details = $state<ReleaseDetails | null>(null)
  let detailsError = $state('')
  let copied = $state(false)

  let pulling = $state<Subtree | null>(null)
  let working = $state('')

  const label: Record<Verdict, { text: string; tone: string }> = {
    disagree: { text: 'Versions differ', tone: 'bad' },
    needsBump: { text: 'Raise version', tone: 'warn' },
    ready: { text: 'Ready to tag', tone: 'ok' },
    noTag: { text: 'Never tagged', tone: 'muted' },
    noVersion: { text: 'No version file', tone: 'muted' },
    noChanges: { text: 'Nothing new', tone: 'muted' },
  }

  const needsAttention = (r: ReleaseRow) => r.unreleasedTag || r.verdict === 'disagree' || r.verdict === 'needsBump' || r.verdict === 'ready'

  const rows = $derived.by(() => {
    if (!board) return []
    const q = query.trim().toLowerCase()
    return board.rows.filter((r) => (filter === 'all' || needsAttention(r)) && (!q || r.repo.toLowerCase().includes(q)))
  })
  const attention = $derived(board ? board.rows.filter(needsAttention).length : 0)
  const subtrees = $derived.by(() => {
    if (!board) return []
    const q = query.trim().toLowerCase()
    return board.subtrees.filter((s) => !q || `${s.host} ${s.dir} ${s.source ?? ''}`.toLowerCase().includes(q))
  })

  function short(full: string): string {
    return full.split('/')[1] ?? full
  }

  function subtreeState(s: Subtree): { text: string; tone: string } {
    if (s.state === 'current') return { text: 'Up to date', tone: 'ok' }
    if (s.state === 'behind') return { text: `${s.behind} behind`, tone: 'warn' }
    return { text: 'Not compared', tone: 'muted' }
  }

  export function focusSearch() {
    search?.focus()
  }

  export async function load() {
    try {
      board = await api.releasesBoard()
    } catch (e) {
      error = String(e)
    }
  }

  export async function refresh() {
    if (busy) return
    busy = 'refresh'
    error = ''
    try {
      board = await api.releasesRefresh()
    } catch (e) {
      error = String(e)
    } finally {
      busy = ''
    }
  }

  async function show(r: ReleaseRow) {
    open = r
    draft = ''
    details = null
    detailsError = ''
    copied = false
    api.changelog(r.path).then(
      (t) => (draft = t),
      (e) => (draft = String(e)),
    )
    if (r.tag && r.tagSha && r.repo.includes('/')) {
      try {
        details = await api.releaseDetails(r.repo, r.tag, r.tagSha)
      } catch (e) {
        detailsError = String(e)
      }
    }
  }

  async function copy() {
    await navigator.clipboard.writeText(draft)
    copied = true
  }

  async function pull(s: Subtree) {
    pulling = null
    working = `${s.host}/${s.dir}`
    try {
      const out = await api.subtreePull(s.path, s.dir)
      note = `${s.dir} in ${short(s.host)} now has the latest ${short(s.source ?? '')}. ${out.split('\n').pop() ?? ''}`
      await refresh()
    } catch (e) {
      error = String(e)
    } finally {
      working = ''
    }
  }

  // GitHub is read when the tab is first opened (and the last answer is old), not at start.
  let checked = false
  $effect(() => {
    if (!active || checked || !board) return
    checked = true
    if (board.subtrees.length && (!board.checkedAt || Date.now() / 1000 - board.checkedAt > 3600)) refresh()
  })

  onMount(() => {
    load()
    const t = setInterval(() => tick++, 30_000)
    return () => clearInterval(t)
  })
</script>

<svelte:window onkeydown={(e) => e.key === 'Escape' && (pulling ? (pulling = null) : (open = null))} />

<div class="bar">
  <div class="filters caps" role="tablist" aria-label="Filter repositories">
    <button role="tab" aria-selected={filter === 'attention'} class:on={filter === 'attention'} onclick={() => (filter = 'attention')}>
      To do <span class="n" class:hot={attention > 0}>{attention}</span>
    </button>
    <button role="tab" aria-selected={filter === 'all'} class:on={filter === 'all'} onclick={() => (filter = 'all')}>
      All <span class="n">{board?.rows.length ?? 0}</span>
    </button>
  </div>
  {#if board}
    {@const c = board.counts}
    <p class="tally caps small" aria-label="Release summary">
      <span class:zero={!c.ready} class="t-ok">{c.ready} ready to tag</span>
      <span class:zero={!c.needsBump} class="t-warn">{c.needsBump} raise version</span>
      <span class:zero={!c.disagree} class="t-bad">{c.disagree} with versions that differ</span>
      <span class:zero={!c.unreleased} class="t-warn">{plural(c.unreleased, 'tag', 'tags')} not released</span>
      <span class:zero={!c.drift} class="t-warn">{plural(c.drift, 'copy', 'copies')} behind or edited</span>
    </p>
  {/if}
  <div class="spacer"></div>
  <input bind:this={search} type="search" placeholder="Search repos  /" bind:value={query} aria-label="Search repositories" />
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
    <p class="empty">Reading your clones…</p>
  {:else}
    <section aria-labelledby="rel-since">
      <h2 id="rel-since" class="caps label">Since the last tag</h2>
      <p class="hint small">
        Read from your clones. <em>Raise version</em> means commits went in after the last tag but the version files still say the tag's version.
        Open a row for a draft changelog and the release's files on GitHub.
      </p>
      {#if !rows.length}
        <p class="empty">{query ? 'No repository matches.' : filter === 'attention' ? 'Nothing waiting for a release.' : 'No clone has a tag or a version file.'}</p>
      {:else}
        <table class="board">
          <thead class="caps">
            <tr>
              <th class="name">Repository</th>
              <th>Last tag</th>
              <th>Since</th>
              <th>Version</th>
              <th>Status</th>
              <th class="act"><span class="sr">Actions</span></th>
            </tr>
          </thead>
          <tbody>
            {#each rows as r (r.path)}
              {@const l = label[r.verdict]}
              <tr>
                <td class="name">
                  <div class="title">{short(r.repo)}</div>
                  <div class="desc">{r.note}</div>
                </td>
                <td class="small">
                  {#if r.tag}<span class="mono">{r.tag}</span><div class="muted">{ago(r.tagDate)}</div>{:else}—{/if}
                </td>
                <td class="small">
                  {plural(r.since, 'commit', 'commits')}{#if r.pullRequests}<div class="muted">{plural(r.pullRequests, 'pull request', 'pull requests')}</div>{/if}
                </td>
                <td class="small mono">{r.version ?? '—'}</td>
                <td>
                  <div class="chips">
                    <span class="chip caps tone-{l.tone}">{l.text}</span>
                    {#if r.unreleasedTag}<span class="chip caps tone-warn" title="The tag is newer than the newest release on GitHub">Tag not released</span>{/if}
                  </div>
                </td>
                <td class="act">
                  <div class="actcell"><button class="btn ghost caps" onclick={() => show(r)}>Open</button></div>
                </td>
              </tr>
            {/each}
          </tbody>
        </table>
      {/if}
    </section>

    {#if board.subtrees.length}
      <section aria-labelledby="rel-sub">
        <h2 id="rel-sub" class="caps label">Copies inside other repositories</h2>
        <p class="hint small">
          Folders added with <span class="mono">git subtree</span>. <em>Behind</em> counts commits on the source that the copy does not have yet. Commits made to
          a copy here never reach its source, and the next pull may conflict with them.
        </p>
        <table class="board">
          <thead class="caps">
            <tr><th class="name">Copy</th><th>Source</th><th>Pulled</th><th>Status</th><th class="act"><span class="sr">Actions</span></th></tr>
          </thead>
          <tbody>
            {#each subtrees as s (s.host + s.dir)}
              {@const st = subtreeState(s)}
              <tr>
                <td class="name">
                  <div class="title mono">{s.dir}</div>
                  <div class="desc">in {short(s.host)}</div>
                </td>
                <td class="small">{s.source ? short(s.source) : '?'} <span class="muted mono">{s.branch}</span></td>
                <td class="small">{ago(s.pulledAt)}</td>
                <td>
                  <div class="chips">
                    <span class="chip caps tone-{st.tone}">{st.text}</span>
                    {#if s.edits.length}<span class="chip caps tone-bad" title={s.edits.join('\n')}>{plural(s.edits.length, 'edit', 'edits')} here</span>{/if}
                  </div>
                  {#if s.note}<div class="desc small">{s.note}</div>{/if}
                </td>
                <td class="act">
                  <div class="actcell">
                    {#if s.state === 'behind' && s.source}
                      <button class="btn ghost caps" disabled={!!working || !!busy} onclick={() => (pulling = s)}
                        >{working === `${s.host}/${s.dir}` ? 'Pulling…' : 'Pull'}</button
                      >
                    {/if}
                  </div>
                </td>
              </tr>
            {/each}
          </tbody>
        </table>
      </section>
    {/if}
  {/if}
</main>

<footer class="foot caps" data-tick={tick}>
  <span>Copies compared {board?.checkedAt ? ago(board.checkedAt) : 'never'}</span>
  {#if board?.warnings.length}
    <button class="linkish caps" onclick={() => (showWarnings = !showWarnings)}>{plural(board.warnings.length, 'note', 'notes')}</button>
  {/if}
</footer>
{#if showWarnings && board?.warnings.length}
  <ul class="warnings small">
    {#each board.warnings as w, i (i)}<li>{w}</li>{/each}
  </ul>
{/if}

{#if open}
  {@const r = open}
  <div class="scrim" role="presentation" onpointerdown={(e) => e.target === e.currentTarget && (open = null)}>
    <div class="dialog wide" role="dialog" aria-modal="true" aria-labelledby="rd-title">
      <h2 id="rd-title" class="caps">{short(r.repo)}</h2>
      <p class="small">{r.note}</p>

      <h3 class="caps label">Version files</h3>
      {#if r.versions.length}
        <ul class="plain small">
          {#each r.versions as v (v.path)}<li><span class="mono">{v.path}</span> <span class="muted">says</span> <span class="mono">{v.version}</span></li>{/each}
        </ul>
      {:else}
        <p class="small muted">None that Sangraha reads (Cargo.toml, package.json, pubspec.yaml, tauri.conf.json).</p>
      {/if}

      <div class="head">
        <h3 class="caps label">Draft changelog</h3>
        <button class="btn ghost caps" disabled={!draft} onclick={copy}>{copied ? 'Copied' : 'Copy'}</button>
      </div>
      <pre class="draft mono small">{draft || 'Reading commits…'}</pre>

      {#if r.tag}
        <h3 class="caps label">{r.tag} on GitHub</h3>
        {#if detailsError}
          <p class="small error">{detailsError}</p>
        {:else if !r.repo.includes('/')}
          <p class="small muted">This clone is not on GitHub.</p>
        {:else if !details}
          <p class="small muted">Reading…</p>
        {:else}
          {#if details.releaseUrl}
            <ul class="plain small">
              {#each details.assets as a (a.name)}<li><span class="mono">{a.name}</span> <span class="muted">{size(a.size)} · {plural(a.downloads, 'download', 'downloads')}</span></li>{/each}
              {#if !details.assets.length}<li class="muted">No files attached to the release.</li>{/if}
            </ul>
            <button class="linkish small" onclick={() => api.openUrl(details!.releaseUrl!)}>Open the release</button>
          {:else}
            <p class="small muted">No GitHub release for this tag.</p>
          {/if}
          {#if details.runs.length}
            <ul class="plain small">
              {#each details.runs as w (w.url)}
                <li>
                  <span class="chip caps tone-{w.conclusion === 'success' ? 'ok' : w.conclusion ? 'bad' : 'warn'}">{w.conclusion ?? w.status.replace('_', ' ')}</span>
                  <button class="linkish" onclick={() => api.openUrl(w.url)}>{w.name}</button>
                </li>
              {/each}
            </ul>
          {/if}
        {/if}
      {/if}

      <div class="dlgact">
        <button class="btn ghost" onclick={() => api.openTerminal(r.path)}>Open terminal</button>
        <button class="btn caps primary" onclick={() => (open = null)}>Close</button>
      </div>
    </div>
  </div>
{/if}

{#if pulling}
  {@const s = pulling}
  <div class="scrim" role="presentation" onpointerdown={(e) => e.target === e.currentTarget && (pulling = null)}>
    <div class="dialog" role="alertdialog" aria-modal="true" aria-labelledby="sp-title" aria-describedby="sp-text">
      <h2 id="sp-title" class="caps">Pull {s.dir} from {short(s.source ?? '')}</h2>
      <div id="sp-text" class="small">
        <p>This merges {plural(s.behind ?? 0, 'commit', 'commits')} from <span class="mono">{s.source}</span> ({s.branch}) into <span class="mono">{s.dir}</span>, as a new commit in your clone. Nothing is pushed.</p>
        <pre class="mono small">git subtree pull --prefix {s.dir} https://github.com/{s.source}.git {s.branch}{s.squash ? ' --squash' : ''}</pre>
        {#if s.edits.length}
          <p class="error">The copy has {plural(s.edits.length, 'commit', 'commits')} made here. If they touch the same lines, git stops with a conflict for you to finish in a terminal.</p>
        {/if}
      </div>
      <div class="dlgact">
        <button class="btn ghost" onclick={() => (pulling = null)}>Cancel</button>
        <button class="btn caps primary" onclick={() => pull(s)}>Pull</button>
      </div>
    </div>
  </div>
{/if}

<style>
  main {
    display: grid;
    gap: 28px;
    align-content: start;
  }
  section {
    display: grid;
    gap: 8px;
  }
  h2 {
    margin: 0;
    font-size: 12px;
    font-weight: 600;
  }
  h3 {
    margin: 12px 0 4px;
    font-size: 11px;
    font-weight: 600;
  }
  .label,
  .muted {
    color: var(--muted);
  }
  .hint {
    margin: 0;
    color: var(--muted);
    max-width: 90ch;
  }
  .tally {
    display: flex;
    gap: 14px;
    margin: 0 0 0 16px;
  }
  .t-ok {
    color: var(--ok);
  }
  .t-warn {
    color: var(--warn);
  }
  .t-bad {
    color: var(--bad);
  }
  .zero {
    color: var(--faint);
  }
  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
  }
  .wide {
    width: min(760px, calc(100vw - 48px));
    max-height: calc(100vh - 48px);
    overflow: auto;
  }
  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  .plain {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    gap: 4px;
  }
  .draft,
  pre {
    margin: 0;
    padding: 10px 12px;
    border: 1px solid var(--line);
    background: var(--bg);
    white-space: pre-wrap;
    max-height: 280px;
    overflow: auto;
  }
  .error {
    color: var(--bad);
  }
</style>
