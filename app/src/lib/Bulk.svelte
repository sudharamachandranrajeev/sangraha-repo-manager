<script lang="ts">
  import { onMount } from 'svelte'
  import { api, onBulkProgress, type BulkPlan, type BulkRequest, type Row, type Setting } from './api'
  import { plural } from './format'

  let { rows, onclose }: { rows: Row[]; onclose: () => void } = $props()

  const settingList: { id: Setting; label: string; hint?: string }[] = [
    { id: 'securityAlerts', label: 'Dependabot alerts' },
    { id: 'securityFixes', label: 'Dependabot security updates', hint: 'pull requests that fix alerts' },
    { id: 'secretScanning', label: 'Secret scanning' },
    { id: 'pushProtection', label: 'Push protection', hint: 'GitHub refuses a push with a secret in it' },
    { id: 'deleteBranchOnMerge', label: 'Delete branch after merge' },
    { id: 'autoMerge', label: 'Allow auto-merge' },
    { id: 'squashMerge', label: 'Allow squash merging' },
    { id: 'mergeCommit', label: 'Allow merge commits' },
    { id: 'rebaseMerge', label: 'Allow rebase merging' },
  ]

  // The owner's own repositories that GitHub still lets change; forks follow their upstream.
  const repos = $derived(rows.filter((r) => r.fullName && !r.foreign && !r.archived && !r.fork))
  let chosen = $state<Record<string, boolean>>({})
  let query = $state('')
  let settings = $state<Record<string, '' | 'on' | 'off'>>({})
  let addTopics = $state('')
  let removeTopics = $state('')
  let dependabot = $state(false)
  let copies = $state<{ repo: string; path: string }[]>([])
  let replace = $state(false)
  let pullRequest = $state(true)
  let message = $state('')
  let plan = $state<BulkPlan | null>(null)
  let working = $state<'' | 'plan' | 'apply'>('')
  let current = $state('')
  let error = $state('')
  let confirming = $state(false)

  const shown = $derived.by(() => {
    const q = query.trim().toLowerCase()
    return repos.filter((r) => !q || r.name.toLowerCase().includes(q))
  })
  const picked = $derived(repos.filter((r) => chosen[r.fullName!]).map((r) => r.fullName!))

  function pick(which: 'shown' | 'public' | 'none') {
    for (const r of repos) chosen[r.fullName!] = which === 'none' ? false : which === 'public' ? !r.private : chosen[r.fullName!] || shown.includes(r)
  }

  function words(s: string): string[] {
    return s
      .split(/[\s,]+/)
      .map((w) => w.trim().toLowerCase())
      .filter(Boolean)
  }

  function request(): BulkRequest {
    const set: BulkRequest['settings'] = {}
    for (const s of settingList) if (settings[s.id]) set[s.id] = settings[s.id] === 'on'
    const files: BulkRequest['files'] = []
    if (dependabot) files.push({ path: '.github/dependabot.yml', source: { kind: 'dependabot' }, replace })
    for (const c of copies) {
      if (c.repo.trim() && c.path.trim()) files.push({ path: c.path.trim(), source: { kind: 'copy', repo: c.repo.trim(), path: c.path.trim() }, replace })
    }
    return { repos: picked, settings: set, addTopics: words(addTopics), removeTopics: words(removeTopics), files, pullRequest, message }
  }

  async function preview() {
    error = ''
    working = 'plan'
    try {
      plan = await api.bulkPlan(request())
    } catch (e) {
      error = String(e)
    } finally {
      working = ''
    }
  }

  async function apply() {
    confirming = false
    error = ''
    working = 'apply'
    try {
      plan = await api.bulkApply(request())
    } catch (e) {
      error = String(e)
    } finally {
      working = ''
      current = ''
    }
  }

  const failed = $derived(plan?.repos.filter((r) => r.error).length ?? 0)
  const touched = $derived(plan?.repos.filter((r) => r.steps.some((s) => s.change)).length ?? 0)

  onMount(() => {
    let stop = () => {}
    onBulkProgress((r) => (current = r)).then((s) => (stop = s))
    return () => stop()
  })
</script>

<svelte:window onkeydown={(e) => e.key === 'Escape' && !working && (confirming ? (confirming = false) : onclose())} />

<div class="scrim side" role="presentation" onpointerdown={(e) => e.target === e.currentTarget && !working && onclose()}>
  <div class="sheet" role="dialog" aria-modal="true" aria-labelledby="bulk-title">
    <header>
      <h2 id="bulk-title">Change many repositories</h2>
      <button class="btn ghost" disabled={!!working} onclick={onclose}>Close</button>
    </header>

    {#if !plan}
      <div class="cols">
        <section class="pickrepos" aria-labelledby="b-repos">
          <h3 id="b-repos" class="caps label">Repositories <span class="muted">{picked.length} of {repos.length}</span></h3>
          <input type="search" placeholder="Search" bind:value={query} aria-label="Search repositories" />
          <div class="quick caps small">
            <button class="linkish caps" onclick={() => pick('shown')}>All shown</button>
            <button class="linkish caps" onclick={() => pick('public')}>Public only</button>
            <button class="linkish caps" onclick={() => pick('none')}>None</button>
          </div>
          <ul class="list">
            {#each shown as r (r.key)}
              <li>
                <label class="check">
                  <input type="checkbox" bind:checked={chosen[r.fullName!]} />
                  <span>{r.name}</span>
                  {#if r.private}<span class="tag caps">private</span>{/if}
                </label>
              </li>
            {/each}
          </ul>
        </section>

        <div class="what">
          <section aria-labelledby="b-set">
            <h3 id="b-set" class="caps label">Settings</h3>
            <div class="settings">
              {#each settingList as s (s.id)}
                <label class="small" for="set-{s.id}">{s.label}{#if s.hint}<span class="muted"> · {s.hint}</span>{/if}</label>
                <select id="set-{s.id}" bind:value={settings[s.id]}>
                  <option value="">Leave as it is</option>
                  <option value="on">On</option>
                  <option value="off">Off</option>
                </select>
              {/each}
            </div>
          </section>

          <section aria-labelledby="b-top">
            <h3 id="b-top" class="caps label">Topics</h3>
            <label class="small" for="t-add">Add</label>
            <input id="t-add" type="text" bind:value={addTopics} placeholder="e.g. tauri, svelte" spellcheck="false" />
            <label class="small" for="t-rm">Remove</label>
            <input id="t-rm" type="text" bind:value={removeTopics} spellcheck="false" />
          </section>

          <section aria-labelledby="b-files">
            <h3 id="b-files" class="caps label">Files</h3>
            <label class="check"
              ><input type="checkbox" bind:checked={dependabot} />
              <span>Dependabot config (<span class="mono">.github/dependabot.yml</span>), made from each repository's own manifests, with minor and patch updates grouped weekly</span></label
            >
            {#each copies as c, i (i)}
              <div class="copy">
                <input type="text" bind:value={c.repo} placeholder="owner/name to copy from" aria-label="Copy from repository" spellcheck="false" />
                <input type="text" bind:value={c.path} placeholder="path, e.g. LICENSE" aria-label="File path" spellcheck="false" />
                <button class="btn ghost" aria-label="Remove this file" onclick={() => copies.splice(i, 1)}>×</button>
              </div>
            {/each}
            <button class="btn ghost start" onclick={() => copies.push({ repo: '', path: '' })}>Copy a file from a repository…</button>
            <label class="check"><input type="checkbox" bind:checked={replace} /> Replace a file that exists and differs</label>
            <fieldset>
              <legend class="small">How files go in</legend>
              <label class="check"><input type="radio" name="mode" value={true} bind:group={pullRequest} /> A pull request in each repository</label>
              <label class="check"><input type="radio" name="mode" value={false} bind:group={pullRequest} /> A commit on the default branch</label>
            </fieldset>
            <label class="small" for="msg">Commit message</label>
            <input id="msg" type="text" bind:value={message} placeholder="Default: Add (file names)" />
            <p class="hint small">Settings and topics change straight away; only files go through the pull request.</p>
          </section>
        </div>
      </div>
    {:else}
      <p class="small" role="status">
        {#if plan.applied}
          Done: {plural(plan.changes, 'change', 'changes')}{failed ? `; ${plural(failed, 'repository', 'repositories')} failed` : ''}.
        {:else}
          Dry run: {plural(plan.changes, 'change', 'changes')} in {plural(touched, 'repository', 'repositories')}. Nothing has changed yet. Files would be
          committed as <span class="mono">{plan.author}</span>.
        {/if}
      </p>
      <ul class="planlist">
        {#each plan.repos as r (r.repo)}
          <li>
            <div class="repo">{r.repo.split('/')[1] ?? r.repo}</div>
            {#if r.error}<p class="error small">{r.error}</p>{/if}
            {#each r.steps as s, i (i)}
              <div class="step small" class:same={!s.change}>
                <span>{s.label}</span>
                {#if s.change}<span class="mono">{s.now} → {s.then}</span>{:else}<span class="muted">{s.note}</span>{/if}
              </div>
            {/each}
            {#if r.pullRequest}<button class="linkish small" onclick={() => api.openUrl(r.pullRequest!)}>Pull request</button>{/if}
          </li>
        {/each}
      </ul>
    {/if}

    {#if error}<p class="error" role="alert">{error}</p>{/if}

    <footer>
      {#if working === 'apply'}
        <span class="small" role="status">Changing {current ? current.split('/')[1] : '…'}</span>
      {/if}
      <span class="spacer"></span>
      {#if !plan}
        <button class="btn caps primary" disabled={!picked.length || !!working} onclick={preview}>{working === 'plan' ? 'Reading…' : 'Preview'}</button>
      {:else if !plan.applied}
        <button class="btn ghost" disabled={!!working} onclick={() => (plan = null)}>Back</button>
        <button class="btn caps primary" disabled={!plan.changes || !!working} onclick={() => (confirming = true)}>Apply {plan.changes}</button>
      {:else}
        <button class="btn ghost" onclick={() => (plan = null)}>Make another change</button>
      {/if}
    </footer>
  </div>
</div>

{#if confirming && plan}
  <div class="scrim" role="presentation">
    <div class="dialog" role="alertdialog" aria-modal="true" aria-labelledby="bc-title" aria-describedby="bc-text">
      <h2 id="bc-title" class="caps">Change {plural(touched, 'repository', 'repositories')} on GitHub</h2>
      <p id="bc-text">
        This makes {plural(plan.changes, 'change', 'changes')} on GitHub. Sangraha reads each repository again first and only changes what still
        differs.
      </p>
      <div class="dlgact">
        <button class="btn ghost" onclick={() => (confirming = false)}>Cancel</button>
        <button class="btn caps primary" onclick={apply}>Apply</button>
      </div>
    </div>
  </div>
{/if}

<style>
  .side {
    display: flex;
    justify-content: flex-end;
    place-items: stretch;
    z-index: 30;
  }
  .sheet {
    width: min(980px, 100%);
    height: 100%;
    display: flex;
    flex-direction: column;
    gap: 16px;
    padding: 20px 24px 0;
    background: var(--surface);
    border-left: 1px solid var(--line-strong);
    overflow: auto;
  }
  header,
  footer {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  footer {
    position: sticky;
    bottom: 0;
    padding: 12px 0;
    margin-top: auto;
    background: var(--surface);
    border-top: 1px solid var(--line);
  }
  h2,
  h3 {
    margin: 0;
    font-weight: 600;
  }
  h2 {
    font-size: 16px;
  }
  .label {
    color: var(--muted);
    font-size: 12px;
  }
  .start {
    justify-self: start;
  }
  .muted {
    color: var(--muted);
  }
  .cols {
    display: grid;
    grid-template-columns: 280px 1fr;
    gap: 24px;
    min-height: 0;
  }
  section {
    display: grid;
    gap: 8px;
    align-content: start;
  }
  .what {
    display: grid;
    gap: 24px;
    align-content: start;
  }
  .quick {
    display: flex;
    gap: 12px;
  }
  .list {
    list-style: none;
    margin: 0;
    padding: 0;
    max-height: calc(100vh - 230px);
    overflow: auto;
    border: 1px solid var(--line);
  }
  .list li {
    padding: 6px 10px;
    border-bottom: 1px solid var(--line);
  }
  .check {
    display: flex;
    gap: 8px;
    align-items: baseline;
  }
  .settings {
    display: grid;
    grid-template-columns: 1fr 170px;
    gap: 6px 12px;
    align-items: center;
  }
  input[type='text'],
  input[type='search'],
  select {
    width: 100%;
    height: 32px;
    padding: 0 8px;
    border: 1px solid var(--line-strong);
    border-radius: 2px;
    background: var(--bg);
    color: var(--text);
    font-size: 13px;
  }
  .copy {
    display: grid;
    grid-template-columns: 1fr 1fr 34px;
    gap: 6px;
  }
  fieldset {
    margin: 0;
    padding: 0;
    border: 0;
    display: grid;
    gap: 4px;
  }
  .hint {
    margin: 0;
    color: var(--muted);
  }
  .error {
    color: var(--bad);
    margin: 0;
  }
  .planlist {
    list-style: none;
    margin: 0;
    padding: 0;
    border-top: 1px solid var(--line);
  }
  .planlist li {
    padding: 10px 0;
    border-bottom: 1px solid var(--line);
    display: grid;
    gap: 4px;
  }
  .repo {
    font-weight: 600;
  }
  .step {
    display: grid;
    grid-template-columns: 260px 1fr;
    gap: 12px;
  }
  .step.same {
    color: var(--muted);
  }
</style>
