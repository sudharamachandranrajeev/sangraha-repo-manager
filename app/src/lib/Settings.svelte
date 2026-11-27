<script lang="ts">
  import { onMount } from 'svelte'
  import { api, type Config } from './api'

  let { onclose, onchange }: { onclose: () => void; onchange: () => void } = $props()

  let cfg = $state<Config | null>(null)
  let rootsText = $state('')
  let error = $state('')
  let appsDir = $state('')
  let prereleases = $state(true)
  let delayDays = $state(0)
  let extraText = $state('')
  let rulesText = $state('')
  let appsError = $state('')
  let appsSaved = $state(false)
  let version = $state('')
  let dialog: HTMLDivElement

  onMount(async () => {
    cfg = await api.settings()
    rootsText = cfg.roots.join('\n')
    appsDir = cfg.appsDir ?? ''
    prereleases = cfg.prereleases
    delayDays = cfg.delayDays
    extraText = cfg.extraApps.join('\n')
    rulesText = Object.entries(cfg.assetRules)
      .map(([k, v]) => `${k} = ${v}`)
      .join('\n')
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

  async function saveApps() {
    appsError = ''
    appsSaved = false
    try {
      cfg = await api.saveAppsSettings({
        appsDir,
        prereleases,
        delayDays: Math.max(0, Math.round(Number(delayDays) || 0)),
        extraApps: extraText.split('\n'),
        rules: rulesText.split('\n'),
      })
      appsSaved = true
      onchange()
    } catch (e) {
      appsError = String(e)
    }
  }

  async function unhideApp(key: string) {
    cfg = await api.setAppHidden(key, false)
    onchange()
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
      <h3 class="caps label">Apps</h3>
      <label for="apps-dir" class="small">Install new apps in</label>
      <input id="apps-dir" type="text" bind:value={appsDir} placeholder="Default: Applications in your home folder" spellcheck="false" />
      <p class="hint">
        Apps are installed for you only, with no admin password. An app found elsewhere (installed by hand) is updated where it is.
      </p>
      <label class="check"><input type="checkbox" bind:checked={prereleases} /> Offer pre-releases (alpha, beta)</label>
      <label class="small" for="delay">Wait before offering an update (days)</label>
      <input id="delay" type="number" min="0" max="365" bind:value={delayDays} />
      <p class="hint">A few days' wait lets a broken release be fixed before it reaches you.</p>
      <label for="extra" class="small">Also follow these repositories (owner/name, one per line)</label>
      <textarea id="extra" rows="3" bind:value={extraText} spellcheck="false" placeholder="someone/their-app"></textarea>
      <label for="rules" class="small">Download rules (owner/name = file pattern, * for any text)</label>
      <textarea id="rules" rows="2" bind:value={rulesText} spellcheck="false" placeholder="owner/app = *-linux-x64-wpe.tar.gz"></textarea>
      <p class="hint">Only needed when Sangraha picks the wrong file for this computer.</p>
      {#if appsError}<p class="error" role="alert">{appsError}</p>{/if}
      <button class="btn" onclick={saveApps}>Save app settings</button>
      {#if appsSaved}<span class="hint" role="status"> Saved. Press refresh on the Apps tab to read the new list.</span>{/if}
      {#if cfg && cfg.hiddenApps.length}
        <p class="small">Hidden apps</p>
        <ul>
          {#each cfg.hiddenApps as key (key)}
            <li>
              <span class="mono">{key}</span>
              <button class="btn ghost" onclick={() => unhideApp(key)}>Show again</button>
            </li>
          {/each}
        </ul>
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
  input[type='text'],
  input[type='number'] {
    width: 100%;
    height: 34px;
    padding: 0 10px;
    border: 1px solid var(--line-strong);
    border-radius: 2px;
    background: var(--bg);
    font-family: var(--mono);
    font-size: 13px;
  }
  input[type='number'] {
    width: 100px;
  }
  .check {
    display: flex;
    gap: 8px;
    align-items: center;
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
