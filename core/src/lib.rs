//! Sangraha's core: what GitHub says about the owner's repositories, what the local clones say,
//! and the board that joins them with the problems that need attention, worst first.
//!
//! Nothing here writes to a repository. GitHub is read through the owner's own `gh` login; the
//! token is asked for at run time and never stored.

pub mod board;
pub mod cache;
pub mod config;
pub mod github;
pub mod local;

pub use board::{Board, Counts, Level, Problem, ProblemKind, Row, build};
pub use config::Config;

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

/// Seconds since the Unix epoch.
pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
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
