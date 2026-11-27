//! Sangraha's core: what GitHub says about the owner's repositories, what the local clones say,
//! and the board that joins them with the problems that need attention, worst first.
//!
//! Nothing here writes to a repository. GitHub is read through the owner's own `gh` login; the
//! token is asked for at run time and never stored.

pub mod apps;
pub mod board;
pub mod bulk;
pub mod cache;
pub mod config;
pub mod digest;
pub mod github;
pub mod hooks;
pub mod housekeeping;
pub mod install;
pub mod leaks;
pub mod local;
pub mod releases;
pub mod security;
pub mod subtrees;

pub use apps::{AppRow, AppStatus, AppsBoard};
pub use board::{Board, Counts, Level, Problem, ProblemKind, Row, build};
pub use config::Config;
pub use releases::ReleaseBoard;
pub use security::{Gate, SecurityBoard};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Auth(String),
    #[error("GitHub: {0}")]
    GitHub(String),
    #[error("git: {0}")]
    Git(String),
    #[error("cache: {0}")]
    Cache(#[from] rusqlite::Error),
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    Install(String),
    /// The release publishes no SHA-256 for the file; installing needs the owner's yes.
    #[error("{0}")]
    Unverified(String),
}

pub type Result<T> = std::result::Result<T, Error>;

/// The board from the cached GitHub snapshot and a fresh look at the clones. No network.
pub fn load(cfg: &Config) -> Result<Board> {
    let cache = cache::Cache::open_default()?;
    let clones = local::scan(&cfg.roots);
    Ok(assemble(&cache, cache.snapshot()?.as_ref(), clones, cfg))
}

/// Reads GitHub again (and, with `fetch_clones`, runs `git fetch` in every clone first), keeps
/// the snapshot, and returns the new board.
pub fn refresh(cfg: &Config, fetch_clones: bool) -> Result<Board> {
    let token = github::token()?;
    let cache = cache::Cache::open_default()?;
    let mut warnings = Vec::new();
    if fetch_clones {
        let paths: Vec<_> = local::scan(&cfg.roots)
            .into_iter()
            .map(|c| c.path)
            .collect();
        for (path, err) in local::fetch(&paths) {
            warnings.push(format!("git fetch failed in {}: {err}", path.display()));
        }
    }
    let snap = github::fetch(&token)?;
    cache.set_snapshot(&snap)?;
    let clones = local::scan(&cfg.roots);
    resolve_renames(&cache, &token, &snap, &clones)?;
    let mut board = assemble(&cache, Some(&snap), clones, cfg);
    board.warnings.extend(warnings);
    Ok(board)
}

/// Asks GitHub once about each clone whose remote names one of the owner's repositories that
/// the snapshot does not list: renamed repositories still answer under their old name.
fn resolve_renames(
    cache: &cache::Cache,
    token: &str,
    snap: &github::Snapshot,
    clones: &[local::LocalClone],
) -> Result<()> {
    let mut renames = cache.renames()?;
    let known: std::collections::HashSet<String> = snap
        .repos
        .iter()
        .map(|r| r.full_name.to_lowercase())
        .collect();
    let prefix = format!("{}/", snap.login.to_lowercase());
    let mut changed = false;
    for g in clones
        .iter()
        .filter_map(|c| c.github.as_ref())
        .map(|g| g.to_lowercase())
    {
        if snap.login.is_empty()
            || !g.starts_with(&prefix)
            || known.contains(&g)
            || renames.contains_key(&g)
        {
            continue;
        }
        let now_called = github::current_name(token, &g)?.unwrap_or_default();
        let now_called = if now_called.to_lowercase() == g {
            String::new()
        } else {
            now_called
        };
        renames.insert(g, now_called);
        changed = true;
    }
    if changed {
        cache.set_renames(&renames)?;
    }
    Ok(())
}

fn assemble(
    cache: &cache::Cache,
    snap: Option<&github::Snapshot>,
    clones: Vec<local::LocalClone>,
    cfg: &Config,
) -> Board {
    let renames: std::collections::HashMap<String, String> = cache
        .renames()
        .unwrap_or_default()
        .into_iter()
        .filter(|(_, v)| !v.is_empty())
        .collect();
    let mut board = build(snap, clones, &renames);
    board.roots = cfg.roots.clone();
    board.hide(&cfg.hidden);
    board
}

/// The Apps tab from the last GitHub answer and what is installed here. No network.
pub fn apps_load(cfg: &Config) -> Result<AppsBoard> {
    let cache = cache::Cache::open_default()?;
    let login = cache.snapshot()?.map(|s| s.login);
    Ok(apps_assemble(cache.apps()?.as_ref(), login.as_deref(), cfg))
}

/// Reads the releases of every followed app again. Uses the repository snapshot to know which
/// of the owner's repositories have releases, reading it first if there is none yet.
pub fn apps_refresh(cfg: &Config) -> Result<AppsBoard> {
    let token = github::token()?;
    let cache = cache::Cache::open_default()?;
    let repos = match cache.snapshot()? {
        Some(s) => s,
        None => {
            let s = github::fetch(&token)?;
            cache.set_snapshot(&s)?;
            s
        }
    };
    let names = apps::followed(Some(&repos), cfg);
    let snap = apps::fetch(&token, &names)?;
    cache.set_apps(&snap)?;
    Ok(apps_assemble(Some(&snap), Some(&repos.login), cfg))
}

fn apps_assemble(
    snap: Option<&apps::AppsSnapshot>,
    login: Option<&str>,
    cfg: &Config,
) -> AppsBoard {
    let registry = install::Registry::load();
    apps::build(snap, login, cfg, &registry, &|name| {
        install::detect(cfg, name)
    })
}

/// Installs or updates one app (see `install::install`).
pub fn install_app(
    cfg: &Config,
    key: &str,
    allow_unverified: bool,
    progress: &(dyn Fn(u64, u64) + Sync),
) -> Result<String> {
    let cache = cache::Cache::open_default()?;
    let snap = cache.apps()?.ok_or_else(|| {
        Error::Install("The Apps list has not been read yet; refresh first".into())
    })?;
    install::install(cfg, &snap, key, allow_unverified, progress)
}

/// The Security tab from the last GitHub answer, with a fresh look at the clones' hooks.
pub fn security_load(cfg: &Config) -> Result<SecurityBoard> {
    let cache = cache::Cache::open_default()?;
    Ok(security::build(
        cache.security()?.as_ref(),
        security::hook_rows(&cfg.roots),
    ))
}

/// Reads the security alerts again. Uses the repository snapshot to know which repositories
/// have Dependabot alerts, reading it first if there is none yet.
pub fn security_refresh(cfg: &Config) -> Result<SecurityBoard> {
    let token = github::token()?;
    let cache = cache::Cache::open_default()?;
    let repos = match cache.snapshot()? {
        Some(s) => s,
        None => {
            let s = github::fetch(&token)?;
            cache.set_snapshot(&s)?;
            s
        }
    };
    let snap = security::fetch(&token, &repos.repos)?;
    cache.set_security(&snap)?;
    Ok(security::build(
        Some(&snap),
        security::hook_rows(&cfg.roots),
    ))
}

fn renames_known(cache: &cache::Cache) -> std::collections::HashMap<String, String> {
    cache.renames().unwrap_or_default()
}

fn releases_assemble(
    cfg: &Config,
    cache: &cache::Cache,
    snap: Option<&github::Snapshot>,
) -> ReleaseBoard {
    let clones = local::scan(&cfg.roots);
    let rows = releases::rows(&clones, snap);
    let mut subtrees = subtrees::find(&clones, snap, &renames_known(cache));
    let drift = cache.drift().ok().flatten();
    if let Some(d) = &drift {
        subtrees::apply(&mut subtrees, d);
    }
    ReleaseBoard {
        counts: releases::count(&rows, &subtrees),
        rows,
        subtrees,
        checked_at: drift.map(|d| d.checked_at),
        warnings: Vec::new(),
    }
}

/// The Releases tab from the clones, with what GitHub said last about the subtrees. No network.
pub fn releases_load(cfg: &Config) -> Result<ReleaseBoard> {
    let cache = cache::Cache::open_default()?;
    let snap = cache.snapshot()?;
    Ok(releases_assemble(cfg, &cache, snap.as_ref()))
}

/// Asks GitHub how far each subtree's source has moved on (following renamed sources).
pub fn releases_refresh(cfg: &Config) -> Result<ReleaseBoard> {
    let token = github::token()?;
    let cache = cache::Cache::open_default()?;
    let snap = cache.snapshot()?;
    let clones = local::scan(&cfg.roots);
    let mut renames = renames_known(&cache);
    let known: std::collections::HashSet<String> = snap
        .iter()
        .flat_map(|s| &s.repos)
        .map(|r| r.full_name.to_lowercase())
        .collect();
    let mut changed = false;
    for sub in subtrees::find(&clones, snap.as_ref(), &renames) {
        let Some(src) = sub.source.map(|s| s.to_lowercase()) else {
            continue;
        };
        if known.contains(&src) || renames.contains_key(&src) || !apps::valid_name(&src) {
            continue;
        }
        let now_called = github::current_name(&token, &src)?.unwrap_or_default();
        renames.insert(
            src.clone(),
            if now_called.to_lowercase() == src {
                String::new()
            } else {
                now_called
            },
        );
        changed = true;
    }
    if changed {
        cache.set_renames(&renames)?;
    }
    let subs = subtrees::find(&clones, snap.as_ref(), &renames);
    cache.set_drift(&subtrees::compare(&token, &subs)?)?;
    Ok(releases_assemble(cfg, &cache, snap.as_ref()))
}

/// The GitHub release for a tag and the workflow runs on its commit.
pub fn release_details(repo: &str, tag: &str, sha: &str) -> Result<releases::Details> {
    releases::details(&github::token()?, repo, tag, sha)
}

/// Runs `git subtree pull` for the copy in `dir` of the clone at `path`, if Sangraha found
/// one there.
pub fn subtree_pull(cfg: &Config, path: &std::path::Path, dir: &str) -> Result<String> {
    let cache = cache::Cache::open_default()?;
    let snap = cache.snapshot()?;
    let clones: Vec<_> = local::scan(&cfg.roots)
        .into_iter()
        .filter(|c| c.path == path)
        .collect();
    let sub = subtrees::find(&clones, snap.as_ref(), &renames_known(&cache))
        .into_iter()
        .find(|s| s.dir == dir)
        .ok_or_else(|| Error::Git(format!("No subtree {dir} in {}", path.display())))?;
    subtrees::pull(&sub)
}

/// The snapshot of the owner's repositories, read first if there is none.
fn snapshot_or_fetch(cache: &cache::Cache, token: &str) -> Result<github::Snapshot> {
    match cache.snapshot()? {
        Some(s) => Ok(s),
        None => {
            let s = github::fetch(token)?;
            cache.set_snapshot(&s)?;
            Ok(s)
        }
    }
}

/// The Housekeeping tab from GitHub's last answer and the clones. No network.
pub fn tidy_load(cfg: &Config) -> Result<housekeeping::TidyBoard> {
    let cache = cache::Cache::open_default()?;
    Ok(housekeeping::build(
        cache.tidy()?.as_ref(),
        &local::scan(&cfg.roots),
    ))
}

/// Reads branches, top-level files and old workflow runs from GitHub again.
pub fn tidy_refresh(cfg: &Config) -> Result<housekeeping::TidyBoard> {
    let token = github::token()?;
    let cache = cache::Cache::open_default()?;
    let snap = snapshot_or_fetch(&cache, &token)?;
    let tidy = housekeeping::fetch(&token, &snap.repos)?;
    cache.set_tidy(&tidy)?;
    Ok(housekeeping::build(Some(&tidy), &local::scan(&cfg.roots)))
}

/// Deletes merged branches of one of the owner's repositories (those the last housekeeping
/// read listed as merged, and only while unchanged).
pub fn delete_merged(repo: &str, names: &[String]) -> Result<Vec<String>> {
    let cache = cache::Cache::open_default()?;
    let known = cache
        .tidy()?
        .and_then(|t| {
            t.repos
                .into_iter()
                .find(|r| r.repo.eq_ignore_ascii_case(repo))
        })
        .ok_or_else(|| Error::GitHub(format!("{repo}: read housekeeping first")))?;
    let branches: Vec<_> = known
        .merged
        .into_iter()
        .filter(|b| names.contains(&b.name))
        .collect();
    let out = housekeeping::delete_merged(&github::token()?, repo, &branches)?;
    // Forget the deleted ones, so the board does not offer them again.
    if let Some(mut t) = cache.tidy()? {
        if let Some(r) = t
            .repos
            .iter_mut()
            .find(|r| r.repo.eq_ignore_ascii_case(repo))
        {
            r.merged
                .retain(|b| !out.iter().any(|l| l == &format!("{}: deleted", b.name)));
        }
        cache.set_tidy(&t)?;
    }
    Ok(out)
}

/// The digest from what Sangraha last read, or (with `refresh`) after reading GitHub again:
/// repositories, apps, security and subtrees every time, housekeeping once a week. With `mark`,
/// its items count as seen, and the next digest calls only later ones new.
pub fn digest(cfg: &Config, refresh_first: bool, mark: bool) -> Result<digest::Digest> {
    let cache = cache::Cache::open_default()?;
    let (board, apps, security, releases) = if refresh_first {
        (
            refresh(cfg, false)?,
            apps_refresh(cfg)?,
            security_refresh(cfg)?,
            releases_refresh(cfg)?,
        )
    } else {
        (
            load(cfg)?,
            apps_load(cfg)?,
            security_load(cfg)?,
            releases_load(cfg)?,
        )
    };
    let week_old = cache
        .tidy()?
        .is_none_or(|t| now() - t.checked_at > 7 * 86_400);
    let tidy = if refresh_first && week_old {
        tidy_refresh(cfg)?
    } else {
        tidy_load(cfg)?
    };
    let seen = cache.digest_seen()?;
    let d = digest::build(
        Some(&board),
        Some(&apps),
        Some(&security),
        Some(&releases),
        Some(&tidy),
        &seen,
    );
    if mark {
        cache.set_digest_seen(&d.items.iter().map(|i| i.id.clone()).collect())?;
    }
    Ok(d)
}

/// What the `pre-push` hook decides: the leak check over the commits being pushed, and for a
/// tag, the release gate.
#[derive(Debug, Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PushCheck {
    pub leaks: leaks::Report,
    pub gate: Option<Gate>,
}

impl PushCheck {
    pub fn blocked(&self) -> bool {
        !self.leaks.findings.is_empty() || self.gate.as_ref().is_some_and(|g| g.blocked)
    }
}

/// Runs the push check in `repo` for the refs a `pre-push` hook got on standard input.
/// `remote_url` names the GitHub repository for the release gate.
pub fn check_push(
    cfg: &Config,
    repo: &std::path::Path,
    stdin: &str,
    remote_url: &str,
) -> Result<PushCheck> {
    let login = cache::Cache::open_default()
        .ok()
        .and_then(|c| c.snapshot().ok().flatten())
        .map(|s| s.login);
    let rules = leaks::Rules::for_owner(cfg, login, repo);
    let (ranges, tags) = leaks::push_ranges(repo, stdin);
    let leaks = leaks::scan_all(repo, &ranges, &rules)?;
    let gate = tags.then(|| {
        let full = local::github_name(remote_url);
        security::gate(full.as_deref(), Some(repo))
    });
    Ok(PushCheck { leaks, gate })
}

/// The owner's repositories (`owner/name`, lower case), from the snapshot, read first if
/// there is none: bulk changes act on these only.
fn owned(token: &str) -> Result<Vec<String>> {
    let cache = cache::Cache::open_default()?;
    let snap = match cache.snapshot()? {
        Some(s) => s,
        None => {
            let s = github::fetch(token)?;
            cache.set_snapshot(&s)?;
            s
        }
    };
    Ok(snap
        .repos
        .iter()
        .map(|r| r.full_name.to_lowercase())
        .collect())
}

/// The dry run of a bulk change: what would change in each repository. Only reads.
pub fn bulk_plan(cfg: &Config, req: &bulk::Request) -> Result<bulk::Plan> {
    let token = github::token()?;
    bulk::plan(&token, req, &owned(&token)?, &cfg.roots)
}

/// Makes a bulk change (planning it again first). `progress` hears each repository's name.
pub fn bulk_apply(
    cfg: &Config,
    req: &bulk::Request,
    progress: &(dyn Fn(&str) + Sync),
) -> Result<bulk::Plan> {
    let token = github::token()?;
    bulk::apply(&token, req, &owned(&token)?, &cfg.roots, progress)
}

/// Seconds since the Unix epoch.
pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Seconds since the Unix epoch for a GitHub time (`2026-11-27T14:05:00Z`).
pub fn parse_time(iso: &str) -> Option<i64> {
    let b = iso.as_bytes();
    if b.len() < 19 || b[4] != b'-' || b[10] != b'T' {
        return None;
    }
    let n = |r: std::ops::Range<usize>| iso.get(r)?.parse::<i64>().ok();
    let (y, m, d) = (n(0..4)?, n(5..7)?, n(8..10)?);
    let (hh, mm, ss) = (n(11..13)?, n(14..16)?, n(17..19)?);
    // Days from 1970-01-01 (Howard Hinnant's days_from_civil).
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400 + hh * 3600 + mm * 60 + ss)
}

/// "2026-11-27" for Unix seconds (UTC).
pub fn iso_date(secs: i64) -> String {
    // Howard Hinnant's civil_from_days.
    let z = secs.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

/// "linux x64", "windows arm64" … for the footer and, later, for picking release assets.
pub fn platform() -> String {
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        other => other,
    };
    format!("{} {arch}", std::env::consts::OS)
}

#[cfg(test)]
mod tests {
    #[test]
    fn github_times() {
        assert_eq!(
            super::iso_date(super::parse_time("2026-11-27T14:05:09Z").unwrap()),
            "2026-11-27"
        );
        assert_eq!(super::iso_date(0), "1970-01-01");
        assert_eq!(
            super::iso_date(super::parse_time("2024-02-29T23:59:59Z").unwrap()),
            "2024-02-29"
        );
        assert_eq!(super::parse_time("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(
            super::parse_time("2026-11-27T14:05:09Z"),
            Some(1_795_788_309)
        );
        assert_eq!(super::parse_time("2000-02-29T00:00:00Z"), Some(951_782_400));
        assert_eq!(super::parse_time("yesterday"), None);
    }
}
