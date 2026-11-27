//! What GitHub says about the owner's repositories, read with the owner's own `gh` login.
//!
//! One GraphQL query, 25 repositories a page, costs 1 point of the 5,000-an-hour budget, so a
//! full refresh of 50 repositories costs 2. The remaining budget is kept and shown, so a limit
//! never stops anything silently.

use std::process::Command;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{Error, Result};

pub(crate) const API: &str = "https://api.github.com";
const PAGE: u32 = 25;

/// The token from `GH_TOKEN`, `GITHUB_TOKEN`, or `gh auth token`. Never written anywhere.
pub fn token() -> Result<String> {
    for var in ["GH_TOKEN", "GITHUB_TOKEN"] {
        if let Ok(t) = std::env::var(var)
            && !t.trim().is_empty()
        {
            return Ok(t.trim().to_owned());
        }
    }
    let out = Command::new("gh")
        .args(["auth", "token"])
        .output()
        .map_err(|_| {
            Error::Auth(
                "GitHub CLI (gh) is not installed. Install it, then run: gh auth login".into(),
            )
        })?;
    let t = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    if !out.status.success() || t.is_empty() {
        return Err(Error::Auth(
            "Not signed in to GitHub. Run: gh auth login".into(),
        ));
    }
    Ok(t)
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Rate {
    pub limit: u32,
    pub remaining: u32,
    /// When the budget refills, ISO 8601.
    pub reset_at: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Release {
    pub tag: String,
    pub prerelease: bool,
    pub draft: bool,
    pub published_at: Option<String>,
    pub url: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Alerts {
    pub total: u32,
    pub critical: u32,
    pub high: u32,
    pub moderate: u32,
    pub low: u32,
}

/// The state of the newest commit on the default branch, from its checks and statuses.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Ci {
    Passing,
    Failing,
    Running,
    #[default]
    None,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RemoteRepo {
    pub name: String,
    pub full_name: String,
    pub url: String,
    pub description: Option<String>,
    pub private: bool,
    pub archived: bool,
    pub fork: bool,
    pub language: Option<String>,
    pub pushed_at: Option<String>,
    pub default_branch: Option<String>,
    pub ci: Ci,
    /// The newest release, pre-releases included (most of the owner's apps only have those).
    pub release: Option<Release>,
    /// `None` when GitHub would not say (alerts turned off, or no permission).
    pub alerts: Option<Alerts>,
    pub open_issues: u32,
    pub open_prs: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub login: String,
    pub repos: Vec<RemoteRepo>,
    pub rate: Option<Rate>,
    /// Unix seconds.
    pub fetched_at: i64,
    /// Parts GitHub refused, e.g. alerts on one repository. The rest is still good.
    pub warnings: Vec<String>,
}

const QUERY: &str = r#"
query($cursor: String, $page: Int!) {
  rateLimit { limit remaining resetAt }
  viewer {
    login
    repositories(first: $page, after: $cursor, ownerAffiliations: OWNER,
                 orderBy: {field: PUSHED_AT, direction: DESC}) {
      pageInfo { hasNextPage endCursor }
      nodes {
        name nameWithOwner url description visibility isArchived isFork pushedAt
        primaryLanguage { name }
        defaultBranchRef { name target { ... on Commit { statusCheckRollup { state } } } }
        releases(first: 1, orderBy: {field: CREATED_AT, direction: DESC}) {
          nodes { tagName isPrerelease isDraft publishedAt url }
        }
        vulnerabilityAlerts(states: OPEN, first: 20) {
          totalCount nodes { securityVulnerability { severity } }
        }
        issues(states: OPEN) { totalCount }
        pullRequests(states: OPEN) { totalCount }
      }
    }
  }
}"#;

pub(crate) fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(60)))
        .user_agent("sangraha")
        .build()
        .into()
}

/// Every repository the signed-in account owns.
pub fn fetch(token: &str) -> Result<Snapshot> {
    let agent = agent();
    let mut snap = Snapshot {
        fetched_at: crate::now(),
        ..Default::default()
    };
    let mut cursor: Option<String> = None;
    loop {
        let body = json!({ "query": QUERY, "variables": { "cursor": cursor, "page": PAGE } });
        let page = post_graphql(&agent, token, &body)?;
        let next = parse_page(&page, &mut snap)?;
        match next {
            Some(c) => cursor = Some(c),
            None => break,
        }
    }
    Ok(snap)
}

pub(crate) fn post_graphql(agent: &ureq::Agent, token: &str, body: &Value) -> Result<Value> {
    let mut last = String::new();
    // GitHub answers a busy GraphQL request with 502; one more try usually goes through.
    for _ in 0..2 {
        let res = agent
            .post(format!("{API}/graphql"))
            .header("Authorization", &format!("bearer {token}"))
            .send_json(body);
        match res {
            Ok(mut r) => {
                return r
                    .body_mut()
                    .read_json::<Value>()
                    .map_err(|e| Error::GitHub(e.to_string()));
            }
            Err(ureq::Error::StatusCode(401)) => {
                return Err(Error::Auth(
                    "GitHub refused the login. Run: gh auth login".into(),
                ));
            }
            Err(ureq::Error::StatusCode(code @ 502..=504)) => last = format!("HTTP {code}"),
            Err(e) => return Err(Error::GitHub(e.to_string())),
        }
    }
    Err(Error::GitHub(format!(
        "GitHub is busy ({last}). Try again in a minute."
    )))
}

/// One GraphQL request asking about each repository in `batch` (at most 25) through
/// `fragment`, which must define `fragment <fragment_name> on Repository`. Answers come back as
/// `data.r0`, `data.r1` … in the order of `batch`, with `data.rateLimit`. Names travel as
/// variables, never inside the query text.
pub(crate) fn repos_batch(
    agent: &ureq::Agent,
    token: &str,
    batch: &[String],
    fragment_name: &str,
    fragment: &str,
) -> Result<Value> {
    let mut vars = serde_json::Map::new();
    let mut params = Vec::new();
    let mut fields = Vec::new();
    for (i, full) in batch.iter().enumerate() {
        let (o, n) = full.split_once('/').unwrap_or((full, ""));
        vars.insert(format!("o{i}"), json!(o));
        vars.insert(format!("n{i}"), json!(n));
        params.push(format!("$o{i}: String!, $n{i}: String!"));
        fields.push(format!(
            "r{i}: repository(owner: $o{i}, name: $n{i}) {{ ...{fragment_name} }}"
        ));
    }
    let query = format!(
        "query({}) {{ {} rateLimit {{ limit remaining resetAt }} }}{fragment}",
        params.join(", "),
        fields.join(" ")
    );
    let page = post_graphql(agent, token, &json!({ "query": query, "variables": vars }))?;
    if page["data"].is_null() {
        let msg = page["errors"][0]["message"]
            .as_str()
            .unwrap_or("no data in the answer");
        return Err(Error::GitHub(msg.to_owned()));
    }
    Ok(page)
}

/// Adds one page of results to `snap`; returns the cursor of the next page, if any.
pub fn parse_page(page: &Value, snap: &mut Snapshot) -> Result<Option<String>> {
    let data = &page["data"];
    if data.is_null() {
        let msg = page["errors"][0]["message"]
            .as_str()
            .unwrap_or("no data in the answer");
        return Err(Error::GitHub(msg.to_owned()));
    }
    // Partial errors (one repository's alerts, say) leave the rest of the page usable.
    if let Some(errs) = page["errors"].as_array() {
        for e in errs {
            let path = e["path"].as_array().map(|p| {
                p.iter()
                    .map(|x| x.to_string().trim_matches('"').to_owned())
                    .collect::<Vec<_>>()
                    .join(".")
            });
            snap.warnings.push(format!(
                "{}{}",
                e["message"].as_str().unwrap_or("error"),
                path.map(|p| format!(" ({p})")).unwrap_or_default()
            ));
        }
    }
    if let Some(r) = data["rateLimit"].as_object() {
        snap.rate = Some(Rate {
            limit: r["limit"].as_u64().unwrap_or(0) as u32,
            remaining: r["remaining"].as_u64().unwrap_or(0) as u32,
            reset_at: r["resetAt"].as_str().unwrap_or_default().to_owned(),
        });
    }
    let viewer = &data["viewer"];
    snap.login = viewer["login"].as_str().unwrap_or_default().to_owned();
    let repos = &viewer["repositories"];
    for n in repos["nodes"].as_array().into_iter().flatten() {
        if n.is_null() {
            continue;
        }
        snap.repos.push(repo_from(n));
    }
    let info = &repos["pageInfo"];
    Ok(if info["hasNextPage"].as_bool() == Some(true) {
        info["endCursor"].as_str().map(str::to_owned)
    } else {
        None
    })
}

fn s(v: &Value) -> Option<String> {
    v.as_str().filter(|s| !s.is_empty()).map(str::to_owned)
}

fn count(v: &Value) -> u32 {
    v["totalCount"].as_u64().unwrap_or(0) as u32
}

fn repo_from(n: &Value) -> RemoteRepo {
    let ci = match n["defaultBranchRef"]["target"]["statusCheckRollup"]["state"].as_str() {
        Some("SUCCESS") => Ci::Passing,
        Some("FAILURE" | "ERROR") => Ci::Failing,
        Some("PENDING" | "EXPECTED") => Ci::Running,
        _ => Ci::None,
    };
    let release = n["releases"]["nodes"]
        .get(0)
        .filter(|r| !r.is_null())
        .map(|r| Release {
            tag: s(&r["tagName"]).unwrap_or_default(),
            prerelease: r["isPrerelease"].as_bool().unwrap_or(false),
            draft: r["isDraft"].as_bool().unwrap_or(false),
            published_at: s(&r["publishedAt"]),
            url: s(&r["url"]).unwrap_or_default(),
        });
    let va = &n["vulnerabilityAlerts"];
    let alerts = (!va.is_null()).then(|| {
        let mut a = Alerts {
            total: count(va),
            ..Default::default()
        };
        for node in va["nodes"].as_array().into_iter().flatten() {
            match node["securityVulnerability"]["severity"].as_str() {
                Some("CRITICAL") => a.critical += 1,
                Some("HIGH") => a.high += 1,
                Some("MODERATE") => a.moderate += 1,
                Some("LOW") => a.low += 1,
                _ => {}
            }
        }
        a
    });
    RemoteRepo {
        name: s(&n["name"]).unwrap_or_default(),
        full_name: s(&n["nameWithOwner"]).unwrap_or_default(),
        url: s(&n["url"]).unwrap_or_default(),
        description: s(&n["description"]),
        private: n["visibility"].as_str() != Some("PUBLIC"),
        archived: n["isArchived"].as_bool().unwrap_or(false),
        fork: n["isFork"].as_bool().unwrap_or(false),
        language: s(&n["primaryLanguage"]["name"]),
        pushed_at: s(&n["pushedAt"]),
        default_branch: s(&n["defaultBranchRef"]["name"]),
        ci,
        release,
        alerts,
        open_issues: count(&n["issues"]),
        open_prs: count(&n["pullRequests"]),
    }
}

/// The current `owner/name` of a repository that was renamed or moved: GitHub still answers
/// the old name and points to the new one. `None` if it does not exist (or is not visible).
pub fn current_name(token: &str, full_name: &str) -> Result<Option<String>> {
    Ok(get_json(&agent(), token, &format!("repos/{full_name}"))?.and_then(|v| s(&v["full_name"])))
}

/// A GET on the REST API. `None` when GitHub says the thing is missing, turned off or not
/// allowed (404, 403, 410, 422, 451), which for alert lists means "not available here".
pub(crate) fn get_json(agent: &ureq::Agent, token: &str, path: &str) -> Result<Option<Value>> {
    let res = agent
        .get(format!("{API}/{path}"))
        .header("Authorization", &format!("bearer {token}"))
        .header("Accept", "application/vnd.github+json")
        .call();
    match res {
        Ok(mut r) => r
            .body_mut()
            .read_json()
            .map(Some)
            .map_err(|e| Error::GitHub(e.to_string())),
        Err(ureq::Error::StatusCode(404 | 403 | 410 | 422 | 451)) => Ok(None),
        Err(ureq::Error::StatusCode(401)) => Err(Error::Auth(
            "GitHub refused the login. Run: gh auth login".into(),
        )),
        Err(e) => Err(Error::GitHub(e.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(next: bool) -> Value {
        json!({
          "data": {
            "rateLimit": { "limit": 5000, "remaining": 4990, "resetAt": "2026-11-27T14:46:18Z" },
            "viewer": { "login": "owner", "repositories": {
              "pageInfo": { "hasNextPage": next, "endCursor": "abc" },
              "nodes": [
                { "name": "setu", "nameWithOwner": "owner/setu", "url": "https://github.com/owner/setu",
                  "description": "Sync", "visibility": "PUBLIC", "isArchived": false, "isFork": false,
                  "pushedAt": "2026-11-20T00:00:00Z", "primaryLanguage": { "name": "Rust" },
                  "defaultBranchRef": { "name": "main", "target": { "statusCheckRollup": { "state": "FAILURE" } } },
                  "releases": { "nodes": [ { "tagName": "v0.1.0-alpha.8", "isPrerelease": true, "isDraft": false,
                                             "publishedAt": "2026-11-21T00:00:00Z", "url": "https://github.com/owner/setu/releases/tag/v0.1.0-alpha.8" } ] },
                  "vulnerabilityAlerts": { "totalCount": 3, "nodes": [
                      { "securityVulnerability": { "severity": "HIGH" } },
                      { "securityVulnerability": { "severity": "CRITICAL" } },
                      { "securityVulnerability": { "severity": "LOW" } } ] },
                  "issues": { "totalCount": 2 }, "pullRequests": { "totalCount": 1 } },
                { "name": "notes", "nameWithOwner": "owner/notes", "url": "https://github.com/owner/notes",
                  "description": "", "visibility": "PRIVATE", "isArchived": true, "isFork": false,
                  "pushedAt": null, "primaryLanguage": null, "defaultBranchRef": null,
                  "releases": { "nodes": [] }, "vulnerabilityAlerts": null,
                  "issues": { "totalCount": 0 }, "pullRequests": { "totalCount": 0 } }
              ] } }
          },
          "errors": [ { "message": "Resource not accessible", "path": ["viewer", "repositories", "nodes", 1, "vulnerabilityAlerts"] } ]
        })
    }

    #[test]
    fn parses_a_page() {
        let mut snap = Snapshot::default();
        let next = parse_page(&page(true), &mut snap).unwrap();
        assert_eq!(next.as_deref(), Some("abc"));
        assert_eq!(snap.login, "owner");
        assert_eq!(snap.rate.as_ref().unwrap().remaining, 4990);
        let setu = &snap.repos[0];
        assert_eq!(setu.ci, Ci::Failing);
        assert!(setu.release.as_ref().unwrap().prerelease);
        let a = setu.alerts.as_ref().unwrap();
        assert_eq!((a.total, a.critical, a.high, a.low), (3, 1, 1, 1));
        assert_eq!((setu.open_issues, setu.open_prs), (2, 1));
        let notes = &snap.repos[1];
        assert!(notes.private && notes.archived);
        assert_eq!(notes.description, None);
        assert_eq!(notes.alerts, None);
        assert_eq!(notes.ci, Ci::None);
        assert_eq!(snap.warnings.len(), 1);
        assert!(snap.warnings[0].contains("vulnerabilityAlerts"));
    }

    #[test]
    fn last_page_has_no_cursor() {
        let mut snap = Snapshot::default();
        assert_eq!(parse_page(&page(false), &mut snap).unwrap(), None);
    }

    #[test]
    fn whole_failure_is_an_error() {
        let mut snap = Snapshot::default();
        let v = json!({ "data": null, "errors": [ { "message": "Bad credentials" } ] });
        let e = parse_page(&v, &mut snap).unwrap_err();
        assert!(e.to_string().contains("Bad credentials"));
    }
}
