//! The Security tab: open security alerts across the owner's repositories, grouped by package
//! so one upgrade can close many; secret and code scanning alerts per repository; and the
//! release gate, which stops a tag while shipped dependencies have high or critical alerts.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::Result;
use crate::github::{Rate, RemoteRepo};
use crate::hooks::HookState;

const BATCH: usize = 25;

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "camelCase")]
pub enum Severity {
    #[default]
    Low,
    Moderate,
    High,
    Critical,
}

impl Severity {
    fn from_github(s: &str) -> Severity {
        match s {
            "CRITICAL" => Severity::Critical,
            "HIGH" => Severity::High,
            "MODERATE" | "MEDIUM" => Severity::Moderate,
            _ => Severity::Low,
        }
    }
}

/// Where a dependency ends up. GitHub knows this for some ecosystems only.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Scope {
    /// Shipped with the app.
    Runtime,
    /// Only used to build or test.
    Development,
    /// GitHub does not say; treated as shipped by the release gate.
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Alert {
    pub repo: String,
    pub number: u32,
    pub package: String,
    pub ecosystem: String,
    pub severity: Severity,
    pub scope: Scope,
    pub manifest: Option<String>,
    /// The version range in use, e.g. `< 1.2.3` or `= 0.4.1`.
    pub requirement: Option<String>,
    pub patched: Option<String>,
    pub advisory: String,
    pub summary: String,
    pub url: String,
}

/// Secret and code scanning alerts in one repository. `None`: not turned on, or not available
/// for this repository.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RepoScan {
    pub repo: String,
    pub secrets: Option<u32>,
    pub code: Option<u32>,
    /// Code scanning alerts rated high or critical.
    pub code_serious: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SecuritySnapshot {
    pub fetched_at: i64,
    pub alerts: Vec<Alert>,
    pub scans: Vec<RepoScan>,
    pub rate: Option<Rate>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PackageGroup {
    pub ecosystem: String,
    pub package: String,
    pub severity: Severity,
    /// Shipped in at least one repository (or GitHub does not say).
    pub shipped: bool,
    pub repos: Vec<String>,
    /// The lowest version that fixes every alert that has a fix.
    pub fix: Option<String>,
    /// Alerts with no fixed version yet.
    pub unfixed: u32,
    pub alerts: Vec<Alert>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SecurityCounts {
    pub critical: u32,
    pub high: u32,
    pub moderate: u32,
    pub low: u32,
    pub shipped: u32,
    pub development: u32,
    pub secrets: u32,
    pub code: u32,
    /// Repositories where secret scanning is off or not available.
    pub secrets_off: u32,
    pub code_off: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HookRow {
    pub path: PathBuf,
    pub github: Option<String>,
    pub state: HookState,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SecurityBoard {
    pub checked_at: Option<i64>,
    pub rate: Option<Rate>,
    pub groups: Vec<PackageGroup>,
    /// Repositories with open secret or code scanning alerts.
    pub scans: Vec<RepoScan>,
    pub counts: SecurityCounts,
    pub hooks: Vec<HookRow>,
    /// The command line program the hook calls; `None` when it cannot be found.
    pub cli: Option<PathBuf>,
    pub warnings: Vec<String>,
}

const FRAGMENT: &str = r#"
fragment Sec on Repository {
  nameWithOwner
  vulnerabilityAlerts(states: OPEN, first: 100) {
    nodes { number dependencyScope vulnerableManifestPath vulnerableRequirements
      securityVulnerability { severity package { name ecosystem }
        firstPatchedVersion { identifier } advisory { ghsaId summary permalink } } }
  }
}"#;

/// Open Dependabot alerts in `repos`, 25 repositories to a query.
pub fn fetch_alerts(
    agent: &ureq::Agent,
    token: &str,
    repos: &[String],
    snap: &mut SecuritySnapshot,
) -> Result<()> {
    for batch in repos.chunks(BATCH) {
        let page = crate::github::repos_batch(agent, token, batch, "Sec", FRAGMENT)?;
        parse_alerts(&page, batch, snap);
    }
    Ok(())
}

pub fn parse_alerts(page: &Value, batch: &[String], snap: &mut SecuritySnapshot) {
    let data = &page["data"];
    for (i, full) in batch.iter().enumerate() {
        let r = &data[format!("r{i}")];
        let va = &r["vulnerabilityAlerts"];
        if va.is_null() {
            snap.warnings
                .push(format!("{full}: GitHub would not list its security alerts"));
            continue;
        }
        let repo = r["nameWithOwner"].as_str().unwrap_or(full);
        for n in va["nodes"].as_array().into_iter().flatten() {
            let v = &n["securityVulnerability"];
            let text = |x: &Value| x.as_str().map(str::to_owned);
            let number = n["number"].as_u64().unwrap_or(0) as u32;
            snap.alerts.push(Alert {
                repo: repo.to_owned(),
                number,
                package: text(&v["package"]["name"]).unwrap_or_default(),
                ecosystem: text(&v["package"]["ecosystem"])
                    .unwrap_or_default()
                    .to_lowercase(),
                severity: Severity::from_github(v["severity"].as_str().unwrap_or("")),
                scope: match n["dependencyScope"].as_str() {
                    Some("RUNTIME") => Scope::Runtime,
                    Some("DEVELOPMENT") => Scope::Development,
                    _ => Scope::Unknown,
                },
                manifest: text(&n["vulnerableManifestPath"]),
                requirement: text(&n["vulnerableRequirements"]),
                patched: text(&v["firstPatchedVersion"]["identifier"]),
                advisory: text(&v["advisory"]["ghsaId"]).unwrap_or_default(),
                summary: text(&v["advisory"]["summary"]).unwrap_or_default(),
                url: format!("https://github.com/{repo}/security/dependabot/{number}"),
            });
        }
    }
    if let Some(rl) = data.get("rateLimit").filter(|v| !v.is_null()) {
        snap.rate = serde_json::from_value(rl.clone()).ok();
    }
}

/// How many alerts a REST list holds (`None` when the feature is off), and how many of them
/// are high or critical.
fn count_list(v: Option<Value>, serious: impl Fn(&Value) -> bool) -> (Option<u32>, u32) {
    match v.as_ref().and_then(Value::as_array) {
        Some(list) => (
            Some(list.len() as u32),
            list.iter().filter(|a| serious(a)).count() as u32,
        ),
        None => (None, 0),
    }
}

/// Secret and code scanning alerts per repository, a few repositories at a time. Only counts
/// are kept: a secret scanning alert carries the secret itself, which Sangraha never stores.
pub fn fetch_scans(agent: &ureq::Agent, token: &str, repos: &[String]) -> Result<Vec<RepoScan>> {
    let chunk = repos.len().div_ceil(6).max(1);
    let results: Vec<Result<RepoScan>> = std::thread::scope(|s| {
        let handles: Vec<_> = repos
            .chunks(chunk)
            .map(|part| {
                s.spawn(move || {
                    part.iter()
                        .map(|full| -> Result<RepoScan> {
                            let get = |what: &str| {
                                crate::github::get_json(
                                    agent,
                                    token,
                                    &format!("repos/{full}/{what}/alerts?state=open&per_page=100"),
                                )
                            };
                            let (secrets, _) = count_list(get("secret-scanning")?, |_| true);
                            let (code, code_serious) = count_list(get("code-scanning")?, |a| {
                                matches!(
                                    a["rule"]["security_severity_level"].as_str(),
                                    Some("high" | "critical")
                                )
                            });
                            Ok(RepoScan {
                                repo: full.clone(),
                                secrets,
                                code,
                                code_serious,
                            })
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().unwrap_or_default())
            .collect()
    });
    results.into_iter().collect()
}

/// Reads alerts for the repositories that have any, and scanning alerts for every repository
/// that is not archived.
pub fn fetch(token: &str, repos: &[RemoteRepo]) -> Result<SecuritySnapshot> {
    let agent = crate::github::agent();
    let mut snap = SecuritySnapshot {
        fetched_at: crate::now(),
        ..Default::default()
    };
    let alerted: Vec<String> = repos
        .iter()
        .filter(|r| r.alerts.as_ref().is_some_and(|a| a.total > 0))
        .map(|r| r.full_name.clone())
        .collect();
    fetch_alerts(&agent, token, &alerted, &mut snap)?;
    let live: Vec<String> = repos
        .iter()
        .filter(|r| !r.archived)
        .map(|r| r.full_name.clone())
        .collect();
    snap.scans = fetch_scans(&agent, token, &live)?;
    Ok(snap)
}

/// One spelling per package: names are case-blind, and on PyPI `-`, `_` and `.` are the same.
fn package_key(ecosystem: &str, name: &str) -> String {
    let lower = name.to_lowercase();
    if ecosystem == "pip" {
        lower.replace(['_', '.'], "-")
    } else {
        lower
    }
}

/// Alerts grouped by package: worst first, shipped before development-only, then by how many
/// repositories one upgrade would fix.
pub fn group(alerts: &[Alert]) -> Vec<PackageGroup> {
    let mut map: BTreeMap<(String, String), PackageGroup> = BTreeMap::new();
    for a in alerts {
        let g = map
            .entry((a.ecosystem.clone(), package_key(&a.ecosystem, &a.package)))
            .or_insert_with(|| PackageGroup {
                ecosystem: a.ecosystem.clone(),
                package: a.package.clone(),
                ..Default::default()
            });
        g.severity = g.severity.max(a.severity);
        g.shipped |= a.scope != Scope::Development;
        if !g.repos.contains(&a.repo) {
            g.repos.push(a.repo.clone());
        }
        match &a.patched {
            Some(p) => {
                let newer = g.fix.as_deref().is_none_or(|f| {
                    crate::apps::compare(p, f) == Some(std::cmp::Ordering::Greater)
                });
                if newer {
                    g.fix = Some(p.clone());
                }
            }
            None => g.unfixed += 1,
        }
        g.alerts.push(a.clone());
    }
    let mut groups: Vec<PackageGroup> = map.into_values().collect();
    for g in &mut groups {
        g.repos.sort();
        g.alerts
            .sort_by(|a, b| b.severity.cmp(&a.severity).then(a.repo.cmp(&b.repo)));
    }
    groups.sort_by(|a, b| {
        b.severity
            .cmp(&a.severity)
            .then(b.shipped.cmp(&a.shipped))
            .then(b.repos.len().cmp(&a.repos.len()))
            .then(a.package.cmp(&b.package))
    });
    groups
}

pub fn build(snap: Option<&SecuritySnapshot>, hooks: Vec<HookRow>) -> SecurityBoard {
    let mut b = SecurityBoard {
        hooks,
        cli: crate::hooks::cli_path(),
        ..Default::default()
    };
    let Some(snap) = snap else {
        return b;
    };
    b.checked_at = Some(snap.fetched_at);
    b.rate = snap.rate.clone();
    b.warnings = snap.warnings.clone();
    b.groups = group(&snap.alerts);
    let c = &mut b.counts;
    for a in &snap.alerts {
        match a.severity {
            Severity::Critical => c.critical += 1,
            Severity::High => c.high += 1,
            Severity::Moderate => c.moderate += 1,
            Severity::Low => c.low += 1,
        }
        if a.scope == Scope::Development {
            c.development += 1;
        } else {
            c.shipped += 1;
        }
    }
    for s in &snap.scans {
        c.secrets += s.secrets.unwrap_or(0);
        c.code += s.code.unwrap_or(0);
        c.secrets_off += s.secrets.is_none() as u32;
        c.code_off += s.code.is_none() as u32;
    }
    b.scans = snap
        .scans
        .iter()
        .filter(|s| s.secrets.unwrap_or(0) + s.code.unwrap_or(0) > 0)
        .cloned()
        .collect();
    b.scans.sort_by(|a, b| {
        (b.secrets.unwrap_or(0), b.code_serious, b.code.unwrap_or(0)).cmp(&(
            a.secrets.unwrap_or(0),
            a.code_serious,
            a.code.unwrap_or(0),
        ))
    });
    b
}

/// The clones under the roots and whether each has the leak check.
pub fn hook_rows(roots: &[PathBuf]) -> Vec<HookRow> {
    crate::local::find(roots)
        .into_iter()
        .map(|path| {
            let github = crate::local::git(&path, &["config", "--get", "remote.origin.url"])
                .ok()
                .and_then(|u| crate::local::github_name(&u));
            let state = crate::hooks::state(&path);
            HookRow {
                path,
                github,
                state,
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------------------------
// The release gate

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Gate {
    pub blocked: bool,
    pub reasons: Vec<String>,
    pub warnings: Vec<String>,
}

/// The reasons GitHub's alerts give to hold a release: high or critical alerts in dependencies
/// that ship (or that GitHub cannot place).
pub fn gate_reasons(full: &str, alerts: &[Alert]) -> Vec<String> {
    let serious: Vec<&Alert> = alerts
        .iter()
        .filter(|a| a.severity >= Severity::High && a.scope != Scope::Development)
        .collect();
    if serious.is_empty() {
        return Vec::new();
    }
    let mut names: Vec<String> = serious
        .iter()
        .map(|a| format!("{} ({:?})", a.package, a.severity).to_lowercase())
        .collect();
    names.sort();
    names.dedup();
    vec![format!(
        "{full} has {} open high or critical security alert{} in shipped dependencies: {}. See https://github.com/{full}/security/dependabot",
        serious.len(),
        if serious.len() == 1 { "" } else { "s" },
        names.join(", ")
    )]
}

/// `package-lock.json` files in the clone, two folders deep, outside `node_modules`.
fn lock_files(clone: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut dirs = vec![(clone.to_path_buf(), 0)];
    while let Some((d, depth)) = dirs.pop() {
        if d.join("package-lock.json").is_file() {
            out.push(d.clone());
        }
        if depth >= 2 {
            continue;
        }
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let name = e.file_name();
            let name = name.to_string_lossy();
            if e.file_type().is_ok_and(|t| t.is_dir())
                && !name.starts_with('.')
                && name != "node_modules"
                && name != "target"
            {
                dirs.push((e.path(), depth + 1));
            }
        }
    }
    out.sort();
    out
}

/// Asks npm about the shipped packages in each `package-lock.json`.
fn npm_audit(clone: &Path, gate: &mut Gate) {
    for dir in lock_files(clone) {
        let out = std::process::Command::new(if cfg!(windows) { "npm.cmd" } else { "npm" })
            .args(["audit", "--omit=dev", "--json"])
            .current_dir(&dir)
            .output();
        let shown = dir
            .strip_prefix(clone)
            .unwrap_or(&dir)
            .display()
            .to_string();
        let shown = if shown.is_empty() { ".".into() } else { shown };
        let out = match out {
            Ok(o) => o,
            Err(_) => {
                gate.warnings
                    .push("npm is not installed, so package-lock.json was not audited".into());
                return;
            }
        };
        let v: Value = serde_json::from_slice(&out.stdout).unwrap_or(Value::Null);
        let vulns = &v["metadata"]["vulnerabilities"];
        if vulns.is_null() {
            gate.warnings.push(format!(
                "npm audit gave no answer in {shown}: {}",
                String::from_utf8_lossy(&out.stderr)
                    .trim()
                    .lines()
                    .last()
                    .unwrap_or("")
            ));
            continue;
        }
        let n = |k: &str| vulns[k].as_u64().unwrap_or(0);
        let (high, critical) = (n("high"), n("critical"));
        if high + critical > 0 {
            gate.reasons.push(format!(
                "npm audit in {shown}: {critical} critical and {high} high in shipped packages. Run npm audit there for details"
            ));
        }
    }
}

/// Should a release (a tag) of `full` go out? Reads its open alerts from GitHub, and runs
/// `npm audit` over the clone's lock files when there is a clone. Fails closed: if GitHub
/// cannot be asked, the release waits.
pub fn gate(full: Option<&str>, clone: Option<&Path>) -> Gate {
    let mut g = Gate::default();
    match full {
        Some(full) => {
            let mut snap = SecuritySnapshot::default();
            let asked = crate::github::token().and_then(|t| {
                fetch_alerts(&crate::github::agent(), &t, &[full.to_owned()], &mut snap)
            });
            match asked {
                Ok(()) if snap.warnings.is_empty() => g.reasons.extend(gate_reasons(full, &snap.alerts)),
                Ok(()) => g.reasons.push(format!(
                    "Could not read the security alerts of {full} ({}). Turn on Dependabot alerts, or push with --no-verify to skip the check",
                    snap.warnings.join("; ")
                )),
                Err(e) => g.reasons.push(format!(
                    "Could not read the security alerts of {full} ({e}). Push with --no-verify to skip the check"
                )),
            }
        }
        None => g
            .warnings
            .push("Not a GitHub repository, so GitHub's security alerts were not checked".into()),
    }
    if let Some(c) = clone {
        npm_audit(c, &mut g);
    }
    g.blocked = !g.reasons.is_empty();
    g
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn page() -> Value {
        let alert = |n: u32, pkg: &str, eco: &str, sev: &str, scope: Value, fix: Value| {
            json!({ "number": n, "dependencyScope": scope, "vulnerableManifestPath": "package-lock.json",
                    "vulnerableRequirements": "< 9", "securityVulnerability": {
                      "severity": sev, "package": { "name": pkg, "ecosystem": eco },
                      "firstPatchedVersion": fix.as_str().map(|f| json!({ "identifier": f })).unwrap_or(Value::Null),
                      "advisory": { "ghsaId": format!("GHSA-{n}"), "summary": "bad", "permalink": "" } } })
        };
        json!({ "data": {
          "rateLimit": { "limit": 5000, "remaining": 4999, "resetAt": "2026-11-27T15:00:00Z" },
          "r0": { "nameWithOwner": "me/one", "vulnerabilityAlerts": { "nodes": [
            alert(1, "vite", "NPM", "MODERATE", json!("DEVELOPMENT"), json!("6.1.2")),
            alert(2, "tar", "NPM", "HIGH", json!("RUNTIME"), json!("7.4.1")),
          ]}},
          "r1": { "nameWithOwner": "me/two", "vulnerabilityAlerts": { "nodes": [
            alert(5, "vite", "NPM", "HIGH", json!("DEVELOPMENT"), json!("6.1.10")),
            alert(6, "vite", "NPM", "LOW", json!("DEVELOPMENT"), Value::Null),
            alert(7, "openssl", "RUST", "CRITICAL", Value::Null, json!("0.10.70")),
            alert(8, "Vite", "NPM", "LOW", json!("DEVELOPMENT"), json!("6.0.0")),
          ]}},
          "r2": { "nameWithOwner": "me/off", "vulnerabilityAlerts": null }
        }})
    }

    #[test]
    fn groups_alerts_by_package() {
        let mut snap = SecuritySnapshot::default();
        parse_alerts(
            &page(),
            &["me/one".into(), "me/two".into(), "me/off".into()],
            &mut snap,
        );
        assert_eq!(snap.alerts.len(), 6);
        assert_eq!(snap.warnings.len(), 1);
        assert_eq!(
            snap.alerts[0].url,
            "https://github.com/me/one/security/dependabot/1"
        );
        let groups = group(&snap.alerts);
        let names: Vec<_> = groups.iter().map(|g| g.package.as_str()).collect();
        // Worst first; at equal severity, shipped before development-only.
        assert_eq!(names, vec!["openssl", "tar", "vite"]);
        let vite = &groups[2];
        assert_eq!(vite.repos, vec!["me/one", "me/two"]);
        assert_eq!(vite.alerts.len(), 4, "Vite and vite are one package");
        assert_eq!(vite.severity, Severity::High);
        assert!(!vite.shipped);
        // 6.1.10 is newer than 6.1.2 (by number, not by text).
        assert_eq!(vite.fix.as_deref(), Some("6.1.10"));
        assert_eq!(vite.unfixed, 1);
        assert!(groups[0].shipped, "unknown scope counts as shipped");

        let board = build(Some(&snap), Vec::new());
        let c = &board.counts;
        assert_eq!((c.critical, c.high, c.moderate, c.low), (1, 2, 1, 2));
        assert_eq!((c.shipped, c.development), (2, 4));
    }

    #[test]
    fn gate_blocks_only_shipped_high_alerts() {
        let mut snap = SecuritySnapshot::default();
        parse_alerts(
            &page(),
            &["me/one".into(), "me/two".into(), "me/off".into()],
            &mut snap,
        );
        let of = |repo: &str| -> Vec<Alert> {
            snap.alerts
                .iter()
                .filter(|a| a.repo == repo)
                .cloned()
                .collect()
        };
        let one = gate_reasons("me/one", &of("me/one"));
        assert_eq!(one.len(), 1);
        assert!(
            one[0].contains("1 open high or critical") && one[0].contains("tar (high)"),
            "{}",
            one[0]
        );
        // me/two: the high vite alert is development-only; openssl's scope is unknown, so it counts.
        let two = gate_reasons("me/two", &of("me/two"));
        assert!(
            two[0].contains("openssl (critical)") && !two[0].contains("vite"),
            "{}",
            two[0]
        );
        let dev_only: Vec<Alert> = of("me/two")
            .into_iter()
            .filter(|a| a.package == "vite")
            .collect();
        assert!(gate_reasons("me/two", &dev_only).is_empty());
    }

    #[test]
    fn finds_lock_files() {
        let t = tempfile::tempdir().unwrap();
        for d in ["", "app", "app/node_modules/x", "a/b/c", ".hidden"] {
            let dir = t.path().join(d);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("package-lock.json"), "{}").unwrap();
        }
        let found = lock_files(t.path());
        assert_eq!(found, vec![t.path().to_path_buf(), t.path().join("app")]);
    }
}
