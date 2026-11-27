//! Bulk changes: one set of settings, topics or files, applied to many repositories.
//!
//! Every change is planned first: Sangraha reads each repository and lists what would change,
//! what is already so, and what cannot be done. That plan is the dry run. Applying plans again
//! (the repository may have changed since) and then makes only the listed changes. Files go in
//! as commits by the owner's private GitHub email, straight to the default branch or through a
//! pull request.

use std::collections::BTreeMap;
use std::path::PathBuf;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::github::{Reply, call};
use crate::{Error, Result};

/// A repository setting that is either on or off.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "camelCase")]
pub enum Setting {
    /// Dependabot alerts.
    SecurityAlerts,
    /// Dependabot security updates (pull requests that fix alerts).
    SecurityFixes,
    SecretScanning,
    /// Secret scanning blocks a push that contains a secret.
    PushProtection,
    DeleteBranchOnMerge,
    AutoMerge,
    SquashMerge,
    MergeCommit,
    RebaseMerge,
}

impl Setting {
    pub fn label(self) -> &'static str {
        match self {
            Setting::SecurityAlerts => "Dependabot alerts",
            Setting::SecurityFixes => "Dependabot security updates",
            Setting::SecretScanning => "Secret scanning",
            Setting::PushProtection => "Push protection",
            Setting::DeleteBranchOnMerge => "Delete branch after merge",
            Setting::AutoMerge => "Allow auto-merge",
            Setting::SquashMerge => "Allow squash merging",
            Setting::MergeCommit => "Allow merge commits",
            Setting::RebaseMerge => "Allow rebase merging",
        }
    }

    /// The field in `GET`/`PATCH /repos/{repo}` for settings that live there.
    fn repo_field(self) -> Option<&'static str> {
        match self {
            Setting::DeleteBranchOnMerge => Some("delete_branch_on_merge"),
            Setting::AutoMerge => Some("allow_auto_merge"),
            Setting::SquashMerge => Some("allow_squash_merge"),
            Setting::MergeCommit => Some("allow_merge_commit"),
            Setting::RebaseMerge => Some("allow_rebase_merge"),
            _ => None,
        }
    }

    /// The `security_and_analysis` key for the scanning settings.
    fn analysis_key(self) -> Option<&'static str> {
        match self {
            Setting::SecretScanning => Some("secret_scanning"),
            Setting::PushProtection => Some("secret_scanning_push_protection"),
            _ => None,
        }
    }
}

/// Where a file's content comes from.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Source {
    /// A Dependabot config made for each repository from the manifests it holds.
    Dependabot,
    /// The same file from another repository (its default branch).
    Copy {
        repo: String,
        path: String,
    },
    Text {
        text: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FileChange {
    /// Where the file goes in each repository, e.g. `.github/dependabot.yml`.
    pub path: String,
    pub source: Source,
    /// Replace the file where it exists and differs. Without it, existing files are left.
    #[serde(default)]
    pub replace: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Request {
    pub repos: Vec<String>,
    #[serde(default)]
    pub settings: BTreeMap<Setting, bool>,
    #[serde(default)]
    pub add_topics: Vec<String>,
    #[serde(default)]
    pub remove_topics: Vec<String>,
    #[serde(default)]
    pub files: Vec<FileChange>,
    /// Files through a pull request instead of a commit on the default branch.
    #[serde(default)]
    pub pull_request: bool,
    /// The commit message and pull request title for the files.
    #[serde(default)]
    pub message: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Step {
    pub label: String,
    pub now: String,
    pub then: String,
    /// Applying would change something.
    pub change: bool,
    /// Why nothing changes, when it cannot (or need not).
    pub note: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RepoPlan {
    pub repo: String,
    pub steps: Vec<Step>,
    /// The repository could not be read (or, after applying, a change failed).
    pub error: Option<String>,
    /// After applying: what was done.
    pub done: Vec<String>,
    pub pull_request: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    pub repos: Vec<RepoPlan>,
    /// Steps that would change something, in all repositories.
    pub changes: u32,
    /// The commit author for files: the owner's private GitHub email.
    pub author: String,
    pub applied: bool,
}

/// One change GitHub will be asked to make.
#[derive(Debug, Clone, PartialEq)]
enum Op {
    Alerts(bool),
    Fixes(bool),
    /// A `PATCH /repos/{repo}` body.
    Patch(Value),
    Topics(Vec<String>),
    File {
        path: String,
        content: Vec<u8>,
        sha: Option<String>,
    },
}

/// What every repository shares: the request checked, copied files read once, the author.
struct Context<'a> {
    token: &'a str,
    req: &'a Request,
    copies: BTreeMap<String, Vec<u8>>,
    clones: BTreeMap<String, PathBuf>,
}

// ---------------------------------------------------------------------------------------------
// Checking the request

/// A topic as GitHub accepts it: lower case letters, digits and hyphens, at most 50.
pub fn valid_topic(t: &str) -> bool {
    !t.is_empty()
        && t.len() <= 50
        && !t.starts_with('-')
        && t.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// A path inside a repository: relative, no `..`, not into `.git`.
pub fn valid_path(p: &str) -> bool {
    !p.is_empty()
        && !p.starts_with('/')
        && !p.contains('\\')
        && p.split('/')
            .all(|part| !part.is_empty() && part != "." && part != ".." && part != ".git")
}

/// Checks a request against the repositories the owner has (`owned`, lower case).
pub fn check(req: &Request, owned: &[String]) -> Result<()> {
    let bad = |m: String| Err(Error::Install(m));
    if req.repos.is_empty() {
        return bad("Choose at least one repository".into());
    }
    for r in &req.repos {
        if !owned.contains(&r.to_lowercase()) {
            return bad(format!("{r} is not one of your repositories"));
        }
    }
    if req.settings.is_empty()
        && req.add_topics.is_empty()
        && req.remove_topics.is_empty()
        && req.files.is_empty()
    {
        return bad("Choose something to change".into());
    }
    for t in req.add_topics.iter().chain(&req.remove_topics) {
        if !valid_topic(t) {
            return bad(format!(
                "\"{t}\" is not a GitHub topic (lower case letters, digits and hyphens)"
            ));
        }
    }
    let merges = [
        Setting::SquashMerge,
        Setting::MergeCommit,
        Setting::RebaseMerge,
    ];
    if merges.iter().all(|m| req.settings.get(m) == Some(&false)) {
        return bad("A repository needs at least one way to merge".into());
    }
    let mut paths = Vec::new();
    for f in &req.files {
        if !valid_path(&f.path) {
            return bad(format!("{}: not a path inside a repository", f.path));
        }
        if paths.contains(&&f.path) {
            return bad(format!("{} is listed twice", f.path));
        }
        paths.push(&f.path);
        match &f.source {
            Source::Copy { repo, path } if !crate::apps::valid_name(repo) || !valid_path(path) => {
                return bad(format!(
                    "Copy from: write owner/name and a path ({repo} {path})"
                ));
            }
            Source::Text { text } if text.len() > 512 * 1024 => {
                return bad(format!("{}: the text is too long", f.path));
            }
            _ => {}
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Dependabot config from a repository's files

fn ecosystem(path: &str) -> Option<(&'static str, String)> {
    let (dir, name) = match path.rsplit_once('/') {
        Some((d, n)) => (format!("/{d}"), n),
        None => ("/".to_owned(), path),
    };
    if dir.starts_with("/.github/workflows") && (name.ends_with(".yml") || name.ends_with(".yaml"))
    {
        return Some(("github-actions", "/".into()));
    }
    let skipped = [
        "node_modules",
        "vendor",
        "third_party",
        ".dart_tool",
        "build",
        "dist",
        "target",
        "example",
        "examples",
        "fixtures",
        "testdata",
    ];
    if dir.split('/').any(|p| skipped.contains(&p)) {
        return None;
    }
    let eco = match name {
        "package.json" => "npm",
        "Cargo.toml" => "cargo",
        "pubspec.yaml" => "pub",
        "requirements.txt" | "pyproject.toml" | "Pipfile" | "setup.py" => "pip",
        "go.mod" => "gomod",
        "Gemfile" => "bundler",
        "composer.json" => "composer",
        "pom.xml" => "maven",
        "build.gradle" | "build.gradle.kts" => "gradle",
        "Dockerfile" => "docker",
        _ => return None,
    };
    Some((eco, dir))
}

/// A Dependabot config (`version: 2`) for a repository holding `files` (paths from its root).
/// `None` when nothing in it is something Dependabot updates.
pub fn dependabot_yml(files: &[String]) -> Option<String> {
    let mut map: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for f in files {
        if let Some((eco, dir)) = ecosystem(f) {
            let dirs = map.entry(eco).or_default();
            if !dirs.contains(&dir) {
                dirs.push(dir);
            }
        }
    }
    // A Cargo workspace's members are updated through the workspace root.
    if let Some(dirs) = map.get_mut("cargo") {
        let all = dirs.clone();
        dirs.retain(|d| {
            !all.iter()
                .any(|o| o != d && (o == "/" || d.starts_with(&format!("{o}/"))))
        });
    }
    if map.is_empty() {
        return None;
    }
    let mut out = String::from(
        "# Made by Sangraha from the manifests in this repository.\n# Minor and patch updates come grouped in one pull request a week.\nversion: 2\nupdates:\n",
    );
    for (eco, mut dirs) in map {
        dirs.sort();
        out.push_str(&format!(
            "  - package-ecosystem: \"{eco}\"\n    directories:\n"
        ));
        for d in dirs {
            out.push_str(&format!("      - \"{d}\"\n"));
        }
        out.push_str(
            "    schedule:\n      interval: \"weekly\"\n    open-pull-requests-limit: 5\n    groups:\n      minor-and-patch:\n        update-types: [\"minor\", \"patch\"]\n",
        );
    }
    Some(out)
}

// ---------------------------------------------------------------------------------------------
// Planning

fn on_off(b: bool) -> String {
    if b { "on" } else { "off" }.into()
}

fn get(ctx: &Context, path: &str) -> Result<Reply> {
    call(ctx.token, "GET", path, None)
}

/// The repository's files: from its clone here when there is one, else from GitHub.
fn file_list(ctx: &Context, repo: &str, branch: &str) -> Result<(Vec<String>, String)> {
    if let Some(dir) = ctx.clones.get(&repo.to_lowercase())
        && let Ok(out) = crate::local::git(dir, &["ls-files"])
    {
        return Ok((
            out.lines().map(str::to_owned).collect(),
            format!("from the clone at {}", dir.display()),
        ));
    }
    let r = get(ctx, &format!("repos/{repo}/git/trees/{branch}?recursive=1"))?;
    if !r.ok() {
        return Err(Error::GitHub(format!(
            "cannot list its files: {}",
            r.message()
        )));
    }
    let list = r.body["tree"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|e| e["type"] == "blob")
        .filter_map(|e| e["path"].as_str().map(str::to_owned))
        .collect();
    Ok((list, "from GitHub".into()))
}

/// Plans one repository: the steps to show and the operations that would make them.
fn plan_repo(ctx: &Context, repo: &str) -> (RepoPlan, Vec<Op>) {
    let mut plan = RepoPlan {
        repo: repo.to_owned(),
        ..Default::default()
    };
    match plan_steps(ctx, repo, &mut plan.steps) {
        Ok(ops) => (plan, ops),
        Err(e) => {
            plan.error = Some(e.to_string());
            (plan, Vec::new())
        }
    }
}

fn plan_steps(ctx: &Context, repo: &str, steps: &mut Vec<Step>) -> Result<Vec<Op>> {
    let req = ctx.req;
    let info = get(ctx, &format!("repos/{repo}"))?;
    if !info.ok() {
        return Err(Error::GitHub(info.message()));
    }
    let info = info.body;
    if info["archived"].as_bool() == Some(true) {
        return Err(Error::GitHub("archived: GitHub allows no changes".into()));
    }
    if info["permissions"]["admin"].as_bool() == Some(false) {
        return Err(Error::GitHub("you are not an admin of it".into()));
    }
    let branch = info["default_branch"].as_str().unwrap_or("main").to_owned();
    let mut ops = Vec::new();
    let mut step = |label: &str, now: String, then: String, note: Option<&str>| {
        let change = note.is_none() && now != then;
        steps.push(Step {
            label: label.to_owned(),
            note: note
                .map(str::to_owned)
                .or_else(|| (!change).then(|| "already so".to_owned())),
            now,
            then,
            change,
        });
        change
    };

    // Dependabot alerts and security updates have their own endpoints.
    let mut alerts_on = None;
    if req.settings.contains_key(&Setting::SecurityAlerts)
        || req.settings.contains_key(&Setting::SecurityFixes)
    {
        let r = get(ctx, &format!("repos/{repo}/vulnerability-alerts"))?;
        alerts_on = Some(r.status == 204);
    }
    if let Some(&want) = req.settings.get(&Setting::SecurityAlerts) {
        let now = alerts_on.unwrap_or(false);
        if step(
            Setting::SecurityAlerts.label(),
            on_off(now),
            on_off(want),
            None,
        ) {
            ops.push(Op::Alerts(want));
        }
        alerts_on = Some(want);
    }
    if let Some(&want) = req.settings.get(&Setting::SecurityFixes) {
        let r = get(ctx, &format!("repos/{repo}/automated-security-fixes"))?;
        let now = r.ok() && r.body["enabled"].as_bool() == Some(true);
        let note = (want && alerts_on != Some(true)).then_some("needs Dependabot alerts on");
        if step(
            Setting::SecurityFixes.label(),
            on_off(now),
            on_off(want),
            note,
        ) {
            ops.push(Op::Fixes(want));
        }
    }

    // Scanning and merge settings go in one PATCH.
    let mut patch = serde_json::Map::new();
    let mut analysis = serde_json::Map::new();
    for (&s, &want) in &req.settings {
        if let Some(key) = s.analysis_key() {
            let status = info["security_and_analysis"][key]["status"].as_str();
            let note = match status {
                None => Some("not offered for this repository"),
                Some(_) if s == Setting::PushProtection && want => {
                    let scanning = info["security_and_analysis"]["secret_scanning"]["status"]
                        .as_str()
                        == Some("enabled")
                        || req.settings.get(&Setting::SecretScanning) == Some(&true);
                    (!scanning).then_some("needs secret scanning on")
                }
                Some(_) => None,
            };
            let now = status == Some("enabled");
            if step(s.label(), on_off(now), on_off(want), note) {
                analysis.insert(
                    key.into(),
                    json!({ "status": if want { "enabled" } else { "disabled" } }),
                );
            }
        } else if let Some(field) = s.repo_field() {
            let now = info[field].as_bool().unwrap_or(false);
            if step(s.label(), on_off(now), on_off(want), None) {
                patch.insert(field.into(), json!(want));
            }
        }
    }
    if !analysis.is_empty() {
        patch.insert("security_and_analysis".into(), Value::Object(analysis));
    }
    if !patch.is_empty() {
        ops.push(Op::Patch(Value::Object(patch)));
    }

    // Topics.
    if !req.add_topics.is_empty() || !req.remove_topics.is_empty() {
        let now: Vec<String> = info["topics"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|t| t.as_str().map(str::to_owned))
            .collect();
        let mut then: Vec<String> = now
            .iter()
            .filter(|t| !req.remove_topics.contains(t))
            .cloned()
            .collect();
        for t in &req.add_topics {
            if !then.contains(t) {
                then.push(t.clone());
            }
        }
        let note = (then.len() > 20).then_some("GitHub allows 20 topics at most");
        let show = |v: &[String]| {
            if v.is_empty() {
                "none".to_owned()
            } else {
                v.join(", ")
            }
        };
        if step("Topics", show(&now), show(&then), note) {
            ops.push(Op::Topics(then));
        }
    }

    // Files.
    for f in &req.files {
        let (content, origin) = match &f.source {
            Source::Text { text } => (text.as_bytes().to_vec(), None),
            Source::Copy { repo: from, path } => (
                ctx.copies
                    .get(&format!("{from}/{path}"))
                    .cloned()
                    .unwrap_or_default(),
                None,
            ),
            Source::Dependabot => {
                let (files, origin) = file_list(ctx, repo, &branch)?;
                match dependabot_yml(&files) {
                    Some(y) => (y.into_bytes(), Some(origin)),
                    None => {
                        step(
                            &f.path,
                            "—".into(),
                            "—".into(),
                            Some("no manifests Dependabot can update"),
                        );
                        continue;
                    }
                }
            }
        };
        let r = get(
            ctx,
            &format!(
                "repos/{repo}/contents/{}?ref={branch}",
                encode_path(&f.path)
            ),
        )?;
        let (now, sha, same) = match r.status {
            404 => ("missing".to_owned(), None, false),
            s if (200..300).contains(&s) => {
                let existing = B64
                    .decode(
                        r.body["content"]
                            .as_str()
                            .unwrap_or("")
                            .replace(['\n', '\r'], ""),
                    )
                    .unwrap_or_default();
                (
                    "exists".to_owned(),
                    r.body["sha"].as_str().map(str::to_owned),
                    existing == content,
                )
            }
            _ => return Err(Error::GitHub(format!("{}: {}", f.path, r.message()))),
        };
        let then = match (&origin, sha.is_some()) {
            (Some(o), false) => format!("added (made {o})"),
            (Some(o), true) => format!("replaced (made {o})"),
            (None, false) => "added".into(),
            (None, true) => "replaced".into(),
        };
        let note = if same {
            Some("already the same")
        } else if sha.is_some() && !f.replace {
            Some("exists and differs; not replaced (tick \"replace\" to change it)")
        } else {
            None
        };
        if step(&f.path, now, then, note) {
            ops.push(Op::File {
                path: f.path.clone(),
                content,
                sha,
            });
        }
    }
    Ok(ops)
}

/// A repository path for a URL: each part percent-encoded, slashes kept.
fn encode_path(p: &str) -> String {
    p.split('/')
        .map(|part| {
            part.bytes()
                .map(|b| match b {
                    b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                        (b as char).to_string()
                    }
                    _ => format!("%{b:02X}"),
                })
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn context<'a>(token: &'a str, req: &'a Request, roots: &[PathBuf]) -> Result<Context<'a>> {
    let mut copies = BTreeMap::new();
    for f in &req.files {
        if let Source::Copy { repo, path } = &f.source {
            let r = call(
                token,
                "GET",
                &format!("repos/{repo}/contents/{}", encode_path(path)),
                None,
            )?;
            if !r.ok() || r.body["encoding"] != "base64" {
                return Err(Error::GitHub(format!(
                    "cannot read {path} in {repo}: {}",
                    if r.ok() {
                        "not a single file under 1 MB".into()
                    } else {
                        r.message()
                    }
                )));
            }
            let bytes = B64
                .decode(
                    r.body["content"]
                        .as_str()
                        .unwrap_or("")
                        .replace(['\n', '\r'], ""),
                )
                .map_err(|e| Error::GitHub(e.to_string()))?;
            copies.insert(format!("{repo}/{path}"), bytes);
        }
    }
    let mut clones = BTreeMap::new();
    if req.files.iter().any(|f| f.source == Source::Dependabot) {
        for path in crate::local::find(roots) {
            if let Some(name) = crate::local::git(&path, &["config", "--get", "remote.origin.url"])
                .ok()
                .and_then(|u| crate::local::github_name(&u))
            {
                clones.entry(name.to_lowercase()).or_insert(path);
            }
        }
    }
    Ok(Context {
        token,
        req,
        copies,
        clones,
    })
}

/// The owner's private commit email, `id+login@users.noreply.github.com`.
fn author(token: &str) -> Result<(String, String)> {
    let r = call(token, "GET", "user", None)?;
    let (Some(id), Some(login)) = (r.body["id"].as_u64(), r.body["login"].as_str()) else {
        return Err(Error::GitHub(format!(
            "cannot read your account: {}",
            r.message()
        )));
    };
    Ok((
        login.to_owned(),
        format!("{id}+{login}@users.noreply.github.com"),
    ))
}

fn plan_all(ctx: &Context) -> Vec<(RepoPlan, Vec<Op>)> {
    let repos = &ctx.req.repos;
    let chunk = repos.len().div_ceil(6).max(1);
    std::thread::scope(|s| {
        let handles: Vec<_> = repos
            .chunks(chunk)
            .map(|part| s.spawn(move || part.iter().map(|r| plan_repo(ctx, r)).collect::<Vec<_>>()))
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().unwrap_or_default())
            .collect()
    })
}

/// The dry run: what applying `req` would change in each repository. Only reads.
pub fn plan(token: &str, req: &Request, owned: &[String], roots: &[PathBuf]) -> Result<Plan> {
    check(req, owned)?;
    let ctx = context(token, req, roots)?;
    let (_, email) = author(token)?;
    let repos: Vec<RepoPlan> = plan_all(&ctx).into_iter().map(|(p, _)| p).collect();
    let changes = repos
        .iter()
        .flat_map(|r| &r.steps)
        .filter(|s| s.change)
        .count() as u32;
    Ok(Plan {
        repos,
        changes,
        author: email,
        applied: false,
    })
}

// ---------------------------------------------------------------------------------------------
// Applying

fn must(r: Reply, what: &str) -> Result<Reply> {
    if r.ok() {
        Ok(r)
    } else {
        Err(Error::GitHub(format!("{what}: {}", r.message())))
    }
}

fn message(req: &Request, paths: &[&str]) -> String {
    let m = req.message.trim();
    if m.is_empty() {
        format!("Add {}", paths.join(", "))
    } else {
        m.to_owned()
    }
}

/// Makes one repository's changes; returns what was done and the pull request, if one opened.
fn apply_repo(
    token: &str,
    req: &Request,
    repo: &str,
    ops: Vec<Op>,
    who: &(String, String),
    done: &mut Vec<String>,
) -> Result<Option<String>> {
    let person = json!({ "name": who.0, "email": who.1 });
    let mut files = Vec::new();
    for op in ops {
        match op {
            Op::Alerts(on) => {
                let m = if on { "PUT" } else { "DELETE" };
                must(
                    call(
                        token,
                        m,
                        &format!("repos/{repo}/vulnerability-alerts"),
                        None,
                    )?,
                    "Dependabot alerts",
                )?;
                done.push(format!("Dependabot alerts {}", on_off(on)));
            }
            Op::Fixes(on) => {
                let m = if on { "PUT" } else { "DELETE" };
                must(
                    call(
                        token,
                        m,
                        &format!("repos/{repo}/automated-security-fixes"),
                        None,
                    )?,
                    "Dependabot security updates",
                )?;
                done.push(format!("Dependabot security updates {}", on_off(on)));
            }
            Op::Patch(body) => {
                must(
                    call(token, "PATCH", &format!("repos/{repo}"), Some(&body))?,
                    "settings",
                )?;
                done.push("settings saved".into());
            }
            Op::Topics(names) => {
                must(
                    call(
                        token,
                        "PUT",
                        &format!("repos/{repo}/topics"),
                        Some(&json!({ "names": names })),
                    )?,
                    "topics",
                )?;
                done.push("topics saved".into());
            }
            Op::File { path, content, sha } => files.push((path, content, sha)),
        }
    }
    if files.is_empty() {
        return Ok(None);
    }
    let info = must(
        call(token, "GET", &format!("repos/{repo}"), None)?,
        "reading the repository",
    )?;
    let base = info.body["default_branch"]
        .as_str()
        .unwrap_or("main")
        .to_owned();
    let paths: Vec<&str> = files.iter().map(|(p, _, _)| p.as_str()).collect();
    let msg = message(req, &paths);
    let branch = if req.pull_request {
        let head = must(
            call(
                token,
                "GET",
                &format!("repos/{repo}/git/ref/heads/{base}"),
                None,
            )?,
            "reading the default branch",
        )?;
        let sha = head.body["object"]["sha"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        let name = format!("sangraha/{}", crate::now());
        must(
            call(
                token,
                "POST",
                &format!("repos/{repo}/git/refs"),
                Some(&json!({ "ref": format!("refs/heads/{name}"), "sha": sha })),
            )?,
            "making a branch",
        )?;
        name
    } else {
        base.clone()
    };
    for (path, content, sha) in &files {
        let mut body = json!({
            "message": msg,
            "content": B64.encode(content),
            "branch": branch,
            "author": person,
            "committer": person,
        });
        if let Some(sha) = sha {
            body["sha"] = json!(sha);
        }
        must(
            call(
                token,
                "PUT",
                &format!("repos/{repo}/contents/{}", encode_path(path)),
                Some(&body),
            )?,
            path,
        )?;
        done.push(format!("{path} committed to {branch}"));
    }
    if !req.pull_request {
        return Ok(None);
    }
    let pr = must(
        call(
            token,
            "POST",
            &format!("repos/{repo}/pulls"),
            Some(&json!({
                "title": msg,
                "head": branch,
                "base": base,
                "body": format!("Changed by Sangraha: {}.", paths.join(", ")),
            })),
        )?,
        "opening the pull request",
    )?;
    Ok(pr.body["html_url"].as_str().map(str::to_owned))
}

/// Plans again (things may have changed since the preview), then makes the planned changes,
/// one repository after another. A repository that fails does not stop the others.
pub fn apply(
    token: &str,
    req: &Request,
    owned: &[String],
    roots: &[PathBuf],
    progress: &(dyn Fn(&str) + Sync),
) -> Result<Plan> {
    check(req, owned)?;
    let ctx = context(token, req, roots)?;
    let who = author(token)?;
    let mut out = Plan {
        author: who.1.clone(),
        applied: true,
        ..Default::default()
    };
    for (mut plan, ops) in plan_all(&ctx) {
        progress(&plan.repo);
        out.changes += plan.steps.iter().filter(|s| s.change).count() as u32;
        if plan.error.is_none() && !ops.is_empty() {
            let mut done = Vec::new();
            match apply_repo(token, req, &plan.repo, ops, &who, &mut done) {
                Ok(pr) => plan.pull_request = pr,
                Err(e) => plan.error = Some(e.to_string()),
            }
            plan.done = done;
        }
        out.repos.push(plan);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dependabot_from_manifests() {
        let files: Vec<String> = [
            "package.json",
            "package-lock.json",
            "app/package.json",
            "app/node_modules/x/package.json",
            "Cargo.toml",
            "core/Cargo.toml",
            "cli/Cargo.toml",
            "app/src-tauri/Cargo.toml",
            "mobile/pubspec.yaml",
            "mobile/example/pubspec.yaml",
            ".github/workflows/ci.yml",
            ".github/workflows/release.yaml",
            "README.md",
        ]
        .map(String::from)
        .to_vec();
        let y = dependabot_yml(&files).unwrap();
        let ecos: Vec<&str> = y
            .lines()
            .filter_map(|l| l.trim().strip_prefix("- package-ecosystem: "))
            .collect();
        assert_eq!(
            ecos,
            vec!["\"cargo\"", "\"github-actions\"", "\"npm\"", "\"pub\""]
        );
        // The workspace root covers its members.
        let cargo = y.split("package-ecosystem").nth(1).unwrap();
        assert!(
            cargo.contains("- \"/\"\n") && !cargo.contains("/core"),
            "{cargo}"
        );
        let npm = y.split("package-ecosystem").nth(3).unwrap();
        assert!(
            npm.contains("- \"/\"\n")
                && npm.contains("- \"/app\"\n")
                && !npm.contains("node_modules"),
            "{npm}"
        );
        let pubs = y.split("package-ecosystem").nth(4).unwrap();
        assert!(
            pubs.contains("/mobile\"") && !pubs.contains("example"),
            "{pubs}"
        );
        assert!(dependabot_yml(&["README.md".into()]).is_none());
    }

    #[test]
    fn requests_are_checked() {
        let owned = vec!["me/a".to_owned(), "me/b".to_owned()];
        let mut req = Request {
            repos: vec!["me/a".into()],
            add_topics: vec!["rust".into()],
            ..Default::default()
        };
        assert!(check(&req, &owned).is_ok());
        req.repos.push("someone/else".into());
        assert!(check(&req, &owned).is_err(), "not the owner's");
        req.repos.pop();
        req.add_topics = vec!["Has Space".into()];
        assert!(check(&req, &owned).is_err(), "bad topic");
        req.add_topics.clear();
        assert!(check(&req, &owned).is_err(), "nothing to change");
        for m in [
            Setting::SquashMerge,
            Setting::MergeCommit,
            Setting::RebaseMerge,
        ] {
            req.settings.insert(m, false);
        }
        assert!(check(&req, &owned).is_err(), "no way to merge");
        req.settings.insert(Setting::SquashMerge, true);
        assert!(check(&req, &owned).is_ok());
        for bad in ["../x", "/abs", ".git/config", "a//b", "a\\b", ""] {
            req.files = vec![FileChange {
                path: bad.into(),
                source: Source::Text { text: "x".into() },
                replace: false,
            }];
            assert!(check(&req, &owned).is_err(), "{bad}");
        }
        req.files[0].path = ".github/dependabot.yml".into();
        assert!(check(&req, &owned).is_ok());
    }

    #[test]
    fn request_json_from_the_window() {
        let req: Request = serde_json::from_value(json!({
            "repos": ["me/a"],
            "settings": { "secretScanning": true, "deleteBranchOnMerge": false },
            "files": [
                { "path": ".github/dependabot.yml", "source": { "kind": "dependabot" } },
                { "path": "LICENSE", "source": { "kind": "copy", "repo": "me/b", "path": "LICENSE" }, "replace": true }
            ],
            "pullRequest": true
        }))
        .unwrap();
        assert_eq!(req.settings.get(&Setting::SecretScanning), Some(&true));
        assert_eq!(
            req.files[1].source,
            Source::Copy {
                repo: "me/b".into(),
                path: "LICENSE".into()
            }
        );
        assert!(req.pull_request && req.files[1].replace && !req.files[0].replace);
    }

    #[test]
    fn paths_in_urls() {
        assert_eq!(
            encode_path(".github/dependabot.yml"),
            ".github/dependabot.yml"
        );
        assert_eq!(encode_path("docs/a b#c.md"), "docs/a%20b%23c.md");
    }
}
