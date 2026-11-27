//! The Releases tab: for each clone, what has happened since its last tag, whether the version
//! files say the next version yet, a draft changelog, and (on request) the GitHub release's
//! files and the workflow run that built them.
//!
//! Everything but the GitHub details comes from the clones, so it costs no network.

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::apps::compare;
use crate::github::{Release, Snapshot, call};
use crate::local::{LocalClone, git};
use crate::subtrees::Subtree;
use crate::{Error, Result};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct VersionFile {
    /// From the clone's root, e.g. `app/src-tauri/tauri.conf.json`.
    pub path: String,
    pub version: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "camelCase")]
pub enum Verdict {
    /// Version files say different versions.
    Disagree,
    /// Commits since the tag, but the version files still say the tag's version (or older).
    NeedsBump,
    /// The version files are ahead of the last tag: ready to tag.
    Ready,
    /// Never tagged.
    NoTag,
    /// No version file Sangraha reads.
    NoVersion,
    /// Nothing since the last tag.
    NoChanges,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseRow {
    /// `owner/name`, or the folder name for a clone without a GitHub remote.
    pub repo: String,
    pub path: PathBuf,
    pub branch: Option<String>,
    /// The newest tag reachable from the checked-out commit.
    pub tag: Option<String>,
    pub tag_date: Option<String>,
    pub tag_sha: Option<String>,
    /// Commits since the tag (all commits when there is none).
    pub since: u32,
    /// Pull requests among them (merges and squash merges that name one).
    pub pull_requests: u32,
    pub versions: Vec<VersionFile>,
    /// The version the files agree on (the root file's, when they do not).
    pub version: Option<String>,
    pub verdict: Verdict,
    pub note: String,
    /// The newest release on GitHub, from the last snapshot.
    pub release: Option<Release>,
    /// The last tag is newer than GitHub's newest release: tagged, not released.
    pub unreleased_tag: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseCounts {
    pub ready: u32,
    pub needs_bump: u32,
    pub disagree: u32,
    pub unreleased: u32,
    pub drift: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseBoard {
    pub rows: Vec<ReleaseRow>,
    pub subtrees: Vec<Subtree>,
    pub counts: ReleaseCounts,
    /// When the subtrees were last compared with their sources on GitHub (Unix seconds).
    pub checked_at: Option<i64>,
    pub warnings: Vec<String>,
}

// ---------------------------------------------------------------------------------------------
// Version files

/// Folders whose manifests belong to something else: vendored packages, examples, builds.
const SKIP: &[&str] = &[
    "node_modules",
    "vendor",
    "third_party",
    "packages",
    "plugins",
    ".dart_tool",
    "build",
    "dist",
    "target",
    "example",
    "examples",
    "fixtures",
    "testdata",
    "test",
    "tests",
];

fn version_names(path: &str) -> bool {
    let parts: Vec<&str> = path.split('/').collect();
    let name = parts[parts.len() - 1];
    parts.len() <= 3
        && !parts[..parts.len() - 1].iter().any(|p| SKIP.contains(p))
        && matches!(
            name,
            "Cargo.toml" | "package.json" | "pubspec.yaml" | "tauri.conf.json"
        )
}

/// The version a manifest states, if it states one of its own.
pub fn read_version(name: &str, text: &str) -> Option<String> {
    let found = match name {
        "Cargo.toml" => {
            // `version = "…"` in [package] or [workspace.package]; `version.workspace = true`
            // states none.
            let mut section = "";
            let mut found = None;
            for line in text.lines().map(str::trim) {
                if line.starts_with('[') {
                    section = line;
                } else if (section == "[package]" || section == "[workspace.package]")
                    && let Some(rest) = line.strip_prefix("version")
                    && let Some(v) = rest.trim_start().strip_prefix('=')
                {
                    found = Some(v.trim().trim_matches('"').to_owned());
                    break;
                }
            }
            found
        }
        "package.json" | "tauri.conf.json" => serde_json::from_str::<serde_json::Value>(text)
            .ok()
            .and_then(|v| v["version"].as_str().map(str::to_owned))
            // A tauri.conf.json may point at package.json instead; a starter's 0.0.0 says
            // nothing.
            .filter(|v| !v.ends_with(".json") && v != "0.0.0"),
        "pubspec.yaml" => text
            .lines()
            .find_map(|l| l.strip_prefix("version:"))
            .map(|v| v.trim().trim_matches(['"', '\'']).to_owned()),
        _ => None,
    };
    found.filter(|v| !v.is_empty())
}

fn versions(dir: &Path) -> Vec<VersionFile> {
    let Ok(list) = git(
        dir,
        &[
            "ls-files",
            "--",
            "Cargo.toml",
            "*/Cargo.toml",
            "package.json",
            "*/package.json",
            "pubspec.yaml",
            "*/pubspec.yaml",
            "*/tauri.conf.json",
        ],
    ) else {
        return Vec::new();
    };
    let mut out: Vec<VersionFile> = list
        .lines()
        .filter(|p| version_names(p))
        .filter_map(|p| {
            let text = std::fs::read_to_string(dir.join(p)).ok()?;
            let name = p.rsplit('/').next().unwrap_or(p);
            Some(VersionFile {
                path: p.to_owned(),
                version: read_version(name, &text)?,
            })
        })
        .collect();
    // Root files first: they speak for the project.
    out.sort_by_key(|f| (f.path.matches('/').count(), f.path.clone()));
    out
}

/// Two versions are the same release if they order equal (`0.5.1+7` and `v0.5.1`).
fn same(a: &str, b: &str) -> bool {
    compare(a, b).map_or(a == b, |o| o == Ordering::Equal)
}

/// The file that speaks for the release: the one closest to the last tag from above (the tag's
/// version itself, or the next one), else the shallowest. A monorepo's other packages keep
/// their own versions.
pub fn primary<'a>(tag: Option<&str>, versions: &'a [VersionFile]) -> Option<&'a VersionFile> {
    let at_or_after = |f: &&VersionFile| {
        tag.is_some_and(|t| {
            matches!(
                compare(&f.version, t),
                Some(Ordering::Equal | Ordering::Greater)
            )
        })
    };
    versions
        .iter()
        .filter(at_or_after)
        .min_by(|a, b| compare(&a.version, &b.version).unwrap_or(Ordering::Equal))
        .or_else(|| versions.first())
}

fn dir_of(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(d, _)| d)
}

/// Files that must say the same version: the primary file's folder, and each Tauri app's
/// `tauri.conf.json` with its `package.json` and its crate's (or workspace's) Cargo.toml.
fn mismatch<'a>(
    main: &'a VersionFile,
    versions: &'a [VersionFile],
) -> Option<(&'a VersionFile, &'a VersionFile)> {
    let find = |p: &str| versions.iter().find(|f| f.path == p);
    for f in versions
        .iter()
        .filter(|f| dir_of(&f.path) == dir_of(&main.path))
    {
        if !same(&f.version, &main.version) {
            return Some((main, f));
        }
    }
    for conf in versions
        .iter()
        .filter(|f| f.path.ends_with("tauri.conf.json"))
    {
        let crate_dir = dir_of(&conf.path);
        let app_dir = dir_of(crate_dir);
        let package = if app_dir.is_empty() {
            "package.json".to_owned()
        } else {
            format!("{app_dir}/package.json")
        };
        let cargo = find(&format!("{crate_dir}/Cargo.toml")).or_else(|| find("Cargo.toml"));
        for other in [find(&package), cargo].into_iter().flatten() {
            if !same(&other.version, &conf.version) {
                return Some((conf, other));
            }
        }
    }
    None
}

/// The verdict for a clone and a sentence that says it.
pub fn verdict(tag: Option<&str>, since: u32, versions: &[VersionFile]) -> (Verdict, String) {
    let Some(main) = primary(tag, versions) else {
        return (
            Verdict::NoVersion,
            "No version file (Cargo.toml, package.json, pubspec.yaml) states a version".into(),
        );
    };
    let v = &main.version;
    if let Some((a, b)) = mismatch(main, versions) {
        return (
            Verdict::Disagree,
            format!(
                "{} says {}, {} says {}",
                a.path, a.version, b.path, b.version
            ),
        );
    }
    let Some(tag) = tag else {
        return (Verdict::NoTag, format!("Never tagged; the files say {v}"));
    };
    if since == 0 {
        return (Verdict::NoChanges, format!("Nothing since {tag}"));
    }
    match compare(v, tag) {
        Some(Ordering::Greater) => (
            Verdict::Ready,
            format!("Ready to tag: the files say {v}, newer than {tag}"),
        ),
        Some(_) => (
            Verdict::NeedsBump,
            format!(
                "{since} commits since {tag}, but the files still say {v}: raise the version first"
            ),
        ),
        None => (
            Verdict::NeedsBump,
            format!("{since} commits since {tag}; the files say {v}"),
        ),
    }
}

// ---------------------------------------------------------------------------------------------
// Rows

fn row(c: &LocalClone, snap: Option<&Snapshot>) -> Option<ReleaseRow> {
    let dir = &c.path;
    let tag = git(dir, &["describe", "--tags", "--abbrev=0"])
        .ok()
        .map(|t| t.trim().to_owned())
        .filter(|t| !t.is_empty());
    let versions = versions(dir);
    if tag.is_none() && versions.is_empty() {
        return None;
    }
    let range = tag
        .as_ref()
        .map_or("HEAD".to_owned(), |t| format!("{t}..HEAD"));
    let since = git(dir, &["rev-list", "--count", &range])
        .ok()
        .and_then(|n| n.trim().parse().ok())
        .unwrap_or(0);
    let subjects = git(dir, &["log", "--format=%s", &range]).unwrap_or_default();
    let pull_requests = subjects.lines().filter(|s| pr_number(s).is_some()).count() as u32;
    let (tag_date, tag_sha) = match &tag {
        Some(t) => {
            let out = git(dir, &["log", "-1", "--format=%cI%x1f%H", t]).unwrap_or_default();
            let mut it = out.trim().split('\x1f');
            (
                it.next().map(str::to_owned).filter(|s| !s.is_empty()),
                it.next().map(str::to_owned),
            )
        }
        None => (None, None),
    };
    let release = snap.and_then(|s| {
        let g = c.github.as_ref()?.to_lowercase();
        s.repos
            .iter()
            .find(|r| r.full_name.to_lowercase() == g)?
            .release
            .clone()
    });
    let unreleased_tag = match (&tag, &release) {
        (Some(t), Some(r)) => compare(t, &r.tag) == Some(Ordering::Greater),
        (Some(_), None) => c.github.is_some(),
        _ => false,
    };
    let (verdict, note) = verdict(tag.as_deref(), since, &versions);
    Some(ReleaseRow {
        repo: c.github.clone().unwrap_or_else(|| {
            dir.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        }),
        path: dir.clone(),
        branch: c.branch.clone(),
        version: primary(tag.as_deref(), &versions).map(|f| f.version.clone()),
        tag,
        tag_date,
        tag_sha,
        since,
        pull_requests,
        versions,
        verdict,
        note,
        release,
        unreleased_tag,
    })
}

/// The rows for the owner's clones (and clones with no GitHub remote), one per repository.
pub fn rows(clones: &[LocalClone], snap: Option<&Snapshot>) -> Vec<ReleaseRow> {
    let login = snap.map(|s| format!("{}/", s.login.to_lowercase()));
    let mut seen = std::collections::HashSet::new();
    let mine: Vec<&LocalClone> = clones
        .iter()
        .filter(|c| c.error.is_none() && !c.worktree)
        .filter(|c| match (&c.github, &login) {
            (Some(g), Some(l)) => g.to_lowercase().starts_with(l.as_str()),
            (Some(_), None) => true,
            (None, _) => c.other_remote.is_none(),
        })
        .filter(|c| {
            seen.insert(
                c.github
                    .clone()
                    .unwrap_or_else(|| c.path.display().to_string()),
            )
        })
        .collect();
    let mut out: Vec<ReleaseRow> = std::thread::scope(|s| {
        let chunk = mine.len().div_ceil(8).max(1);
        let handles: Vec<_> = mine
            .chunks(chunk)
            .map(|part| {
                s.spawn(move || part.iter().filter_map(|c| row(c, snap)).collect::<Vec<_>>())
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().unwrap_or_default())
            .collect()
    });
    out.sort_by(|a, b| {
        a.verdict
            .cmp(&b.verdict)
            .then(b.since.cmp(&a.since))
            .then(a.repo.to_lowercase().cmp(&b.repo.to_lowercase()))
    });
    out
}

pub fn count(rows: &[ReleaseRow], subtrees: &[Subtree]) -> ReleaseCounts {
    let n = |v: Verdict| rows.iter().filter(|r| r.verdict == v).count() as u32;
    ReleaseCounts {
        ready: n(Verdict::Ready),
        needs_bump: n(Verdict::NeedsBump),
        disagree: n(Verdict::Disagree),
        unreleased: rows.iter().filter(|r| r.unreleased_tag).count() as u32,
        drift: subtrees
            .iter()
            .filter(|s| s.behind.unwrap_or(0) > 0 || !s.edits.is_empty())
            .count() as u32,
    }
}

// ---------------------------------------------------------------------------------------------
// Changelog

/// `#12` from "Merge pull request #12 from …" or "Fix the thing (#12)".
fn pr_number(subject: &str) -> Option<u32> {
    let digits = |s: &str| -> Option<u32> {
        let d: String = s.chars().take_while(char::is_ascii_digit).collect();
        d.parse().ok()
    };
    if let Some(rest) = subject.strip_prefix("Merge pull request #") {
        return digits(rest);
    }
    let open = subject.rfind("(#")?;
    subject
        .ends_with(')')
        .then(|| digits(&subject[open + 2..]))?
}

/// The heading a conventional-commit type goes under; `None` for chores nobody reads.
fn heading(kind: &str) -> Option<&'static str> {
    match kind {
        "feat" | "add" => Some("Added"),
        "fix" | "bug" => Some("Fixed"),
        "sec" | "security" => Some("Security"),
        "perf" | "refactor" | "change" | "ui" => Some("Changed"),
        "docs" | "doc" => Some("Documentation"),
        "chore" | "ci" | "build" | "test" | "tests" | "style" => None,
        _ => Some("Changed"),
    }
}

/// A draft changelog from commit subjects: grouped by their `type:` prefix where they have one,
/// merges left out, squash-merged pull requests linked by number.
pub fn changelog(title: &str, subjects: &[(String, String)]) -> String {
    let mut groups: BTreeMap<usize, (&str, Vec<String>)> = BTreeMap::new();
    let order = ["Added", "Changed", "Fixed", "Security", "Documentation"];
    for (sha, subject) in subjects {
        if subject.starts_with("Merge ") {
            continue;
        }
        let (kind, text) = match subject.split_once(':') {
            Some((k, t))
                if !k.is_empty()
                    && k.len() <= 20
                    && k.trim_end_matches('!')
                        .split('(')
                        .next()
                        .is_some_and(|w| w.chars().all(|c| c.is_ascii_alphabetic())) =>
            {
                let k = k.trim_end_matches('!');
                (
                    k.split('(').next().unwrap_or(k).to_ascii_lowercase(),
                    t.trim(),
                )
            }
            _ => (String::new(), subject.as_str()),
        };
        let Some(h) = heading(&kind) else { continue };
        let mut chars = text.chars();
        let text = match chars.next() {
            Some(f) => f.to_uppercase().chain(chars).collect::<String>(),
            None => continue,
        };
        let i = order.iter().position(|o| *o == h).unwrap_or(1);
        groups
            .entry(i)
            .or_insert((h, Vec::new()))
            .1
            .push(format!("- {text} ({sha})"));
    }
    let mut out = format!("## {title}\n");
    if groups.is_empty() {
        out.push_str("\nNo changes worth listing.\n");
    }
    for (_, (h, lines)) in groups {
        out.push_str(&format!("\n### {h}\n\n{}\n", lines.join("\n")));
    }
    out
}

/// The draft changelog for the commits since `path`'s last tag.
pub fn changelog_for(path: &Path) -> Result<String> {
    let tag = git(path, &["describe", "--tags", "--abbrev=0"])
        .ok()
        .map(|t| t.trim().to_owned());
    let range = tag
        .as_ref()
        .map_or("HEAD".to_owned(), |t| format!("{t}..HEAD"));
    let out =
        git(path, &["log", "--no-merges", "--format=%h%x1f%s", &range]).map_err(Error::Git)?;
    let subjects: Vec<(String, String)> = out
        .lines()
        .filter_map(|l| l.split_once('\x1f'))
        .map(|(h, s)| (h.to_owned(), s.to_owned()))
        .collect();
    let files = versions(path);
    let title = primary(tag.as_deref(), &files)
        .map(|f| f.version.clone())
        .filter(|v| tag.as_ref().is_none_or(|t| !same(v, t)))
        .unwrap_or_else(|| "Unreleased".into());
    Ok(changelog(&title, &subjects))
}

// ---------------------------------------------------------------------------------------------
// GitHub details, read when a row is opened

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Asset {
    pub name: String,
    pub size: u64,
    pub downloads: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Run {
    pub name: String,
    /// `queued`, `in_progress` or `completed`.
    pub status: String,
    /// `success`, `failure`, `cancelled`… once completed.
    pub conclusion: Option<String>,
    pub url: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Details {
    /// `None` when the tag has no GitHub release.
    pub release_url: Option<String>,
    pub prerelease: bool,
    pub assets: Vec<Asset>,
    /// Workflow runs on the tagged commit (the release build among them).
    pub runs: Vec<Run>,
}

/// The release for `tag` and the workflow runs on `sha`. Two REST calls.
pub fn details(token: &str, repo: &str, tag: &str, sha: &str) -> Result<Details> {
    if !crate::apps::valid_name(repo) {
        return Err(Error::GitHub(format!("{repo} is not owner/name")));
    }
    let mut d = Details::default();
    let tag_path: String = tag
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect();
    let r = call(
        token,
        "GET",
        &format!("repos/{repo}/releases/tags/{tag_path}"),
        None,
    )?;
    if r.ok() {
        d.release_url = r.body["html_url"].as_str().map(str::to_owned);
        d.prerelease = r.body["prerelease"].as_bool().unwrap_or(false);
        d.assets = r.body["assets"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|a| Asset {
                name: a["name"].as_str().unwrap_or_default().to_owned(),
                size: a["size"].as_u64().unwrap_or(0),
                downloads: a["download_count"].as_u64().unwrap_or(0),
            })
            .collect();
    } else if r.status != 404 {
        return Err(Error::GitHub(r.message()));
    }
    if sha.len() == 40 && sha.bytes().all(|b| b.is_ascii_hexdigit()) {
        let r = call(
            token,
            "GET",
            &format!("repos/{repo}/actions/runs?head_sha={sha}&per_page=10"),
            None,
        )?;
        d.runs = r.body["workflow_runs"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|w| Run {
                name: w["name"].as_str().unwrap_or_default().to_owned(),
                status: w["status"].as_str().unwrap_or_default().to_owned(),
                conclusion: w["conclusion"].as_str().map(str::to_owned),
                url: w["html_url"].as_str().unwrap_or_default().to_owned(),
            })
            .collect();
    }
    Ok(d)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vf(path: &str, version: &str) -> VersionFile {
        VersionFile {
            path: path.into(),
            version: version.into(),
        }
    }

    #[test]
    fn versions_from_manifests() {
        let cargo = "[workspace]\nmembers = [\"core\"]\n\n[workspace.package]\nversion = \"0.1.0-alpha.1\"\nedition = \"2024\"\n";
        assert_eq!(
            read_version("Cargo.toml", cargo).as_deref(),
            Some("0.1.0-alpha.1")
        );
        assert_eq!(
            read_version(
                "Cargo.toml",
                "[package]\nname = \"x\"\nversion.workspace = true\n"
            ),
            None
        );
        assert_eq!(
            read_version(
                "Cargo.toml",
                "[dependencies]\nversion = \"9\"\n[package]\nversion = \"1.2.0\"\n"
            )
            .as_deref(),
            Some("1.2.0")
        );
        assert_eq!(
            read_version("pubspec.yaml", "name: x\nversion: 0.5.1+7\n").as_deref(),
            Some("0.5.1+7")
        );
        assert_eq!(
            read_version("package.json", r#"{"version":"0.0.0","private":true}"#),
            None
        );
        assert_eq!(
            read_version("tauri.conf.json", r#"{"version":"../package.json"}"#),
            None
        );
        assert!(version_names("app/src-tauri/tauri.conf.json"));
        assert!(!version_names("packages/thing/pubspec.yaml"));
        assert!(!version_names("a/b/c/package.json"));
    }

    #[test]
    fn verdicts() {
        let v = [vf("Cargo.toml", "0.2.0"), vf("app/package.json", "0.2.0")];
        assert_eq!(verdict(Some("v0.1.0"), 4, &v).0, Verdict::Ready);
        assert_eq!(verdict(Some("v0.2.0"), 4, &v).0, Verdict::NeedsBump);
        assert_eq!(verdict(Some("v0.2.0"), 0, &v).0, Verdict::NoChanges);
        assert_eq!(verdict(None, 9, &v).0, Verdict::NoTag);
        assert_eq!(verdict(Some("v0.1.0"), 1, &[]).0, Verdict::NoVersion);
        let odd = [vf("Cargo.toml", "0.2.0"), vf("package.json", "0.1.9")];
        let (d, note) = verdict(Some("v0.1.0"), 1, &odd);
        assert_eq!(d, Verdict::Disagree);
        assert!(note.contains("package.json says 0.1.9"), "{note}");
        assert_eq!(
            verdict(Some("v0.5.1"), 2, &[vf("pubspec.yaml", "0.5.1+8")]).0,
            Verdict::NeedsBump
        );
    }

    #[test]
    fn changelog_groups_by_type() {
        let s = |h: &str, t: &str| (h.to_owned(), t.to_owned());
        let log = changelog(
            "0.2.0",
            &[
                s("a1", "feat(ui): add a dark theme"),
                s("b2", "fix: crash on empty list (#12)"),
                s("c3", "chore: bump deps"),
                s("d4", "Merge pull request #13 from me/x"),
                s("e5", "Phase 4: bulk settings"),
                s("f6", "Better errors"),
            ],
        );
        assert!(log.starts_with("## 0.2.0\n"), "{log}");
        let added = log.find("### Added").unwrap();
        let changed = log.find("### Changed").unwrap();
        let fixed = log.find("### Fixed").unwrap();
        assert!(added < changed && changed < fixed, "{log}");
        assert!(log.contains("- Add a dark theme (a1)"));
        assert!(log.contains("- Crash on empty list (#12) (b2)"));
        assert!(
            log.contains("- Phase 4: bulk settings (e5)"),
            "a phrase with a space is no type: {log}"
        );
        assert!(!log.contains("bump deps") && !log.contains("Merge"));
    }

    #[test]
    fn pull_request_numbers() {
        assert_eq!(pr_number("Merge pull request #13 from me/x"), Some(13));
        assert_eq!(pr_number("Fix it (#7)"), Some(7));
        assert_eq!(pr_number("Fix (#7) later"), None);
        assert_eq!(pr_number("Plain"), None);
    }
}
