<script lang="ts">
  import { onMount } from 'svelte'
  import { api, type HookRow, type PackageGroup, type SecurityBoard, type Severity } from './api'
  import { ago, folderName, plural } from './format'
  import Menu, { type MenuItem } from './Menu.svelte'

  let { active, busy = $bindable('') }: { active: boolean; busy?: '' | 'refresh' } = $props()

  type Filter = 'all' | 'shipped' | 'development'

  let board = $state<SecurityBoard | null>(null)
  let filter = $state<Filter>('all')
  let query = $state('')
  let error = $state('')
  let note = $state('')
  let search: HTMLInputElement
  let tick = $state(0)
  let working = $state<string | null>(null)
  let preview = $state<HookRow[] | null>(null)
  let showWarnings = $state(false)

  const tone: Record<Severity, string> = { critical: 'bad', high: 'bad', moderate: 'warn', low: 'muted' }

  const groups = $derived.by(() => {
    if (!board) return []
    const q = query.trim().toLowerCase()
    return board.groups.filter(
      (g) =>
        (filter === 'all' || (filter === 'shipped') === g.shipped) &&
        (!q || g.package.toLowerCase().includes(q) || g.repos.some((r) => r.toLowerCase().includes(q))),
    )
  })

  const shippedCount = $derived(board ? board.groups.filter((g) => g.shipped).length : 0)
  const missing = $derived(board ? board.hooks.filter((h) => h.state === 'none') : [])
  const hookRows = $derived.by(() => {
    if (!board) return []
    const order = { none: 0, other: 1, shared: 2, sangraha: 3 }
    const q = query.trim().toLowerCase()
    return board.hooks
      .filter((h) => !q || h.path.toLowerCase().includes(q) || (h.github ?? '').toLowerCase().includes(q))
      .sort((a, b) => order[a.state] - order[b.state] || a.path.localeCompare(b.path))
  })

  /** `name` from `owner/name`: this tab only lists the owner's own repositories. */
  function short(full: string): string {
    return full.split('/')[1] ?? full
  }

  function fix(g: PackageGroup): string {
    if (!g.fix) return 'No fix yet'
    return g.unfixed ? `${g.fix} or later (${g.unfixed} with no fix yet)` : `${g.fix} or later`
  }

  function items(g: PackageGroup): MenuItem[] {
    const list: MenuItem[] = g.repos.map((r) => ({
      label: `Alerts in ${short(r)}`,
      run: () => api.openUrl(`https://github.com/${r}/security/dependabot`),
    }))
    const seen = new Set<string>()
    for (const a of g.alerts) {
      if (!a.advisory || seen.has(a.advisory) || seen.size >= 5) continue
      seen.add(a.advisory)
      list.push({ label: `${a.advisory}: ${a.summary}`, run: () => api.openUrl(`https://github.com/advisories/${a.advisory}`) })
    }
    return list
  }

  const hookLabel: Record<HookRow['state'], { text: string; tone: string }> = {
    none: { text: 'Not checked', tone: 'warn' },
    sangraha: { text: 'Checked', tone: 'ok' },
    other: { text: 'Other hook', tone: 'muted' },
    shared: { text: 'Shared hooks', tone: 'muted' },
  }

  export function focusSearch() {
    search?.focus()
  }

  export async function load() {
    try {
      board = await api.securityBoard()
    } catch (e) {
      error = String(e)
    }
  }

  export async function refresh() {
    if (busy) return
    busy = 'refresh'
    error = ''
    try {
      board = await api.securityRefresh()
    } catch (e) {
      error = String(e)
    } finally {
      busy = ''
    }
  }

  async function setHook(h: HookRow, on: boolean): Promise<boolean> {
    working = h.path
    try {
      note = await api.setHook(h.path, on)
      return true
    } catch (e) {
      error = String(e)
      return false
    } finally {
      working = null
    }
  }

  async function installAll(list: HookRow[]) {
    preview = null
    let done = 0
    for (const h of list) if (await setHook(h, true)) done++
    note = `Leak check installed in ${plural(done, 'clone', 'clones')}.`
    await load()
  }

  // GitHub is read when the tab is first opened (and the last answer is old), not at start.
  let checked = false
  $effect(() => {
    if (!active || checked || !board) return
    checked = true
    if (!board.checkedAt || Date.now() / 1000 - board.checkedAt > 3600) refresh()
  })

  onMount(() => {
    load()
    const t = setInterval(() => tick++, 30_000)
    return () => clearInterval(t)
  })
</script>

<div class="bar">
  <div class="filters caps" role="tablist" aria-label="Filter alerts">
    <button role="tab" aria-selected={filter === 'all'} class:on={filter === 'all'} onclick={() => (filter = 'all')}>
      All <span class="n">{board?.groups.length ?? 0}</span>
    </button>
    <button role="tab" aria-selected={filter === 'shipped'} class:on={filter === 'shipped'} onclick={() => (filter = 'shipped')}>
      Shipped <span class="n" class:hot={shippedCount > 0}>{shippedCount}</span>
    </button>
    <button role="tab" aria-selected={filter === 'development'} class:on={filter === 'development'} onclick={() => (filter = 'development')}>
      Build only <span class="n">{(board?.groups.length ?? 0) - shippedCount}</span>
    </button>
  </div>
  {#if board}
    <p class="tally caps small" aria-label="Open alerts by severity">
      {#each [['critical', board.counts.critical], ['high', board.counts.high], ['moderate', board.counts.moderate], ['low', board.counts.low]] as [name, n] (name)}
        <span class:zero={!n} class="sev-{n ? tone[name as Severity] : 'muted'}">{n} {name}</span>
      {/each}
    </p>
  {/if}
  <div class="spacer"></div>
  <input bind:this={search} type="search" placeholder="Search packages, repos  /" bind:value={query} aria-label="Search packages and repositories" />
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
    <p class="empty">Reading security alerts…</p>
  {:else}
    <section aria-labelledby="sec-pk">
      <h2 id="sec-pk" class="caps label">Security alerts by package</h2>
      <p class="hint small">One upgrade closes every alert in a row. <em>Shipped</em> packages reach the people who use your apps; <em>build only</em> ones do not.</p>
      {#if !board.checkedAt}
        <p class="empty">Alerts have not been read yet: press R or the refresh button.</p>
      {:else if !groups.length}
        <p class="empty">{query ? 'No package matches.' : 'No open security alerts here.'}</p>
      {:else}
        <table class="board">
          <thead class="caps">
            <tr>
              <th class="name">Package</th>
              <th>Worst</th>
              <th>Used in</th>
              <th>Alerts</th>
              <th>Fixed in</th>
              <th class="act"><span class="sr">Actions</span></th>
            </tr>
          </thead>
          <tbody>
            {#each groups as g (g.ecosystem + g.package)}
              <tr>
                <td class="name">
                  <div class="title mono">{g.package}</div>
                  <div class="desc">{g.ecosystem} · {g.shipped ? 'shipped' : 'build only'}</div>
                </td>
                <td><span class="chip caps tone-{tone[g.severity]}">{g.severity}</span></td>
                <td class="small repos">{g.repos.map(short).join(', ')}</td>
                <td class="mono small">{g.alerts.length}</td>
                <td class="mono small">{fix(g)}</td>
                <td class="act"><div class="actcell"><Menu label="Alerts for {g.package}" items={items(g)} onerror={(m) => (error = m)} /></div></td>
              </tr>
            {/each}
          </tbody>
        </table>
      {/if}
    </section>

    {#if board.checkedAt}
      <section aria-labelledby="sec-scan">
        <h2 id="sec-scan" class="caps label">Secret and code scanning</h2>
        {#if board.scans.length}
          <table class="board">
            <thead class="caps">
              <tr><th class="name">Repository</th><th>Secrets</th><th>Code scanning</th><th class="act"><span class="sr">Actions</span></th></tr>
            </thead>
            <tbody>
              {#each board.scans as s (s.repo)}
                <tr>
                  <td class="name"><div class="title">{short(s.repo)}</div></td>
                  <td>{#if s.secrets}<span class="chip caps tone-bad">{plural(s.secrets, 'secret', 'secrets')}</span>{:else}—{/if}</td>
                  <td class="small">{s.code ?? '—'}{#if s.codeSerious}{' · '}<span class="sev-bad">{s.codeSerious} high or critical</span>{/if}</td>
                  <td class="act">
                    <div class="actcell">
                      <button class="btn ghost caps" onclick={() => api.openUrl(`https://github.com/${s.repo}/security/${s.secrets ? 'secret-scanning' : 'code-scanning'}`)}>Open</button>
                    </div>
                  </td>
                </tr>
              {/each}
            </tbody>
          </table>
        {:else}
          <p class="hint small">No open secret or code scanning alerts.</p>
        {/if}
        <p class="hint small">
          Secret scanning is off or not offered in {plural(board.counts.secretsOff, 'repository', 'repositories')};
          code scanning is not set up in {board.counts.codeOff}.
        </p>
      </section>
    {/if}

    <section aria-labelledby="sec-hook">
      <div class="sechead">
        <h2 id="sec-hook" class="caps label">Leak check before every push</h2>
        <span class="spacer"></span>
        <button class="btn caps" disabled={!missing.length || !board.cli || !!working} onclick={() => (preview = missing)}>Add to all ({missing.length})</button>
      </div>
      <p class="hint small">
        A git hook that reads each push before it leaves: private emails, paths into your home folder, words on your private list and anything
        that looks like a password stop it. Pushing a tag also runs the release gate. A hook already there keeps running after it.
      </p>
      {#if !board.cli}
        <p class="banner bad small">The <span class="mono">sangraha</span> command line program was not found, and the hook needs it. Install it (README, "Just the command line"), then reopen this tab.</p>
      {/if}
      <table class="board">
        <thead class="caps">
          <tr><th class="name">Clone</th><th>GitHub</th><th>Leak check</th><th class="act"><span class="sr">Actions</span></th></tr>
        </thead>
        <tbody>
          {#each hookRows as h (h.path)}
            {@const l = hookLabel[h.state]}
            <tr>
              <td class="name"><div class="title">{folderName(h.path)}</div><div class="desc mono">{h.path}</div></td>
              <td class="small">{h.github ? short(h.github) : '—'}</td>
              <td><span class="chip caps tone-{l.tone}">{l.text}</span></td>
              <td class="act">
                <div class="actcell">
                  {#if h.state === 'none' || h.state === 'other'}
                    <button class="btn ghost caps" disabled={!board.cli || !!working} onclick={async () => (await setHook(h, true), load())}>Add</button>
                  {:else if h.state === 'sangraha'}
                    <button class="btn ghost caps" disabled={!!working} onclick={async () => (await setHook(h, false), load())}>Remove</button>
                  {/if}
                </div>
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    </section>
  {/if}
</main>

<footer class="foot caps" data-tick={tick}>
  <span>Checked {board?.checkedAt ? ago(board.checkedAt) : 'never'}</span>
  {#if board?.rate}
    <span class:lowbudget={board.rate.remaining < 500}>GitHub budget {board.rate.remaining.toLocaleString()}/{board.rate.limit.toLocaleString()}</span>
  {/if}
  {#if board?.warnings.length}
    <button class="linkish caps" onclick={() => (showWarnings = !showWarnings)}>{plural(board.warnings.length, 'note', 'notes')}</button>
  {/if}
</footer>
{#if showWarnings && board?.warnings.length}
  <ul class="warnings small">
    {#each board.warnings as w, i (i)}<li>{w}</li>{/each}
  </ul>
{/if}

{#if preview}
  <div class="scrim" role="presentation" onpointerdown={(e) => e.target === e.currentTarget && (preview = null)}>
    <div class="dialog" role="dialog" aria-modal="true" aria-labelledby="hk-title">
      <h2 id="hk-title" class="caps">Add the leak check to {plural(preview.length, 'clone', 'clones')}</h2>
      <p class="small">Each gets a <span class="mono">pre-push</span> hook in its <span class="mono">.git/hooks</span> folder. Nothing in the clone's files changes. Remove takes it out again.</p>
      <ul class="plan">
        {#each preview as h (h.path)}<li class="mono small">{h.path}</li>{/each}
      </ul>
      <div class="dlgact">
        <button class="btn ghost" onclick={() => (preview = null)}>Cancel</button>
        <button class="btn caps primary" onclick={() => installAll(preview ?? [])}>Add to {preview.length}</button>
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
  .label {
    color: var(--muted);
  }
  .sechead {
    display: flex;
    align-items: center;
    gap: 12px;
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
  .sev-bad {
    color: var(--bad);
  }
  .sev-warn {
    color: var(--warn);
  }
  .sev-muted,
  .zero {
    color: var(--faint);
  }
  .repos {
    max-width: 420px;
  }
</style>
