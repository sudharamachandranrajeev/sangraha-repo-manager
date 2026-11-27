<div align="center">
  <img src="app/public/favicon.svg" width="140" alt="Sangraha logo">
  <h1>Sangraha</h1>
  <h4>Your GitHub repositories and the apps you publish, on one board: what needs you first, and what to update.</h4>

  <p>
    <img src="https://img.shields.io/badge/status-alpha-orange" alt="Status: alpha">
    <img src="https://img.shields.io/badge/platforms-Linux%20%7C%20Windows-blue" alt="Platforms: Linux and Windows">
    <img src="https://img.shields.io/badge/licence-MIT-green" alt="Licence: MIT">
  </p>

  <p>
    <a href="#-what-it-does">What it does</a> •
    <a href="#-get-started">Get started</a> •
    <a href="#-how-it-works">How it works</a> •
    <a href="#-faq">FAQ</a> •
    <a href="#-for-developers">For developers</a>
  </p>

  <img src="docs/screenshots/board-dark.png" width="900" alt="The Sangraha board: a table of repositories with failing checks, security alerts and unpushed work at the top">
</div>

Sangraha ("collection" in Sanskrit) is for people who keep many repositories at once. It has
two tabs:

- **Repos** reads GitHub and the clones on your computer together and answers *what needs me
  today?* Failing checks, security alerts, commits that never reached GitHub and forgotten
  changes show worst first, each with one button that takes you there.
- **Apps** lists the apps your repositories publish as GitHub releases (and any other
  repository you follow), shows which are installed here and which have an update, and
  installs them for you, checked and with an undo.

Sangraha is **alpha** software. On the Repos tab it only reads: it never commits, pushes or
changes your files. There is no download yet; build it from source (below).

## ✨ What it does

- 🚦 **Worst first** – a red check or a serious security alert sits above a stray file. Each
  row has one status and one button for the next step (**Open checks**, **Open alerts**,
  **Open terminal**).
- 💻 **GitHub and your computer together** – each repository shows its clones: branch, commits
  ahead or behind, changed files, stashes. Two clones of one repository show as one row.
- 📤 **Unpushed work** – commits on any branch that are not on GitHub, and folders with no
  remote at all, so nothing lives on one laptop only.
- 🛡️ **Security alerts** – open alerts per repository (Dependabot), with how many are high or
  critical.
- 🏷️ **Latest release, pre-releases included** – GitHub's "latest release" skips
  pre-releases; Sangraha shows the newest one either way.
- 🔎 **Filters with counts** – *Needs attention*, *Unpushed*, *Checks failing*, *Alerts*,
  *Not cloned*, *Local only*. Search with <kbd>/</kbd>, refresh with <kbd>R</kbd>.
- 🔁 **Renamed repositories** – a clone whose remote still uses an old name is matched to the
  new one, and the row says so.
- 🙈 **Hide what you do not care about** – hidden rows stay one click away under *Hidden*.
- 📦 **Your apps, installed and current** – the Apps tab finds the right file for this computer
  in each release (pre-releases included), and shows the version here next to the one on
  GitHub. It also finds copies you installed by hand.
- 🔐 **Checked downloads** – every file is checked against the SHA-256 (a fingerprint) that
  GitHub or the release publishes. A file without one is installed only if you say so.
- ↩️ **Roll back** – an update keeps the version it replaced, one click away. An app that is
  running is never replaced.
- 🧑 **No admin password** – apps go in a folder of your own (`~/Applications`, or
  `%LOCALAPPDATA%\Programs` on Windows) with a menu entry. **Update all** shows the list and
  the download size first.
- ⏳ **Wait a few days if you like** – an optional delay before an update is offered, so a
  broken release can be fixed before it reaches you.
- ⌨️ **Command line too** – `sangraha status` and `sangraha apps` print the same boards in a
  terminal.

## 📸 Screenshots

<table>
  <tr>
    <td><img src="docs/screenshots/board-light.png" alt="The board in the light theme"></td>
    <td><img src="docs/screenshots/apps-dark.png" alt="The Apps tab: one card per app, with updates first"></td>
  </tr>
  <tr>
    <td align="center">Repos tab, light theme</td>
    <td align="center">Apps tab, dark theme</td>
  </tr>
</table>

The screenshots use made-up demo repositories and apps.

## 🚀 Get started

There is no installer yet. Until the first release, build Sangraha on your own computer.

1. **Install the tools.** You need [Rust](https://rustup.rs), [Node.js](https://nodejs.org)
   22 or later, `git`, and the [GitHub CLI](https://cli.github.com) (`gh`). On Linux, also
   install the WebKitGTK libraries listed in Tauri's
   [prerequisites](https://v2.tauri.app/start/prerequisites/).
2. **Sign in to GitHub once.** Run `gh auth login` and follow the prompts. Sangraha borrows
   that sign-in each time it reads GitHub. It never saves the token.
3. **Build and start it.**

   ```sh
   git clone https://github.com/sudharamachandranrajeev/sangraha-repo-manager
   cd sangraha-repo-manager/app
   npm ci
   npx tauri dev
   ```

4. **First launch.** Sangraha looks for clones in your `Work` folder (or your home folder if
   there is none). To change that, press the settings button (top right) and list your
   folders, one per line. Then press the refresh button to read GitHub.

<details>
<summary>Just the command line</summary>

```sh
cargo install --path cli
sangraha status           # what needs attention
sangraha status --all     # every repository
sangraha status --fetch   # git fetch in every clone first
sangraha status --check   # exit code 1 when something needs attention
sangraha roots ~/code ~/Work
sangraha hide owner/name
sangraha apps --all       # every app, installed or not
sangraha install owner/name
sangraha install --updates
sangraha rollback owner/name
sangraha follow someone/their-app
```

</details>

## 🧭 How it works

```mermaid
flowchart LR
  A["Your clones<br/>(git status)"] --> C["Sangraha"]
  B["GitHub<br/>(one query per 25 repos)"] --> C
  C --> D["Repos tab,<br/>worst first"]
  C --> F["Apps tab"]
  F --> G["Checked install<br/>in your own folder"]
```

- Sangraha walks your chosen folders three levels deep and asks `git` about each clone. This
  is local and quick.
- It reads GitHub with a few GraphQL queries: about one point of your 5,000-an-hour budget per
  25 repositories. The footer shows what is left.
- It keeps the last GitHub answer on disk, so the board opens at once and works offline.
- **Fetch all** runs `git fetch` in every clone. That is the only time it touches a clone, and
  it only updates what your computer knows about GitHub, never your files.
- An install downloads the release file, checks its SHA-256, unpacks it beside the installed
  copy, then swaps the two. The old copy stays as `<folder>.previous` for **Roll back**.

## ❓ FAQ

<details>
<summary>It says it cannot sign in to GitHub</summary>

Run `gh auth login` in a terminal, then press refresh. Sangraha also accepts a token in the
`GH_TOKEN` or `GITHUB_TOKEN` environment variable.
</details>

<details>
<summary>A repository is missing</summary>

Sangraha lists repositories you own. A clone of someone else's repository shows as
*upstream*. Check the folders in settings: clones deeper than three levels, and folders such as
`node_modules`, `target` and `build`, are not searched.
</details>

<details>
<summary>Does it change anything in my repositories?</summary>

No. It runs read-only `git` commands, plus `git fetch` when you press **Fetch all**. Buttons
open GitHub, a file manager or a terminal; what happens next is up to you.
</details>

<details>
<summary>Which file does the Apps tab install?</summary>

On Linux: a portable `.tar.gz`, else an AppImage. On Windows: a portable `.zip`, else a single
`.exe`, else the setup program (which shows its own steps). System packages (`.deb`, `.rpm`,
`.pkg.tar.zst`) need an admin password, so Sangraha only links to them. If it picks the wrong
file, add a download rule in settings, for example `owner/app = *-linux-x64-wpe.tar.gz`.
</details>

<details>
<summary>It says "version unknown"</summary>

Sangraha found the app on disk but the copy does not say which version it is. **Install
latest** replaces it with the newest release, keeping the old copy for roll back.
</details>

<details>
<summary>An update failed: "is running"</summary>

Sangraha never replaces an app that is open. Close it, then press **Update** again.
</details>

<details>
<summary>What does "Local only" mean?</summary>

The folder is a git repository with no remote, or its remote is not on GitHub. Its commits
exist on this computer only.
</details>

## 🛠️ For developers

<details>
<summary>Build, test and screenshots</summary>

- `core/` – the boards: GitHub, clones, cache, and app installs (`apps.rs`, `install.rs`). `cli/` – the `sangraha` command. `app/` – the
  window (Tauri 2, Svelte 5).
- Checks: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo test --workspace`, and in `app/`, `npm run check`.
- In a plain browser, `npm run dev` reads `app/demo/board.json` and `app/demo/apps.json`
  instead of the real boards. They hold made-up repositories and apps for screenshots.

</details>

## License

[MIT](LICENSE)

> GitHub [@sudharamachandranrajeev](https://github.com/sudharamachandranrajeev)
