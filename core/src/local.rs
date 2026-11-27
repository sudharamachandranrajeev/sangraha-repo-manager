//! The local clones: found by walking the configured folders, read with the `git` command.
//!
//! Reading only. `fetch` is the one network action, and it only updates remote-tracking refs;
//! it never touches the working tree. Folders shared by Syncthing are walked like any other and
//! never changed.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

const DEPTH: usize = 3;
/// Big build or dependency folders that never hold a clone worth listing.
const SKIP: &[&str] = &[
    "node_modules",
    "target",
    "build",
    "dist",
    "vendor",
    "_vendor",
    "venv",
];

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LocalClone {
    pub path: PathBuf,
    /// `owner/name` from the `origin` remote, when it is on github.com.
    pub github: Option<String>,
    /// The `origin` URL when it is somewhere other than github.com.
    pub other_remote: Option<String>,
    /// `None` when detached.
    pub branch: Option<String>,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    /// Commits on any local branch that no remote has.
    pub unpushed: u32,
    /// Changed or staged tracked files, conflicts included.
    pub changed: u32,
    pub untracked: u32,
    pub stashes: u32,
    /// A linked worktree (its `.git` is a file).
    pub worktree: bool,
    /// Why git could not read it, if it could not.
    pub error: Option<String>,
}

/// Every clone under the roots, sorted by path. Clones are read in parallel.
pub fn scan(roots: &[PathBuf]) -> Vec<LocalClone> {
    let found = find(roots);
    let workers = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .min(8);
    let chunk = found.len().div_ceil(workers).max(1);
    let mut clones: Vec<LocalClone> = std::thread::scope(|s| {
        let handles: Vec<_> = found
            .chunks(chunk)
            .map(|paths| s.spawn(move || paths.iter().map(|p| read(p)).collect::<Vec<_>>()))
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().unwrap_or_default())
            .collect()
    });
    clones.sort_by(|a, b| a.path.cmp(&b.path));
    clones
}

/// The clone folders under the roots, sorted, without reading them.
pub fn find(roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for root in roots {
        walk(root, 0, &mut found);
    }
    found.sort();
    found.dedup();
    found
}

fn walk(dir: &Path, depth: usize, found: &mut Vec<PathBuf>) {
    if is_clone(dir) {
        found.push(dir.to_path_buf());
    }
    if depth >= DEPTH {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let name = e.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') || SKIP.contains(&name.as_ref()) {
            continue;
        }
        // Not following links keeps the walk out of loops and out of other disks.
        if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            walk(&e.path(), depth + 1, found);
        }
    }
}

/// A real clone has `.git/HEAD`, or a `.git` file pointing at a worktree's git folder. A `.git`
/// folder without `HEAD` is a leftover (a partial copy, say) and git cannot use it.
fn is_clone(dir: &Path) -> bool {
    let dot = dir.join(".git");
    if dot.is_dir() {
        return dot.join("HEAD").is_file();
    }
    std::fs::read(&dot).is_ok_and(|b| b.starts_with(b"gitdir:"))
}

pub(crate) fn git(path: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(args)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .map_err(|e| format!("git is not installed ({e})"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&out.stderr)
            .trim()
            .lines()
            .last()
            .unwrap_or("failed")
            .to_owned())
    }
}

/// Reads one clone. Never fails: a clone git cannot read comes back with `error` set.
pub fn read(path: &Path) -> LocalClone {
    let mut c = LocalClone {
        path: path.to_path_buf(),
        worktree: path.join(".git").is_file(),
        ..Default::default()
    };
    match git(path, &["status", "--porcelain=v2", "--branch"]) {
        Ok(out) => apply_status(&out, &mut c),
        Err(e) => {
            c.error = Some(e);
            return c;
        }
    }
    if let Ok(url) = git(path, &["config", "--get", "remote.origin.url"]) {
        let url = url.trim();
        match github_name(url) {
            Some(n) => c.github = Some(n),
            None if !url.is_empty() => c.other_remote = Some(url.to_owned()),
            None => {}
        }
    }
    // Commits on local branches that no remote-tracking ref contains.
    if let Ok(n) = git(
        path,
        &["rev-list", "--count", "--branches", "--not", "--remotes"],
    ) {
        c.unpushed = n.trim().parse().unwrap_or(0);
    }
    if let Ok(list) = git(path, &["stash", "list"]) {
        c.stashes = list.lines().count() as u32;
    }
    c
}

/// Reads `git status --porcelain=v2 --branch`.
pub fn apply_status(out: &str, c: &mut LocalClone) {
    for line in out.lines() {
        if let Some(head) = line.strip_prefix("# branch.head ") {
            c.branch = (head != "(detached)").then(|| head.to_owned());
        } else if let Some(up) = line.strip_prefix("# branch.upstream ") {
            c.upstream = Some(up.to_owned());
        } else if let Some(ab) = line.strip_prefix("# branch.ab ") {
            for part in ab.split_whitespace() {
                if let Some(n) = part.strip_prefix('+') {
                    c.ahead = n.parse().unwrap_or(0);
                } else if let Some(n) = part.strip_prefix('-') {
                    c.behind = n.parse().unwrap_or(0);
                }
            }
        } else if line.starts_with("1 ") || line.starts_with("2 ") || line.starts_with("u ") {
            c.changed += 1;
        } else if line.starts_with("? ") {
            c.untracked += 1;
        }
    }
}

/// `owner/name` from a github.com remote URL (https, ssh or scp style), without `.git`.
pub fn github_name(url: &str) -> Option<String> {
    let url = url.trim();
    let rest = [
        "https://github.com/",
        "http://github.com/",
        "ssh://git@github.com/",
        "git@github.com:",
        "git://github.com/",
    ]
    .iter()
    .find_map(|p| url.strip_prefix(p))
    .or_else(|| {
        // https://user@github.com/owner/name
        let after = url.strip_prefix("https://")?;
        let (cred, path) = after.split_once('@')?;
        (!cred.contains('/'))
            .then_some(path)?
            .strip_prefix("github.com/")
    })?;
    let rest = rest.trim_end_matches('/');
    let rest = rest.strip_suffix(".git").unwrap_or(rest);
    let mut parts = rest.split('/');
    let (owner, name) = (parts.next()?, parts.next()?);
    (!owner.is_empty() && !name.is_empty() && parts.next().is_none())
        .then(|| format!("{owner}/{name}"))
}

/// `git fetch --prune` in every clone, a few at a time. Returns the clones that failed and why.
pub fn fetch(paths: &[PathBuf]) -> Vec<(PathBuf, String)> {
    let chunk = paths.len().div_ceil(6).max(1);
    std::thread::scope(|s| {
        let handles: Vec<_> = paths
            .chunks(chunk)
            .map(|ps| {
                s.spawn(move || {
                    ps.iter()
                        .filter_map(|p| {
                            git(p, &["fetch", "--prune", "--quiet"])
                                .err()
                                .map(|e| (p.clone(), e))
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().unwrap_or_default())
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_names() {
        for (url, want) in [
            ("https://github.com/o/r.git", Some("o/r")),
            ("https://github.com/o/r", Some("o/r")),
            ("https://github.com/o/r/", Some("o/r")),
            ("git@github.com:o/r.git", Some("o/r")),
            ("ssh://git@github.com/o/r.git", Some("o/r")),
            ("https://tok@github.com/o/r.git", Some("o/r")),
            ("https://gitlab.com/o/r.git", None),
            ("https://github.com/o", None),
            ("https://github.com/o/r/tree/main", None),
            ("", None),
        ] {
            assert_eq!(github_name(url).as_deref(), want, "{url}");
        }
    }

    #[test]
    fn porcelain_v2() {
        let out = "# branch.oid 1234\n# branch.head main\n# branch.upstream origin/main\n# branch.ab +2 -5\n\
                   1 .M N... 100644 100644 100644 a b src/x.rs\n2 R. N... 100644 100644 100644 a b R100 y z\n\
                   u UU N... 1 2 3 4 a b c w\n? new.txt\n? other\n";
        let mut c = LocalClone::default();
        apply_status(out, &mut c);
        assert_eq!(c.branch.as_deref(), Some("main"));
        assert_eq!(c.upstream.as_deref(), Some("origin/main"));
        assert_eq!((c.ahead, c.behind, c.changed, c.untracked), (2, 5, 3, 2));

        let mut d = LocalClone::default();
        apply_status("# branch.oid 1\n# branch.head (detached)\n", &mut d);
        assert_eq!(d.branch, None);
        assert_eq!(d.upstream, None);
    }

    fn run(dir: &Path, args: &[&str]) {
        let ok = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@example.com")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@example.com")
            .status()
            .unwrap()
            .success();
        assert!(ok, "git {args:?}");
    }

    #[test]
    fn scans_real_clones() {
        let root = tempfile::tempdir().unwrap();
        let a = root.path().join("a");
        let nested = root.path().join("group/b");
        let skipped = root.path().join("node_modules/c");
        for d in [&a, &nested, &skipped] {
            std::fs::create_dir_all(d).unwrap();
            run(d, &["init", "-q", "-b", "main"]);
        }
        // A partial copy: `.git` without HEAD is not a clone.
        std::fs::create_dir_all(root.path().join("leftover/.git/objects")).unwrap();
        run(&a, &["remote", "add", "origin", "git@github.com:o/a.git"]);
        std::fs::write(a.join("f"), "1").unwrap();
        run(&a, &["add", "f"]);
        run(&a, &["commit", "-qm", "one"]);
        std::fs::write(a.join("f"), "2").unwrap();
        std::fs::write(a.join("g"), "new").unwrap();
        run(&a, &["stash", "push", "-q", "--", "f"]);
        std::fs::write(a.join("f"), "3").unwrap();

        let clones = scan(&[root.path().to_path_buf()]);
        let paths: Vec<_> = clones.iter().map(|c| c.path.clone()).collect();
        assert_eq!(paths, vec![a.clone(), nested.clone()]);
        let ca = &clones[0];
        assert_eq!(ca.github.as_deref(), Some("o/a"));
        assert_eq!(ca.branch.as_deref(), Some("main"));
        assert_eq!(
            (ca.changed, ca.untracked, ca.stashes, ca.unpushed),
            (1, 1, 1, 1)
        );
        assert!(ca.error.is_none() && !ca.worktree);
        let cb = &clones[1];
        assert_eq!(
            (cb.github.as_deref(), cb.other_remote.as_deref()),
            (None, None)
        );
    }
}
