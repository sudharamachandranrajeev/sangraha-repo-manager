//! Housekeeping: branches that were merged or went quiet, repositories without a README,
//! licence or CI, old workflow runs, and large files in the clones.
//!
//! One GraphQL query per 25 repositories reads the branches (with the pull requests made from
//! them) and the top of each repository's file tree. Old workflow runs cost one REST call per
//! repository that has workflows. Large files come from the clones, without network.
//!
//! The one change Sangraha makes here is deleting merged branches, and only a branch whose
//! newest commit is still the one a merged pull request brought in, so nothing is lost.

use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::github::{RemoteRepo, call};
use crate::local::{LocalClone, git};
use crate::{Error, Result};

const BATCH: usize = 25;

/// A branch with no commit for this long, no open pull request and nothing merged counts as
/// stale.
pub const STALE_DAYS: i64 = 90;
/// Workflow runs older than this count as old.
pub const OLD_RUN_DAYS: i64 = 90;
/// Files at least this large are listed (GitHub warns at 50 MB and refuses 100 MB).
pub const LARGE_FILE: u64 = 20 * 1024 * 1024;

const FRAGMENT: &str = r#"
fragment Tidy on Repository {
  nameWithOwner isPrivate isFork
  licenseInfo { spdxId }
  defaultBranchRef { name }
  root: object(expression: "HEAD:") { ... on Tree { entries { name } } }
  workflows: object(expression: "HEAD:.github/workflows") { ... on Tree { entries { name } } }
  refs(refPrefix: "refs/heads/", first: 50) {
    totalCount
    nodes {
      name
      target { oid ... on Commit { committedDate } }
      merged: associatedPullRequests(states: [MERGED], first: 1, orderBy: {field: UPDATED_AT, direction: DESC}) {
        nodes { number headRefOid }
      }
      open: associatedPullRequests(states: [OPEN]) { totalCount }
    }
  }
}"#;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Branch {
    pub name: String,
    /// The newest commit, as GitHub last saw it.
    pub oid: String,
    pub last_commit: Option<String>,
    /// The merged pull request that brought this branch in, when its newest commit is the one
    /// merged.
    pub pull_request: Option<u32>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LargeFile {
    pub path: String,
    pub size: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TidyRepo {
    pub repo: String,
    pub private: bool,
    /// Every branch on GitHub, the default one included.
    pub branches: u32,
    /// Merged through a pull request and not changed since: safe to delete.
    pub merged: Vec<Branch>,
    /// Quiet for [`STALE_DAYS`], with no open or merged pull request.
    pub stale: Vec<Branch>,
    /// "README", "LICENSE" (public repositories only), "CI".
    pub missing: Vec<String>,
    /// Workflow runs older than [`OLD_RUN_DAYS`]; `None` when not counted.
    pub old_runs: Option<u32>,
    /// From a clone here.
    pub large_files: Vec<LargeFile>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TidySnapshot {
    pub repos: Vec<TidyRepo>,
    pub checked_at: i64,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TidyCounts {
    pub merged: u32,
    pub stale: u32,
    pub no_readme: u32,
    pub no_license: u32,
    pub no_ci: u32,
    pub old_runs: u32,
    pub large_files: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TidyBoard {
    /// Repositories with something to tidy, most first.
    pub repos: Vec<TidyRepo>,
    pub counts: TidyCounts,
    pub checked_at: Option<i64>,
    pub warnings: Vec<String>,
}

fn days_ago(iso: &str, now: i64) -> Option<i64> {
    crate::parse_time(iso).map(|t| (now - t) / 86_400)
}

/// One repository's answer from the query.
pub fn parse_repo(r: &Value, now: i64) -> Option<TidyRepo> {
    let repo = r["nameWithOwner"].as_str()?.to_owned();
    let private = r["isPrivate"].as_bool().unwrap_or(false);
    let default = r["defaultBranchRef"]["name"].as_str().unwrap_or("");
    let names: Vec<String> = r["root"]["entries"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|e| e["name"].as_str().map(str::to_ascii_lowercase))
        .collect();
    let has = |prefixes: &[&str]| {
        names
            .iter()
            .any(|n| prefixes.iter().any(|p| n.starts_with(p)))
    };
    let mut missing = Vec::new();
    // An empty repository has no tree to judge.
    if !r["root"].is_null() {
        if !has(&["readme"]) {
            missing.push("README".to_owned());
        }
        if !private && r["licenseInfo"].is_null() && !has(&["license", "licence", "copying"]) {
            missing.push("LICENSE".to_owned());
        }
        let ci = r["workflows"]["entries"].as_array().is_some_and(|e| {
            e.iter().any(|f| {
                f["name"]
                    .as_str()
                    .is_some_and(|n| n.ends_with(".yml") || n.ends_with(".yaml"))
            })
        });
        if !ci {
            missing.push("CI".to_owned());
        }
    }
    let mut out = TidyRepo {
        repo,
        private,
        branches: r["refs"]["totalCount"].as_u64().unwrap_or(0) as u32,
        missing,
        ..Default::default()
    };
    for b in r["refs"]["nodes"].as_array().into_iter().flatten() {
        let name = b["name"].as_str().unwrap_or_default();
        if name.is_empty() || name == default {
            continue;
        }
        let oid = b["target"]["oid"].as_str().unwrap_or_default().to_owned();
        let last = b["target"]["committedDate"].as_str().map(str::to_owned);
        let open = b["open"]["totalCount"].as_u64().unwrap_or(0) > 0;
        let merged_pr = b["merged"]["nodes"][0]
            .as_object()
            .filter(|pr| pr.get("headRefOid").and_then(Value::as_str) == Some(oid.as_str()))
            .and_then(|pr| pr.get("number").and_then(Value::as_u64))
            .map(|n| n as u32);
        let branch = Branch {
            name: name.to_owned(),
            oid,
            last_commit: last.clone(),
            pull_request: merged_pr,
        };
        if open {
            continue;
        }
        if merged_pr.is_some() {
            out.merged.push(branch);
        } else if last
            .as_deref()
            .and_then(|d| days_ago(d, now))
            .is_some_and(|d| d >= STALE_DAYS)
        {
            out.stale.push(branch);
        }
    }
    Some(out)
}

fn old_runs(token: &str, repo: &str, now: i64) -> Result<Option<u32>> {
    let cutoff = now - OLD_RUN_DAYS * 86_400;
    let date = crate::iso_date(cutoff);
    let r = call(
        token,
        "GET",
        &format!("repos/{repo}/actions/runs?created=%3C{date}&per_page=1"),
        None,
    )?;
    Ok(r.ok()
        .then(|| r.body["total_count"].as_u64().unwrap_or(0) as u32))
}

/// Reads GitHub for the owner's repositories (archived ones and forks left out).
pub fn fetch(token: &str, repos: &[RemoteRepo]) -> Result<TidySnapshot> {
    let now = crate::now();
    let names: Vec<String> = repos
        .iter()
        .filter(|r| !r.archived && !r.fork)
        .map(|r| r.full_name.clone())
        .collect();
    let agent = crate::github::agent();
    let mut snap = TidySnapshot {
        checked_at: now,
        ..Default::default()
    };
    for batch in names.chunks(BATCH) {
        let page = crate::github::repos_batch(&agent, token, batch, "Tidy", FRAGMENT)?;
        for (i, full) in batch.iter().enumerate() {
            match parse_repo(&page["data"][format!("r{i}")], now) {
                Some(t) => snap.repos.push(t),
                None => snap
                    .warnings
                    .push(format!("{full}: GitHub did not answer about it")),
            }
        }
    }
    // Old runs only where there are workflows to run.
    let with_ci: Vec<usize> = snap
        .repos
        .iter()
        .enumerate()
        .filter(|(_, r)| !r.missing.iter().any(|m| m == "CI"))
        .map(|(i, _)| i)
        .collect();
    let counts: Vec<(usize, Result<Option<u32>>)> = std::thread::scope(|s| {
        let chunk = with_ci.len().div_ceil(6).max(1);
        let repos = &snap.repos;
        let handles: Vec<_> = with_ci
            .chunks(chunk)
            .map(|part| {
                s.spawn(move || {
                    part.iter()
                        .map(|&i| (i, old_runs(token, &repos[i].repo, now)))
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().unwrap_or_default())
            .collect()
    });
    for (i, n) in counts {
        match n {
            Ok(n) => snap.repos[i].old_runs = n,
            Err(e @ Error::Auth(_)) => return Err(e),
            Err(e) => snap
                .warnings
                .push(format!("{}: old workflow runs: {e}", snap.repos[i].repo)),
        }
    }
    Ok(snap)
}

/// Tracked files at least [`LARGE_FILE`] in a clone's checked-out commit.
pub fn large_files(dir: &Path) -> Vec<LargeFile> {
    let Ok(out) = git(dir, &["ls-tree", "-r", "-l", "HEAD"]) else {
        return Vec::new();
    };
    // "<mode> blob <sha> <size>\t<path>"
    let mut files: Vec<LargeFile> = out
        .lines()
        .filter_map(|l| {
            let (meta, path) = l.split_once('\t')?;
            let size: u64 = meta.split_whitespace().nth(3)?.parse().ok()?;
            (size >= LARGE_FILE).then(|| LargeFile {
                path: path.to_owned(),
                size,
            })
        })
        .collect();
    files.sort_by_key(|f| std::cmp::Reverse(f.size));
    files
}

/// The board: GitHub's last answer with a fresh look at the clones for large files.
pub fn build(snap: Option<&TidySnapshot>, clones: &[LocalClone]) -> TidyBoard {
    let mut repos: Vec<TidyRepo> = snap.map(|s| s.repos.clone()).unwrap_or_default();
    let mut seen = std::collections::HashSet::new();
    let usable: Vec<&LocalClone> = clones
        .iter()
        .filter(|c| c.error.is_none() && !c.worktree)
        .filter(|c| {
            seen.insert(
                c.github
                    .clone()
                    .unwrap_or_else(|| c.path.display().to_string()),
            )
        })
        .collect();
    let found: Vec<(String, Vec<LargeFile>)> = std::thread::scope(|s| {
        let chunk = usable.len().div_ceil(8).max(1);
        let handles: Vec<_> = usable
            .chunks(chunk)
            .map(|part| {
                s.spawn(move || {
                    part.iter()
                        .map(|c| {
                            (
                                c.github
                                    .clone()
                                    .unwrap_or_else(|| c.path.display().to_string()),
                                large_files(&c.path),
                            )
                        })
                        .filter(|(_, f)| !f.is_empty())
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().unwrap_or_default())
            .collect()
    });
    for (repo, files) in found {
        match repos
            .iter_mut()
            .find(|r| r.repo.eq_ignore_ascii_case(&repo))
        {
            Some(r) => r.large_files = files,
            None => repos.push(TidyRepo {
                repo,
                large_files: files,
                ..Default::default()
            }),
        }
    }
    let weight = |r: &TidyRepo| {
        r.merged.len()
            + r.stale.len()
            + r.missing.len()
            + r.large_files.len() * 3
            + usize::from(r.old_runs.unwrap_or(0) > 0)
    };
    repos.retain(|r| weight(r) > 0);
    repos.sort_by(|a, b| {
        weight(b)
            .cmp(&weight(a))
            .then(a.repo.to_lowercase().cmp(&b.repo.to_lowercase()))
    });
    let n = |f: &dyn Fn(&TidyRepo) -> usize| repos.iter().map(f).sum::<usize>() as u32;
    let missing = |what: &str| n(&|r| usize::from(r.missing.iter().any(|m| m == what)));
    TidyBoard {
        counts: TidyCounts {
            merged: n(&|r| r.merged.len()),
            stale: n(&|r| r.stale.len()),
            no_readme: missing("README"),
            no_license: missing("LICENSE"),
            no_ci: missing("CI"),
            old_runs: n(&|r| r.old_runs.unwrap_or(0) as usize),
            large_files: n(&|r| r.large_files.len()),
        },
        repos,
        checked_at: snap.map(|s| s.checked_at),
        warnings: snap.map(|s| s.warnings.clone()).unwrap_or_default(),
    }
}

/// Deletes merged branches on GitHub. Each branch is read again first and left alone if its
/// newest commit is not the one seen (someone pushed to it since). Returns one line per branch.
pub fn delete_merged(token: &str, repo: &str, branches: &[Branch]) -> Result<Vec<String>> {
    if !crate::apps::valid_name(repo) {
        return Err(Error::GitHub(format!("{repo} is not owner/name")));
    }
    let mut out = Vec::new();
    for b in branches {
        let path = format!(
            "repos/{repo}/git/refs/heads/{}",
            b.name
                .split('/')
                .map(|p| p
                    .bytes()
                    .map(|c| match c {
                        b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' =>
                            (c as char).to_string(),
                        _ => format!("%{c:02X}"),
                    })
                    .collect::<String>())
                .collect::<Vec<_>>()
                .join("/")
        );
        let now = call(token, "GET", &path, None)?;
        let oid = now.body["object"]["sha"].as_str().unwrap_or_default();
        if !now.ok() {
            out.push(format!("{}: already gone", b.name));
            continue;
        }
        if oid != b.oid {
            out.push(format!("{}: has new commits since; left alone", b.name));
            continue;
        }
        let r = call(token, "DELETE", &path, None)?;
        out.push(if r.ok() {
            format!("{}: deleted", b.name)
        } else {
            format!("{}: {}", b.name, r.message())
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn branches_and_missing_files() {
        let now = crate::parse_time("2026-11-27T00:00:00Z").unwrap();
        let r = json!({
            "nameWithOwner": "me/app", "isPrivate": false, "isFork": false,
            "licenseInfo": null,
            "defaultBranchRef": { "name": "main" },
            "root": { "entries": [{ "name": "Readme.md" }, { "name": "src" }] },
            "workflows": null,
            "refs": { "totalCount": 5, "nodes": [
                { "name": "main", "target": { "oid": "m", "committedDate": "2026-11-26T00:00:00Z" },
                  "merged": { "nodes": [] }, "open": { "totalCount": 0 } },
                { "name": "feat/done", "target": { "oid": "a", "committedDate": "2026-11-01T00:00:00Z" },
                  "merged": { "nodes": [{ "number": 7, "headRefOid": "a" }] }, "open": { "totalCount": 0 } },
                { "name": "pushed-after-merge", "target": { "oid": "b2", "committedDate": "2026-11-20T00:00:00Z" },
                  "merged": { "nodes": [{ "number": 8, "headRefOid": "b1" }] }, "open": { "totalCount": 0 } },
                { "name": "old-idea", "target": { "oid": "c", "committedDate": "2026-03-01T00:00:00Z" },
                  "merged": { "nodes": [] }, "open": { "totalCount": 0 } },
                { "name": "old-but-open", "target": { "oid": "d", "committedDate": "2026-03-01T00:00:00Z" },
                  "merged": { "nodes": [] }, "open": { "totalCount": 1 } }
            ]}
        });
        let t = parse_repo(&r, now).unwrap();
        assert_eq!(t.missing, vec!["LICENSE", "CI"]);
        assert_eq!(
            t.merged.iter().map(|b| b.name.as_str()).collect::<Vec<_>>(),
            vec!["feat/done"]
        );
        assert_eq!(t.merged[0].pull_request, Some(7));
        assert_eq!(
            t.stale.iter().map(|b| b.name.as_str()).collect::<Vec<_>>(),
            vec!["old-idea"]
        );
        assert_eq!(t.branches, 5);

        // A private repository needs no licence; an empty one is not judged.
        let mut p = r.clone();
        p["isPrivate"] = json!(true);
        p["workflows"] = json!({ "entries": [{ "name": "ci.yml" }] });
        assert!(parse_repo(&p, now).unwrap().missing.is_empty());
        p["root"] = Value::Null;
        assert!(parse_repo(&p, now).unwrap().missing.is_empty());
    }

    #[test]
    fn large_files_in_a_clone() {
        let t = tempfile::tempdir().unwrap();
        let d = t.path();
        let git = |args: &[&str]| {
            assert!(
                std::process::Command::new("git")
                    .arg("-C")
                    .arg(d)
                    .args(["-c", "user.name=t", "-c", "user.email=t@example.com"])
                    .args(args)
                    .status()
                    .unwrap()
                    .success()
            )
        };
        git(&["init", "-q"]);
        std::fs::write(d.join("small.txt"), "x").unwrap();
        let f = std::fs::File::create(d.join("big.bin")).unwrap();
        f.set_len(LARGE_FILE + 1).unwrap();
        git(&["add", "."]);
        git(&["commit", "-qm", "files"]);
        let found = large_files(d);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].path, "big.bin");
    }
}
