//! The board: every repository on GitHub joined with its local clones, each with the problems
//! that need attention and the one action that fixes the worst of them.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::github::{Alerts, Ci, Rate, Release, RemoteRepo, Snapshot};
use crate::local::LocalClone;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "camelCase")]
pub enum Level {
    Info,
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ProblemKind {
    CiFailing,
    Alerts,
    Unpushed,
    Uncommitted,
    Unreadable,
    LocalOnly,
    Behind,
    Untracked,
    Stashes,
    Detached,
    OldRemoteName,
    NotCloned,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Problem {
    pub kind: ProblemKind,
    pub level: Level,
    /// Plain words, e.g. "3 commits not on GitHub (sangraha)".
    pub text: String,
    /// The clone it is about, for local problems.
    pub path: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum Action {
    OpenUrl { label: String, url: String },
    OpenTerminal { label: String, path: PathBuf },
    OpenFolder { label: String, path: PathBuf },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Row {
    /// `owner/name`, or the folder path for a clone with no GitHub repository.
    pub key: String,
    pub name: String,
    pub full_name: Option<String>,
    pub url: Option<String>,
    pub description: Option<String>,
    pub private: bool,
    pub archived: bool,
    pub fork: bool,
    pub language: Option<String>,
    pub pushed_at: Option<String>,
    pub default_branch: Option<String>,
    pub ci: Ci,
    pub release: Option<Release>,
    pub alerts: Option<Alerts>,
    pub open_issues: u32,
    pub open_prs: u32,
    /// Someone else's repository (a clone of an upstream project).
    pub foreign: bool,
    pub clones: Vec<LocalClone>,
    /// Worst first.
    pub problems: Vec<Problem>,
    /// The fix for the worst problem, or the natural next step when there is none.
    pub action: Option<Action>,
    /// For sorting: high = 100, medium = 10, low = 1 each.
    pub score: u32,
    /// The owner hid it; left out of the counts.
    pub hidden: bool,
}

impl Row {
    pub fn worst(&self) -> Option<&Problem> {
        self.problems.first()
    }
    pub fn needs_attention(&self) -> bool {
        self.worst().is_some_and(|p| p.level >= Level::Medium)
    }
    fn has(&self, kind: ProblemKind) -> bool {
        self.problems.iter().any(|p| p.kind == kind)
    }
}

/// Numbers for the filter tabs.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Counts {
    pub all: u32,
    pub attention: u32,
    pub unpushed: u32,
    pub ci_red: u32,
    pub alerts: u32,
    pub not_cloned: u32,
    pub local_only: u32,
    pub hidden: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Board {
    pub login: String,
    /// When GitHub was last read (Unix seconds); `None` if never.
    pub checked_at: Option<i64>,
    pub rate: Option<Rate>,
    pub platform: String,
    pub roots: Vec<PathBuf>,
    pub rows: Vec<Row>,
    pub counts: Counts,
    pub warnings: Vec<String>,
}

fn plural(n: u32, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

fn folder(c: &LocalClone) -> String {
    c.path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| c.path.display().to_string())
}

/// Joins GitHub's view with the clones. `renames` maps an old `owner/name` (still in a clone's
/// remote) to the current one.
pub fn build(
    snap: Option<&Snapshot>,
    clones: Vec<LocalClone>,
    renames: &HashMap<String, String>,
) -> Board {
    let login = snap.map(|s| s.login.clone()).unwrap_or_default();
    let mut by_name: BTreeMap<String, Vec<LocalClone>> = BTreeMap::new();
    let mut loose = Vec::new();
    let known: HashMap<String, &RemoteRepo> = snap
        .map(|s| {
            s.repos
                .iter()
                .map(|r| (r.full_name.to_lowercase(), r))
                .collect()
        })
        .unwrap_or_default();

    for c in clones {
        let target = c.github.as_ref().map(|g| {
            let g = g.to_lowercase();
            renames.get(&g).map(|n| n.to_lowercase()).unwrap_or(g)
        });
        match target {
            Some(t) if known.contains_key(&t) => by_name.entry(t).or_default().push(c),
            _ => loose.push(c),
        }
    }

    let mut rows = Vec::new();
    for r in snap.map(|s| s.repos.as_slice()).unwrap_or_default() {
        let clones = by_name
            .remove(&r.full_name.to_lowercase())
            .unwrap_or_default();
        rows.push(remote_row(r, clones, renames));
    }
    // Clones of repositories that are not the owner's (or not readable): one row per remote.
    let mut foreign: BTreeMap<String, Vec<LocalClone>> = BTreeMap::new();
    for c in loose {
        match &c.github {
            Some(g) => foreign.entry(g.clone()).or_default().push(c),
            None => rows.push(local_row(c)),
        }
    }
    for (full, clones) in foreign {
        let mine = !login.is_empty()
            && full
                .to_lowercase()
                .starts_with(&format!("{}/", login.to_lowercase()));
        rows.push(foreign_row(&full, clones, !mine));
    }

    for row in &mut rows {
        finish(row);
    }
    rows.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| b.pushed_at.cmp(&a.pushed_at))
            .then_with(|| a.key.cmp(&b.key))
    });

    let counts = count(&rows);
    Board {
        login,
        checked_at: snap.map(|s| s.fetched_at),
        rate: snap.and_then(|s| s.rate.clone()),
        platform: crate::platform(),
        roots: Vec::new(),
        rows,
        counts,
        warnings: snap.map(|s| s.warnings.clone()).unwrap_or_default(),
    }
}

fn count(rows: &[Row]) -> Counts {
    let mut counts = Counts::default();
    for r in rows {
        if r.hidden {
            counts.hidden += 1;
            continue;
        }
        counts.all += 1;
        counts.attention += r.needs_attention() as u32;
        counts.unpushed += r.has(ProblemKind::Unpushed) as u32;
        counts.ci_red += r.has(ProblemKind::CiFailing) as u32;
        counts.alerts += r.has(ProblemKind::Alerts) as u32;
        counts.not_cloned += r.has(ProblemKind::NotCloned) as u32;
        counts.local_only += r.has(ProblemKind::LocalOnly) as u32;
    }
    counts
}

impl Board {
    /// Marks the rows the owner hid and counts again.
    pub fn hide(&mut self, keys: &[String]) {
        for r in &mut self.rows {
            r.hidden = keys.iter().any(|k| k == &r.key);
        }
        self.counts = count(&self.rows);
    }
}

pub(crate) fn empty_row(key: String, name: String) -> Row {
    Row {
        key,
        name,
        full_name: None,
        url: None,
        description: None,
        private: false,
        archived: false,
        fork: false,
        language: None,
        pushed_at: None,
        default_branch: None,
        ci: Ci::None,
        release: None,
        alerts: None,
        open_issues: 0,
        open_prs: 0,
        foreign: false,
        clones: Vec::new(),
        problems: Vec::new(),
        action: None,
        score: 0,
        hidden: false,
    }
}

fn remote_row(r: &RemoteRepo, clones: Vec<LocalClone>, renames: &HashMap<String, String>) -> Row {
    let mut row = empty_row(r.full_name.clone(), r.name.clone());
    row.full_name = Some(r.full_name.clone());
    row.url = Some(r.url.clone());
    row.description = r.description.clone();
    row.private = r.private;
    row.archived = r.archived;
    row.fork = r.fork;
    row.language = r.language.clone();
    row.pushed_at = r.pushed_at.clone();
    row.default_branch = r.default_branch.clone();
    row.ci = r.ci;
    row.release = r.release.clone();
    row.alerts = r.alerts.clone();
    row.open_issues = r.open_issues;
    row.open_prs = r.open_prs;

    let p = &mut row.problems;
    if !r.archived {
        if r.ci == Ci::Failing {
            let branch = r.default_branch.as_deref().unwrap_or("default branch");
            p.push(Problem {
                kind: ProblemKind::CiFailing,
                level: Level::High,
                text: format!("Checks failing on {branch}"),
                path: None,
            });
        }
        if let Some(a) = r.alerts.as_ref().filter(|a| a.total > 0) {
            let serious = a.critical + a.high;
            let text = if serious > 0 {
                format!(
                    "{} ({} critical, {} high)",
                    plural(a.total, "security alert", "security alerts"),
                    a.critical,
                    a.high
                )
            } else {
                plural(a.total, "security alert", "security alerts")
            };
            let level = if serious > 0 {
                Level::High
            } else {
                Level::Medium
            };
            p.push(Problem {
                kind: ProblemKind::Alerts,
                level,
                text,
                path: None,
            });
        }
    }
    if clones.is_empty() && !r.archived {
        p.push(Problem {
            kind: ProblemKind::NotCloned,
            level: Level::Info,
            text: "Not cloned on this computer".into(),
            path: None,
        });
    }
    for c in &clones {
        if let Some(old) = c
            .github
            .as_ref()
            .filter(|g| renames.contains_key(&g.to_lowercase()))
        {
            p.push(Problem {
                kind: ProblemKind::OldRemoteName,
                level: Level::Low,
                text: format!("{}: origin still uses the old name {old}", folder(c)),
                path: Some(c.path.clone()),
            });
        }
    }
    row.clones = clones;
    row
}

fn local_row(c: LocalClone) -> Row {
    let mut row = empty_row(c.path.display().to_string(), folder(&c));
    let text = match &c.other_remote {
        Some(url) => format!("Not on GitHub (origin is {url})"),
        None => "No remote: only on this computer".to_owned(),
    };
    let level = if c.other_remote.is_some() {
        Level::Info
    } else {
        Level::Medium
    };
    row.problems.push(Problem {
        kind: ProblemKind::LocalOnly,
        level,
        text,
        path: Some(c.path.clone()),
    });
    row.clones = vec![c];
    row
}

fn foreign_row(full: &str, clones: Vec<LocalClone>, foreign: bool) -> Row {
    let name = full.rsplit('/').next().unwrap_or(full).to_owned();
    let mut row = empty_row(full.to_owned(), name);
    row.full_name = Some(full.to_owned());
    row.url = Some(format!("https://github.com/{full}"));
    row.foreign = foreign;
    if !foreign {
        // The owner's name, but GitHub did not list it: deleted, renamed without a redirect, or
        // GitHub has not been read yet.
        row.problems.push(Problem {
            kind: ProblemKind::LocalOnly,
            level: Level::Low,
            text: "Not among your GitHub repositories (deleted or renamed?)".into(),
            path: None,
        });
    }
    row.clones = clones;
    row
}

/// Adds the local problems of every clone, sorts the problems, picks the action and the score.
fn finish(row: &mut Row) {
    for c in &row.clones {
        let f = folder(c);
        let at = || Some(c.path.clone());
        if let Some(e) = &c.error {
            row.problems.push(Problem {
                kind: ProblemKind::Unreadable,
                level: Level::Medium,
                text: format!("{f}: git cannot read it ({e})"),
                path: at(),
            });
            continue;
        }
        if c.unpushed > 0 {
            row.problems.push(Problem {
                kind: ProblemKind::Unpushed,
                level: Level::Medium,
                text: format!(
                    "{f}: {} not on GitHub",
                    plural(c.unpushed, "commit", "commits")
                ),
                path: at(),
            });
        }
        if c.changed > 0 {
            row.problems.push(Problem {
                kind: ProblemKind::Uncommitted,
                level: Level::Medium,
                text: format!(
                    "{f}: {} not committed",
                    plural(c.changed, "changed file", "changed files")
                ),
                path: at(),
            });
        }
        if c.behind > 0 {
            row.problems.push(Problem {
                kind: ProblemKind::Behind,
                level: Level::Low,
                text: format!(
                    "{f}: {} behind {}",
                    plural(c.behind, "commit", "commits"),
                    c.upstream.as_deref().unwrap_or("upstream")
                ),
                path: at(),
            });
        }
        if c.untracked > 0 {
            row.problems.push(Problem {
                kind: ProblemKind::Untracked,
                level: Level::Low,
                text: format!(
                    "{f}: {} not tracked",
                    plural(c.untracked, "new file", "new files")
                ),
                path: at(),
            });
        }
        if c.stashes > 0 {
            row.problems.push(Problem {
                kind: ProblemKind::Stashes,
                level: Level::Low,
                text: format!("{f}: {} kept aside", plural(c.stashes, "stash", "stashes")),
                path: at(),
            });
        }
        if c.branch.is_none() {
            row.problems.push(Problem {
                kind: ProblemKind::Detached,
                level: Level::Low,
                text: format!("{f}: not on a branch (detached)"),
                path: at(),
            });
        }
    }
    // Stable sort keeps the order above within one level.
    row.problems.sort_by_key(|p| std::cmp::Reverse(p.level));
    row.score = row
        .problems
        .iter()
        .map(|p| match p.level {
            Level::High => 100,
            Level::Medium => 10,
            Level::Low => 1,
            Level::Info => 0,
        })
        .sum();
    row.action = action(row);
}

fn action(row: &Row) -> Option<Action> {
    let url = row.url.clone();
    let first_clone = row.clones.first().map(|c| c.path.clone());
    let worst = row.worst();
    match worst.map(|p| p.kind) {
        Some(ProblemKind::CiFailing) => url.map(|u| Action::OpenUrl {
            label: "Open checks".into(),
            url: format!("{u}/actions"),
        }),
        Some(ProblemKind::Alerts) => url.map(|u| Action::OpenUrl {
            label: "Open alerts".into(),
            url: format!("{u}/security/dependabot"),
        }),
        Some(ProblemKind::NotCloned) => url.map(|u| Action::OpenUrl {
            label: "Open on GitHub".into(),
            url: u,
        }),
        Some(_) => worst
            .and_then(|p| p.path.clone())
            .or(first_clone)
            .map(|path| Action::OpenTerminal {
                label: "Open terminal".into(),
                path,
            }),
        None => match first_clone {
            Some(path) => Some(Action::OpenFolder {
                label: "Open folder".into(),
                path,
            }),
            None => url.map(|u| Action::OpenUrl {
                label: "Open on GitHub".into(),
                url: u,
            }),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo(name: &str) -> RemoteRepo {
        RemoteRepo {
            name: name.into(),
            full_name: format!("me/{name}"),
            url: format!("https://github.com/me/{name}"),
            default_branch: Some("main".into()),
            ..Default::default()
        }
    }

    fn clone_of(path: &str, gh: Option<&str>) -> LocalClone {
        LocalClone {
            path: path.into(),
            github: gh.map(Into::into),
            branch: Some("main".into()),
            ..Default::default()
        }
    }

    fn snap(repos: Vec<RemoteRepo>) -> Snapshot {
        Snapshot {
            login: "me".into(),
            repos,
            fetched_at: 1,
            ..Default::default()
        }
    }

    #[test]
    fn joins_and_ranks_worst_first() {
        let mut red = repo("red");
        red.ci = Ci::Failing;
        let mut alerted = repo("alerted");
        alerted.alerts = Some(Alerts {
            total: 2,
            moderate: 2,
            ..Default::default()
        });
        let clean = repo("clean");
        let far = repo("far");
        let s = snap(vec![clean, far, alerted, red]);

        let mut dirty = clone_of("/w/clean", Some("Me/Clean"));
        dirty.changed = 2;
        let clones = vec![
            dirty,
            clone_of("/w/red", Some("me/red")),
            clone_of("/w/alerted", Some("me/alerted")),
        ];
        let b = build(Some(&s), clones, &HashMap::new());

        let keys: Vec<_> = b.rows.iter().map(|r| r.key.as_str()).collect();
        assert_eq!(keys, ["me/red", "me/alerted", "me/clean", "me/far"]);
        assert_eq!(
            b.rows[0].action,
            Some(Action::OpenUrl {
                label: "Open checks".into(),
                url: "https://github.com/me/red/actions".into()
            })
        );
        assert_eq!(
            b.rows[2].clones.len(),
            1,
            "owner/name matched case-insensitively"
        );
        assert_eq!(b.rows[2].worst().unwrap().kind, ProblemKind::Uncommitted);
        assert!(matches!(
            b.rows[2].action,
            Some(Action::OpenTerminal { .. })
        ));
        assert_eq!(b.rows[3].worst().unwrap().kind, ProblemKind::NotCloned);
        assert!(!b.rows[3].needs_attention());
        assert_eq!(
            b.counts,
            Counts {
                all: 4,
                attention: 3,
                unpushed: 0,
                ci_red: 1,
                alerts: 1,
                not_cloned: 1,
                local_only: 0,
                hidden: 0
            }
        );
    }

    #[test]
    fn hidden_rows_leave_the_counts() {
        let mut red = repo("red");
        red.ci = Ci::Failing;
        let mut b = build(Some(&snap(vec![red, repo("ok")])), vec![], &HashMap::new());
        b.hide(&["me/red".to_owned()]);
        assert!(b.rows[0].hidden);
        assert_eq!((b.counts.all, b.counts.ci_red, b.counts.hidden), (1, 0, 1));
    }

    #[test]
    fn serious_alerts_are_high() {
        let mut r = repo("x");
        r.alerts = Some(Alerts {
            total: 3,
            critical: 1,
            ..Default::default()
        });
        let b = build(
            Some(&snap(vec![r])),
            vec![clone_of("/w/x", Some("me/x"))],
            &HashMap::new(),
        );
        let p = b.rows[0].worst().unwrap();
        assert_eq!((p.kind, p.level), (ProblemKind::Alerts, Level::High));
        assert_eq!(p.text, "3 security alerts (1 critical, 0 high)");
    }

    #[test]
    fn archived_repos_raise_nothing() {
        let mut r = repo("old");
        r.archived = true;
        r.ci = Ci::Failing;
        r.alerts = Some(Alerts {
            total: 5,
            high: 5,
            ..Default::default()
        });
        let b = build(Some(&snap(vec![r])), vec![], &HashMap::new());
        assert!(b.rows[0].problems.is_empty());
        assert_eq!(b.counts.attention, 0);
    }

    #[test]
    fn renamed_remote_joins_the_new_name() {
        let s = snap(vec![repo("new-name")]);
        let renames = HashMap::from([("me/old".to_owned(), "me/new-name".to_owned())]);
        let b = build(Some(&s), vec![clone_of("/w/old", Some("me/old"))], &renames);
        assert_eq!(b.rows.len(), 1);
        assert_eq!(b.rows[0].clones.len(), 1);
        assert_eq!(b.rows[0].worst().unwrap().kind, ProblemKind::OldRemoteName);
    }

    #[test]
    fn local_only_and_foreign_clones() {
        let mut lonely = clone_of("/w/scratch", None);
        lonely.unpushed = 4;
        let upstream = clone_of("/w/vendor-lib", Some("someone/lib"));
        let gone = clone_of("/w/gone", Some("me/gone"));
        let b = build(
            Some(&snap(vec![])),
            vec![lonely, upstream, gone],
            &HashMap::new(),
        );
        let get = |k: &str| b.rows.iter().find(|r| r.key == k).unwrap();

        let l = get("/w/scratch");
        assert_eq!(
            l.problems.iter().map(|p| p.kind).collect::<Vec<_>>(),
            [ProblemKind::LocalOnly, ProblemKind::Unpushed]
        );
        assert_eq!(l.problems[1].text, "scratch: 4 commits not on GitHub");
        assert!(get("someone/lib").foreign && get("someone/lib").problems.is_empty());
        assert!(!get("me/gone").foreign);
        assert_eq!(b.counts.unpushed, 1);
        assert_eq!(b.counts.local_only, 2);
    }

    #[test]
    fn works_before_github_was_ever_read() {
        let mut c = clone_of("/w/a", Some("me/a"));
        c.behind = 1;
        let b = build(None, vec![c], &HashMap::new());
        assert_eq!(b.checked_at, None);
        assert_eq!(b.rows.len(), 1);
        assert_eq!(b.rows[0].worst().unwrap().kind, ProblemKind::Behind);
    }
}
