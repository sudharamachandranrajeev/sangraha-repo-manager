//! Copies of one repository inside another, made with `git subtree`, and whether the copy has
//! fallen behind its source.
//!
//! `git subtree add` leaves `git-subtree-dir` and `git-subtree-split` trailers in its commit.
//! A later `git subtree pull` (without `--squash`) merges the source's commits in without them,
//! so the commit last pulled is found from history: the source's commits are those that
//! descend from the first split but not from the commit that added the folder, and the newest
//! of them is what was pulled last. GitHub then says how far the source has moved on since.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

use crate::github::{Snapshot, call};
use crate::local::{LocalClone, git, github_name};
use crate::{Error, Result};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SubtreeState {
    /// The copy has everything the source's branch has.
    Current,
    /// The source has commits the copy does not.
    Behind,
    /// Not compared yet, or GitHub could not say.
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Subtree {
    /// The repository holding the copy: `owner/name`, or the folder's name.
    pub host: String,
    /// The clone of `host`.
    pub path: PathBuf,
    /// The copy's folder inside it, e.g. `apps/udyoga`.
    pub dir: String,
    /// The source's commit the copy last took in.
    pub pulled: String,
    /// When it was taken in.
    pub pulled_at: Option<String>,
    /// Pulled with `--squash` (the source's history is not in the clone).
    pub squash: bool,
    /// The source, `owner/name`, when Sangraha could tell.
    pub source: Option<String>,
    pub branch: String,
    /// Commits on the source's branch that the copy lacks.
    pub behind: Option<u32>,
    pub state: SubtreeState,
    pub note: Option<String>,
    /// Commits made to the copy here since it was last pulled ("abc1234 subject"). The source
    /// does not have them, and the next pull may conflict with them.
    pub edits: Vec<String>,
}

/// GitHub's answer for one comparison, kept between runs.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Drift {
    pub behind: Option<u32>,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DriftSnapshot {
    /// By [`Subtree::key`].
    pub drift: BTreeMap<String, Drift>,
    pub checked_at: i64,
}

impl Subtree {
    /// What a comparison depends on: the source, the commit pulled and the branch.
    pub fn key(&self) -> Option<String> {
        Some(format!(
            "{}@{}...{}",
            self.source.as_ref()?.to_lowercase(),
            self.pulled,
            self.branch
        ))
    }
}

struct Added {
    commit: String,
    subject: String,
    dir: String,
    split: String,
}

/// The newest `git subtree add` (or squash) commit for each folder, newest first.
fn added(repo: &Path) -> Vec<Added> {
    let Ok(out) = git(
        repo,
        &[
            "log",
            "--grep=git-subtree-dir:",
            "--format=%H%x1f%s%x1f%(trailers:key=git-subtree-dir,valueonly,separator=%x1d)%x1f%(trailers:key=git-subtree-split,valueonly,separator=%x1d)%x1e",
        ],
    ) else {
        return Vec::new();
    };
    let mut seen = HashSet::new();
    out.split('\x1e')
        .filter_map(|rec| {
            let f: Vec<&str> = rec.trim().split('\x1f').collect();
            let [commit, subject, dir, split] = f[..] else {
                return None;
            };
            let dir = dir.split('\x1d').next()?.trim().trim_end_matches('/');
            let split = split.split('\x1d').next()?.trim();
            (!dir.is_empty() && split.len() == 40 && seen.insert(dir.to_owned())).then(|| Added {
                commit: commit.to_owned(),
                subject: subject.to_owned(),
                dir: dir.to_owned(),
                split: split.to_owned(),
            })
        })
        .collect()
}

fn lines(repo: &Path, args: &[&str]) -> Vec<String> {
    git(repo, args)
        .map(|o| o.lines().map(str::to_owned).collect())
        .unwrap_or_default()
}

/// The repository a commit subject says it came from: "… from owner/name main",
/// "… from name main" (the host's owner), or "(subtree of owner/name)".
pub fn named_source(subject: &str, owner: &str) -> Option<(String, Option<String>)> {
    let ok = |s: &str| {
        !s.is_empty()
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/'))
            && s.matches('/').count() <= 1
    };
    if let Some(i) = subject.find("subtree of ") {
        let name = subject[i + 11..]
            .trim_end_matches(')')
            .split_whitespace()
            .next()?;
        return (ok(name) && name.contains('/')).then(|| (name.to_owned(), None));
    }
    let i = subject.rfind(" from ")?;
    let mut words = subject[i + 6..].split_whitespace();
    let name = words.next()?;
    let next = words.next();
    // git's own "… from commit 'abc…'" names no repository.
    if !ok(name) || next.is_some_and(|w| w.starts_with('\'')) {
        return None;
    }
    let full = if name.contains('/') {
        name.to_owned()
    } else if owner.is_empty() {
        return None;
    } else {
        format!("{owner}/{name}")
    };
    Some((full, next.filter(|b| ok(b)).map(str::to_owned)))
}

/// The subtrees in one clone, from its history alone.
pub fn scan(clone: &LocalClone, login: &str) -> Vec<Subtree> {
    let repo = &clone.path;
    let host = clone.github.clone().unwrap_or_else(|| {
        repo.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    });
    let owner = clone
        .github
        .as_ref()
        .and_then(|g| g.split('/').next())
        .unwrap_or(login)
        .to_owned();
    let remotes: HashMap<String, String> = lines(repo, &["remote", "-v"])
        .iter()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            Some((it.next()?.to_owned(), it.next()?.to_owned()))
        })
        .collect();
    let mut out = Vec::new();
    for a in added(repo) {
        if !repo.join(&a.dir).is_dir() {
            continue; // Taken out since.
        }
        let squash = git(
            repo,
            &["cat-file", "-e", &format!("{}^{{commit}}", a.split)],
        )
        .is_err();
        // The source's commits: after the split, not after the add.
        let pulled = if squash {
            a.split.clone()
        } else {
            let host_side: HashSet<String> = lines(
                repo,
                &[
                    "rev-list",
                    "--ancestry-path",
                    &format!("{}..HEAD", a.commit),
                ],
            )
            .into_iter()
            .chain([a.commit.clone()])
            .collect();
            lines(
                repo,
                &[
                    "rev-list",
                    "--topo-order",
                    "--ancestry-path",
                    &format!("{}..HEAD", a.split),
                ],
            )
            .into_iter()
            .find(|c| !host_side.contains(c))
            .unwrap_or_else(|| a.split.clone())
        };
        // The merge that took it in: the oldest merge after it.
        let merge = if squash || pulled == a.split {
            a.commit.clone()
        } else {
            lines(
                repo,
                &[
                    "rev-list",
                    "--ancestry-path",
                    "--merges",
                    "--reverse",
                    &format!("{pulled}..HEAD"),
                ],
            )
            .into_iter()
            .next()
            .unwrap_or_else(|| a.commit.clone())
        };
        let meta = git(repo, &["log", "-1", "--format=%cI%x1f%s", &merge]).unwrap_or_default();
        let (pulled_at, merge_subject) = meta
            .trim()
            .split_once('\x1f')
            .map(|(d, s)| (Some(d.to_owned()), s.to_owned()))
            .unwrap_or((None, String::new()));
        let edits = lines(
            repo,
            &[
                "log",
                "--no-merges",
                "--format=%h %s",
                &format!("{merge}..HEAD"),
                "--",
                &a.dir,
            ],
        );
        let base = a.dir.rsplit('/').next().unwrap_or(&a.dir);
        let from_remote = remotes.get(base).and_then(|u| github_name(u));
        let from_subject =
            named_source(&merge_subject, &owner).or_else(|| named_source(&a.subject, &owner));
        let (source, branch) = match (from_remote, from_subject) {
            (Some(r), s) => (Some(r), s.and_then(|(_, b)| b)),
            (None, Some((r, b))) => (Some(r), b),
            (None, None) => ((!owner.is_empty()).then(|| format!("{owner}/{base}")), None),
        };
        out.push(Subtree {
            host: host.clone(),
            path: repo.clone(),
            dir: a.dir,
            pulled,
            pulled_at,
            squash,
            source,
            branch: branch.unwrap_or_default(),
            behind: None,
            state: SubtreeState::Unknown,
            note: None,
            edits,
        });
    }
    out
}

/// Every clone's subtrees, with sources followed through renames and the branch set to the
/// source's default branch where the snapshot knows it.
pub fn find(
    clones: &[LocalClone],
    snap: Option<&Snapshot>,
    renames: &HashMap<String, String>,
) -> Vec<Subtree> {
    let login = snap.map(|s| s.login.as_str()).unwrap_or("");
    let usable: Vec<&LocalClone> = clones
        .iter()
        .filter(|c| c.error.is_none() && !c.worktree)
        .collect();
    let mut subs: Vec<Subtree> = std::thread::scope(|s| {
        let chunk = usable.len().div_ceil(8).max(1);
        let handles: Vec<_> = usable
            .chunks(chunk)
            .map(|part| {
                s.spawn(move || part.iter().flat_map(|c| scan(c, login)).collect::<Vec<_>>())
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().unwrap_or_default())
            .collect()
    });
    for sub in &mut subs {
        if let Some(src) = &sub.source
            && let Some(now) = renames.get(&src.to_lowercase()).filter(|n| !n.is_empty())
        {
            sub.source = Some(now.clone());
        }
        let default = snap.and_then(|s| {
            let src = sub.source.as_ref()?.to_lowercase();
            s.repos
                .iter()
                .find(|r| r.full_name.to_lowercase() == src)?
                .default_branch
                .clone()
        });
        if let Some(b) = default {
            sub.branch = b;
        } else if sub.branch.is_empty() {
            sub.branch = "main".into();
        }
    }
    // One row per copy: a host cloned twice shows its subtrees once.
    let mut seen = HashSet::new();
    subs.retain(|s| seen.insert((s.host.to_lowercase(), s.dir.clone())));
    subs.sort_by(|a, b| (a.host.to_lowercase(), &a.dir).cmp(&(b.host.to_lowercase(), &b.dir)));
    subs
}

/// Fills in what GitHub said last time.
pub fn apply(subs: &mut [Subtree], snap: &DriftSnapshot) {
    for s in subs {
        let Some(d) = s.key().and_then(|k| snap.drift.get(&k)) else {
            s.note = s
                .source
                .is_none()
                .then(|| "Sangraha cannot tell which repository this came from".into());
            continue;
        };
        s.behind = d.behind;
        s.note = d.note.clone();
        s.state = match d.behind {
            Some(0) => SubtreeState::Current,
            Some(_) => SubtreeState::Behind,
            None => SubtreeState::Unknown,
        };
    }
}

/// Asks GitHub how far each source has moved on since its copy was pulled: one call each.
pub fn compare(token: &str, subs: &[Subtree]) -> Result<DriftSnapshot> {
    let mut snap = DriftSnapshot {
        checked_at: crate::now(),
        ..Default::default()
    };
    for s in subs {
        let (Some(key), Some(src)) = (s.key(), &s.source) else {
            continue;
        };
        if snap.drift.contains_key(&key) || !crate::apps::valid_name(src) {
            continue;
        }
        let r = call(
            token,
            "GET",
            &format!("repos/{src}/compare/{}...{}", s.pulled, s.branch),
            None,
        )?;
        let d = if r.ok() {
            let ahead = r.body["ahead_by"].as_u64().unwrap_or(0) as u32;
            let behind = r.body["behind_by"].as_u64().unwrap_or(0);
            Drift {
                behind: Some(ahead),
                note: (behind > 0).then(|| {
                    format!(
                        "The commit pulled is not on {src}'s {} branch any more ({behind} commits apart)",
                        s.branch
                    )
                }),
            }
        } else if r.status == 404 {
            Drift {
                behind: None,
                note: Some(format!(
                    "{src} does not have the commit pulled, or its {} branch: is this the right source?",
                    s.branch
                )),
            }
        } else {
            Drift {
                behind: None,
                note: Some(r.message()),
            }
        };
        snap.drift.insert(key, d);
    }
    Ok(snap)
}

/// Runs `git subtree pull` for one copy. Refuses a clone with uncommitted changes, as git
/// would. Returns git's output; on a conflict the merge is left for the owner to finish.
pub fn pull(sub: &Subtree) -> Result<String> {
    let src = sub
        .source
        .as_ref()
        .filter(|s| crate::apps::valid_name(s))
        .ok_or_else(|| Error::Git("Sangraha cannot tell which repository this came from".into()))?;
    let dirty = git(
        &sub.path,
        &["status", "--porcelain", "--untracked-files=no"],
    )
    .map_err(Error::Git)?;
    if !dirty.trim().is_empty() {
        return Err(Error::Git(format!(
            "{} has uncommitted changes. Commit or stash them first.",
            sub.path.display()
        )));
    }
    let url = format!("https://github.com/{src}.git");
    let message = format!("Update {} from {src} {}", sub.dir, sub.branch);
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(&sub.path).args([
        "subtree",
        "pull",
        "--prefix",
        &sub.dir,
        &url,
        &sub.branch,
        "-m",
        &message,
    ]);
    if sub.squash {
        cmd.arg("--squash");
    }
    let out = cmd.output()?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
    .trim()
    .to_owned();
    if out.status.success() {
        Ok(text)
    } else {
        Err(Error::Git(if text.is_empty() {
            "git subtree pull failed".into()
        } else {
            text
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sources_named_in_subjects() {
        assert_eq!(
            named_source("Update apps/udyoga from udyoga-career-manager main", "me"),
            Some(("me/udyoga-career-manager".into(), Some("main".into())))
        );
        assert_eq!(
            named_source(
                "Add vidya to the suite as apps/vidya (subtree of me/vidya)",
                "me"
            ),
            Some(("me/vidya".into(), None))
        );
        assert_eq!(
            named_source("Add 'apps/x/' from commit 'abc123'", "me"),
            None
        );
        assert_eq!(
            named_source("Merge x from them/thing", "me"),
            Some(("them/thing".into(), None))
        );
        assert_eq!(named_source("No source here", "me"), None);
    }

    fn run(dir: &Path, args: &[&str]) {
        let ok = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["-c", "user.name=t", "-c", "user.email=t@example.com"])
            .args(args)
            .output()
            .unwrap();
        assert!(
            ok.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&ok.stderr)
        );
    }

    #[test]
    fn finds_the_last_pull_and_local_edits() {
        let has_subtree = Command::new("git")
            .args(["subtree", "-h"])
            .output()
            .is_ok_and(|o| o.status.code() == Some(129));
        if !has_subtree {
            return; // git subtree is not installed here.
        }
        let t = tempfile::tempdir().unwrap();
        let (app, host) = (t.path().join("app"), t.path().join("host"));
        for d in [&app, &host] {
            std::fs::create_dir_all(d).unwrap();
            run(d, &["init", "-q", "-b", "main"]);
        }
        std::fs::write(app.join("a.txt"), "1").unwrap();
        run(&app, &["add", "."]);
        run(&app, &["commit", "-qm", "one"]);
        std::fs::write(host.join("h.txt"), "h").unwrap();
        run(&host, &["add", "."]);
        run(&host, &["commit", "-qm", "host"]);
        let url = app.to_string_lossy().into_owned();
        run(
            &host,
            &[
                "subtree",
                "add",
                "-q",
                "--prefix",
                "apps/app",
                &url,
                "main",
                "-m",
                "Add apps/app from app main",
            ],
        );
        // The source moves on and is pulled; then the copy is edited here.
        std::fs::write(app.join("a.txt"), "2").unwrap();
        run(&app, &["commit", "-qam", "two"]);
        let tip = git(&app, &["rev-parse", "HEAD"]).unwrap().trim().to_owned();
        run(
            &host,
            &[
                "subtree",
                "pull",
                "-q",
                "--prefix",
                "apps/app",
                &url,
                "main",
                "-m",
                "Update apps/app from app main",
            ],
        );
        std::fs::write(host.join("apps/app/a.txt"), "edited here").unwrap();
        run(&host, &["commit", "-qam", "edit the copy"]);

        let clone = LocalClone {
            path: host.clone(),
            github: Some("me/host".into()),
            ..Default::default()
        };
        let subs = scan(&clone, "me");
        assert_eq!(subs.len(), 1);
        let s = &subs[0];
        assert_eq!(s.dir, "apps/app");
        assert_eq!(s.pulled, tip, "the commit last pulled");
        assert!(!s.squash);
        assert_eq!(s.source.as_deref(), Some("me/app"));
        assert_eq!(s.edits.len(), 1, "{:?}", s.edits);
        assert!(s.edits[0].ends_with("edit the copy"));
    }
}
