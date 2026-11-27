<script lang="ts">
  import type { Row } from './api'
  import { kindLabel, plural, shortDate, tidyPath, toneOf } from './format'

  let { row, roots }: { row: Row; roots: string[] } = $props()
</script>

<div class="detail">
  {#if row.problems.length}
    <ul class="problems">
      {#each row.problems as p, i (i)}
        <li>
          <span class="chip caps tone-{toneOf(p.level, p.kind)}">{kindLabel(p.kind, row)}</span>
          <span>{p.text}</span>
        </li>
      {/each}
    </ul>
  {:else}
    <p class="muted">Nothing needs attention.</p>
  {/if}

  {#if row.clones.length}
    <table class="clones">
      <thead class="caps">
        <tr><th>Clone</th><th>Branch</th><th>Ahead / behind</th><th>Not on GitHub</th><th>Changed</th><th>New</th></tr>
      </thead>
      <tbody>
        {#each row.clones as c (c.path)}
          <tr>
            <td title={c.path}>{tidyPath(c.path, roots)}{c.worktree ? ' (worktree)' : ''}</td>
            <td class="mono">{c.branch ?? 'detached'}</td>
            <td class="mono">{c.upstream ? `↑${c.ahead} ↓${c.behind}` : 'no upstream'}</td>
            <td class="mono">{c.unpushed || '—'}</td>
            <td class="mono">{c.changed || '—'}</td>
            <td class="mono">{c.untracked || '—'}</td>
          </tr>
        {/each}
      </tbody>
    </table>
  {/if}

  <p class="facts muted">
    {#if row.fullName}{row.fullName}{/if}
    {#if row.language} · {row.language}{/if}
    {#if row.defaultBranch} · default branch {row.defaultBranch}{/if}
    {#if row.url && !row.foreign} · {plural(row.openIssues, 'open issue', 'open issues')} · {plural(
        row.openPrs,
        'open pull request',
        'open pull requests',
      )}{/if}
    {#if row.release} · {row.release.tag} {row.release.prerelease ? '(pre-release)' : ''} {shortDate(
        row.release.publishedAt,
      )}{/if}
  </p>
</div>

<style>
  .detail {
    padding: 10px 16px 16px 64px;
    display: grid;
    gap: 12px;
  }
  .problems {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    gap: 6px;
  }
  .problems li {
    display: flex;
    gap: 10px;
    align-items: baseline;
  }
  .problems .chip {
    min-width: 120px;
    text-align: center;
  }
  .clones {
    border-collapse: collapse;
    width: 100%;
    max-width: 900px;
  }
  .clones th {
    text-align: left;
    font-weight: 400;
    color: var(--faint);
    padding: 4px 12px 4px 0;
  }
  .clones td {
    padding: 4px 12px 4px 0;
    border-top: 1px solid var(--line);
  }
  .muted {
    color: var(--muted);
    margin: 0;
  }
</style>
