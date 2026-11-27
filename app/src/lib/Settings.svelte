<script lang="ts">
  import { onMount } from 'svelte'
  import { api, type Config } from './api'

  let { onclose, onchange }: { onclose: () => void; onchange: () => void } = $props()

  let cfg = $state<Config | null>(null)
  let rootsText = $state('')
  let error = $state('')
  let version = $state('')
  let dialog: HTMLDivElement

  onMount(async () => {
    cfg = await api.settings()
    rootsText = cfg.roots.join('\n')
    version = await api.version()
    ;(dialog.querySelector('textarea') as HTMLTextAreaElement | null)?.focus()
  })

  async function saveRoots() {
    error = ''
    try {
      cfg = await api.setRoots(rootsText.split('\n'))
      onchange()
    } catch (e) {
      error = String(e)
    }
  }

  async function unhide(key: string) {
    cfg = await api.setHidden(key, false)
    onchange()
  }
</script>

<svelte:window onkeydown={(e) => e.key === 'Escape' && onclose()} />

<div class="scrim" role="presentation" onpointerdown={(e) => e.target === e.currentTarget && onclose()}>
  <div class="sheet" role="dialog" aria-modal="true" aria-labelledby="settings-title" bind:this={dialog}>
    <header>
      <h2 id="settings-title" class="caps">Settings</h2>
      <button class="btn ghost icon" aria-label="Close settings" onclick={onclose}>✕</button>
    </header>

    <section>
      <label for="roots" class="caps label">Folders searched for clones</label>
      <p class="hint">One folder per line. Sangraha looks three levels deep and only reads; it never changes a clone.</p>
      <textarea id="roots" rows="4" bind:value={rootsText} spellcheck="false"></textarea>
      {#if error}<p class="error" role="alert">{error}</p>{/if}
      <button class="btn" onclick={saveRoots}>Save folders</button>
    </section>

    <section>
      <h3 class="caps label">Hidden from the board</h3>
      {#if cfg && cfg.hidden.length}
        <ul>
          {#each cfg.hidden as key (key)}
            <li>
              <span class="mono">{key}</span>
              <button class="btn ghost" onclick={() => unhide(key)}>Show again</button>
            </li>
          {/each}
        </ul>
      {:else}
        <p class="hint">Nothing hidden. Use … on a row to hide it.</p>
      {/if}
    </section>

    <section>
      <h3 class="caps label">GitHub</h3>
      <p class="hint">
        Sangraha reads GitHub with your GitHub CLI login (<span class="mono">gh auth login</span>). The token is asked for when
        needed and never stored.
      </p>
    </section>

    <footer class="caps">Sangraha {version}</footer>
  </div>
</div>

<style>
  .scrim {
    position: fixed;
    inset: 0;
    z-index: 30;
    display: flex;
    justify-content: flex-end;
    background: rgb(0 0 0 / 35%);
    animation: fade 160ms ease-out;
  }
  .sheet {
    width: min(520px, 100%);
    height: 100%;
    overflow: auto;
    padding: 20px 24px;
    background: var(--surface);
    border-left: 1px solid var(--line-strong);
    display: flex;
    flex-direction: column;
    gap: 22px;
    animation: slide 220ms cubic-bezier(0.2, 0.9, 0.3, 1);
  }
  @keyframes fade {
    from {
      opacity: 0;
    }
  }
  @keyframes slide {
    from {
      transform: translateX(24px);
      opacity: 0;
    }
  }
  header {
    display: flex;
    justify-content: space-between;
    align-items: center;
  }
  h2,
  h3 {
    margin: 0;
    font-weight: 600;
  }
  section {
    display: grid;
    gap: 8px;
    justify-items: start;
  }
  .label {
    color: var(--muted);
  }
  .hint {
    margin: 0;
    color: var(--muted);
  }
  textarea {
    width: 100%;
    padding: 8px 10px;
    border: 1px solid var(--line-strong);
    border-radius: 2px;
    background: var(--bg);
    font-family: var(--mono);
    font-size: 13px;
    resize: vertical;
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    width: 100%;
    display: grid;
    gap: 6px;
  }
  li {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 12px;
  }
  .error {
    margin: 0;
    color: var(--bad);
  }
  footer {
    margin-top: auto;
    color: var(--faint);
  }
</style>
