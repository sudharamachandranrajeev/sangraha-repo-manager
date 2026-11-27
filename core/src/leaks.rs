//! The leak check: what a push is about to send to GitHub, read before it leaves.
//!
//! It looks at the commits that are not on GitHub yet: their author and committer emails, their
//! messages and the lines they add. It blocks a private email, a path into this computer's home
//! folder, a word from the owner's private list, and anything that looks like a password or
//! token. `gitleaks`, when installed, adds its own secret rules. Nothing is changed; the check
//! only reads.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::config::{Config, home};
use crate::local::git;
use crate::{Error, Result};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "camelCase")]
pub enum LeakKind {
    /// A private email in a commit's author or committer, or in the text.
    Email,
    /// Something that looks like a password, token or private key.
    Secret,
    /// A path into this computer's home folder, which names the account.
    HomePath,
    /// A word from the owner's private list.
    Term,
    /// A commit without a signature, when signatures are required.
    Unsigned,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Finding {
    pub kind: LeakKind,
    /// Short commit id.
    pub commit: String,
    /// `None` for the commit's emails, signature or message.
    pub file: Option<String>,
    pub line: Option<u32>,
    /// What was found. Secrets are cut to their first few characters.
    pub text: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub commits: u32,
    pub findings: Vec<Finding>,
    /// Checks that could not run, e.g. gitleaks is not installed.
    pub warnings: Vec<String>,
}

/// What counts as a leak for this owner.
#[derive(Debug, Clone, Default)]
pub struct Rules {
    /// Home folder paths, as written on this system (and with `/` on Windows).
    pub home: Vec<String>,
    /// The owner's own emails that are not GitHub's private ones (from git's settings).
    pub own_emails: Vec<String>,
    /// Emails allowed in commits besides `@users.noreply.github.com`.
    pub allowed_emails: Vec<String>,
    /// Private words, lower case.
    pub terms: Vec<String>,
    /// The GitHub login: public by definition, so it never counts as a private word.
    pub login: Option<String>,
    pub require_signed: bool,
}

const NOREPLY: &str = "@users.noreply.github.com";

impl Rules {
    /// The rules from the settings, git's own email setting, and this computer's home folder.
    pub fn for_owner(cfg: &Config, login: Option<String>, repo: &Path) -> Rules {
        let mut allowed: Vec<String> = cfg
            .allowed_emails
            .iter()
            .map(|e| e.trim().to_lowercase())
            .filter(|e| !e.is_empty())
            .collect();
        // Commits made on github.com (a merge button, an edit in the browser).
        allowed.push("noreply@github.com".into());
        let mut rules = Rules {
            home: home_forms(&home()),
            allowed_emails: allowed,
            terms: cfg
                .terms_file
                .as_deref()
                .map(read_terms)
                .unwrap_or_default(),
            login: login.map(|l| l.to_lowercase()),
            require_signed: cfg.require_signed,
            ..Default::default()
        };
        if let Ok(e) = git(repo, &["config", "--get", "user.email"]) {
            let e = e.trim().to_lowercase();
            if !e.is_empty() && !rules.email_ok(&e) {
                rules.own_emails.push(e);
            }
        }
        rules
    }

    fn email_ok(&self, email: &str) -> bool {
        let e = email.to_lowercase();
        e.ends_with(NOREPLY) || self.allowed_emails.contains(&e)
    }
}

/// `/home/me`, and on Windows `C:\Users\me` and `C:/Users/me`. A home folder at the top of the
/// disk (`/root`, say) is too short to be told apart from ordinary text and is left out.
pub fn home_forms(home: &Path) -> Vec<String> {
    let text = home
        .to_string_lossy()
        .trim_end_matches(['/', '\\'])
        .to_owned();
    if home.components().count() < 3 {
        return Vec::new();
    }
    let mut out = vec![text.clone()];
    if text.contains('\\') {
        out.push(text.replace('\\', "/"));
    }
    out
}

/// One private word per line; blank lines and `#` comments skipped; words shorter than 3
/// letters would match everywhere and are left out.
pub fn read_terms(path: &Path) -> Vec<String> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(|l| l.trim().to_lowercase())
        .filter(|l| l.chars().count() >= 3 && !l.starts_with('#'))
        .collect()
}

fn secret_rules() -> &'static [(&'static str, Regex)] {
    static RULES: OnceLock<Vec<(&'static str, Regex)>> = OnceLock::new();
    RULES.get_or_init(|| {
        [
            (
                "GitHub token",
                r"\b(gh[pousr]_[A-Za-z0-9]{36,}|github_pat_[A-Za-z0-9_]{50,})",
            ),
            ("Anthropic key", r"\bsk-ant-[A-Za-z0-9_-]{20,}"),
            ("OpenAI key", r"\bsk-(proj-)?[A-Za-z0-9]{32,}"),
            ("AWS key", r"\b(AKIA|ASIA)[0-9A-Z]{16}\b"),
            ("Google API key", r"\bAIza[0-9A-Za-z_-]{35}"),
            ("Slack token", r"\bxox[abprs]-[A-Za-z0-9-]{10,}"),
            ("Stripe key", r"\b[sr]k_live_[0-9A-Za-z]{24,}"),
            ("Private key", r"-----BEGIN [A-Z ]*PRIVATE KEY-----"),
        ]
        .into_iter()
        .map(|(name, re)| (name, Regex::new(re).expect("valid pattern")))
        .collect()
    })
}

/// The first characters of a secret, enough to find it, not enough to use it.
fn cut(secret: &str) -> String {
    let head: String = secret.chars().take(8).collect();
    format!("{head}…")
}

/// The leaks in one line of text.
pub fn check_text(rules: &Rules, text: &str) -> Vec<(LeakKind, String)> {
    let mut out = Vec::new();
    for (name, re) in secret_rules() {
        if let Some(m) = re.find(text) {
            out.push((LeakKind::Secret, format!("{name}: {}", cut(m.as_str()))));
        }
    }
    let lower = text.to_lowercase();
    for h in &rules.home {
        if lower.contains(&h.to_lowercase()) {
            out.push((LeakKind::HomePath, h.clone()));
        }
    }
    for e in &rules.own_emails {
        if lower.contains(e) {
            out.push((LeakKind::Email, e.clone()));
        }
    }
    if !rules.terms.is_empty() {
        // The login and GitHub's private emails are public; take them out before looking for
        // private words that they might contain.
        let mut public = lower.clone();
        if let Some(l) = &rules.login {
            public = public.replace(l.as_str(), " ");
        }
        for t in &rules.terms {
            if public.contains(t.as_str()) {
                out.push((LeakKind::Term, t.clone()));
            }
        }
    }
    out
}

/// Runs the check over the commits `revs` selects (arguments for `git log`).
pub fn scan(repo: &Path, revs: &[String], rules: &Rules) -> Result<Report> {
    let mut report = Report::default();
    let args = |extra: &[&str]| -> Vec<String> {
        let mut v: Vec<String> = extra.iter().map(|s| s.to_string()).collect();
        v.extend(revs.iter().cloned());
        v
    };
    let run = |a: Vec<String>| -> Result<String> {
        let refs: Vec<&str> = a.iter().map(String::as_str).collect();
        git(repo, &refs).map_err(Error::Git)
    };

    // Emails and signatures.
    let sig = if rules.require_signed { "%x09%G?" } else { "" };
    let heads = run(args(&["log", &format!("--format=%h%x09%ae%x09%ce{sig}")]))?;
    for line in heads.lines().filter(|l| !l.is_empty()) {
        report.commits += 1;
        let mut f = line.split('\t');
        let commit = f.next().unwrap_or_default().to_owned();
        let mut seen = Vec::new();
        for email in [f.next(), f.next()].into_iter().flatten() {
            if !email.is_empty() && !rules.email_ok(email) && !seen.contains(&email) {
                seen.push(email);
                report.findings.push(Finding {
                    kind: LeakKind::Email,
                    commit: commit.clone(),
                    file: None,
                    line: None,
                    text: email.to_owned(),
                });
            }
        }
        if rules.require_signed && matches!(f.next(), Some("N") | None) {
            report.findings.push(Finding {
                kind: LeakKind::Unsigned,
                commit: commit.clone(),
                file: None,
                line: None,
                text: "not signed".into(),
            });
        }
    }
    if report.commits == 0 {
        return Ok(report);
    }

    // Messages and added lines.
    let patch = run(args(&[
        "log",
        "-p",
        "--no-color",
        "--no-ext-diff",
        "--unified=0",
        "--format=%x01%h%n%B%x02",
    ]))?;
    for (commit, file, line, text) in added_lines(&patch) {
        for (kind, found) in check_text(rules, text) {
            report.findings.push(Finding {
                kind,
                commit: commit.to_owned(),
                file: file.map(str::to_owned),
                line,
                text: found,
            });
        }
    }

    match gitleaks(repo, revs) {
        Ok(found) => {
            for f in found {
                let same = report.findings.iter().any(|g| {
                    g.kind == LeakKind::Secret
                        && f.commit.starts_with(&g.commit)
                        && g.file == f.file
                        && g.line == f.line
                });
                if !same {
                    report.findings.push(f);
                }
            }
        }
        Err(w) => report.warnings.push(w),
    }
    report.findings.sort_by_key(|a| a.kind);
    Ok(report)
}

/// `(commit, file, line, text)` for each message line (no file) and each added line of a
/// `git log -p --unified=0 --format=%x01%h%n%B%x02` output.
pub fn added_lines(patch: &str) -> Vec<(&str, Option<&str>, Option<u32>, &str)> {
    let mut out = Vec::new();
    let mut commit = "";
    let mut file: Option<&str> = None;
    let mut next = 0u32;
    let mut in_message = false;
    let mut in_header = false;
    for line in patch.lines() {
        if let Some(c) = line.strip_prefix('\u{1}') {
            commit = c.trim();
            in_message = true;
            file = None;
            continue;
        }
        if in_message {
            match line.strip_suffix('\u{2}') {
                Some(last) => {
                    in_message = false;
                    if !last.is_empty() {
                        out.push((commit, None, None, last));
                    }
                }
                None => out.push((commit, None, None, line)),
            }
            continue;
        }
        if line.starts_with("diff --git ") {
            in_header = true;
            file = None;
        } else if in_header {
            if let Some(p) = line.strip_prefix("+++ ") {
                file = p.strip_prefix("b/").or((p != "/dev/null").then_some(p));
            } else if let Some(h) = line.strip_prefix("@@ ") {
                in_header = false;
                next = hunk_start(h);
            }
        } else if let Some(h) = line.strip_prefix("@@ ") {
            next = hunk_start(h);
        } else if let Some(text) = line.strip_prefix('+') {
            out.push((commit, file, Some(next), text));
            next += 1;
        }
    }
    out
}

/// The first new line number from a hunk header's `+c,d` part.
fn hunk_start(h: &str) -> u32 {
    h.split_whitespace()
        .find_map(|p| p.strip_prefix('+'))
        .and_then(|p| p.split(',').next())
        .and_then(|n| n.parse().ok())
        .unwrap_or(0)
}

/// Secrets found by gitleaks over the same commits, or why it could not run.
fn gitleaks(repo: &Path, revs: &[String]) -> std::result::Result<Vec<Finding>, String> {
    let dir = tempdir_for_report().map_err(|e| format!("gitleaks did not run: {e}"))?;
    let report = dir.join("gitleaks.json");
    let out = std::process::Command::new("gitleaks")
        .arg("git")
        .arg("--no-banner")
        .arg("--redact")
        .arg("--log-opts")
        .arg(revs.join(" "))
        .args([
            "--report-format",
            "json",
            "--exit-code",
            "0",
            "--log-level",
            "error",
        ])
        .arg("--report-path")
        .arg(&report)
        .arg(repo)
        .output();
    let result = match out {
        Err(_) => {
            Err("gitleaks is not installed, so only Sangraha's own secret patterns ran".to_owned())
        }
        Ok(o) if !o.status.success() => Err(format!(
            "gitleaks failed: {}",
            String::from_utf8_lossy(&o.stderr)
                .trim()
                .lines()
                .last()
                .unwrap_or("")
        )),
        Ok(_) => {
            let text = std::fs::read_to_string(&report).unwrap_or_else(|_| "[]".into());
            let list: Vec<serde_json::Value> = serde_json::from_str(&text).unwrap_or_default();
            Ok(list
                .iter()
                .map(|f| Finding {
                    kind: LeakKind::Secret,
                    commit: f["Commit"].as_str().unwrap_or("").chars().take(7).collect(),
                    file: f["File"].as_str().map(str::to_owned),
                    line: f["StartLine"].as_u64().map(|n| n as u32),
                    text: format!("gitleaks: {}", f["RuleID"].as_str().unwrap_or("secret")),
                })
                .collect())
        }
    };
    let _ = std::fs::remove_dir_all(&dir);
    result
}

fn tempdir_for_report() -> std::io::Result<PathBuf> {
    let dir = std::env::temp_dir().join(format!(
        "sangraha-leaks-{}-{}",
        std::process::id(),
        crate::now()
    ));
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// What a `pre-push` hook is told on standard input: one line per ref, `local-ref local-sha
/// remote-ref remote-sha`. Returns the `git log` arguments for the commits each ref sends,
/// and whether a tag is among them (a release, so the release gate runs too).
pub fn push_ranges(repo: &Path, stdin: &str) -> (Vec<Vec<String>>, bool) {
    let zero = |s: &str| !s.is_empty() && s.bytes().all(|b| b == b'0');
    let mut ranges = Vec::new();
    let mut tags = false;
    for line in stdin.lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        let [_, lsha, rref, rsha] = f[..] else {
            continue;
        };
        if zero(lsha) {
            continue; // deleting a branch or tag sends nothing
        }
        tags |= rref.starts_with("refs/tags/");
        let known =
            !zero(rsha) && git(repo, &["cat-file", "-e", &format!("{rsha}^{{commit}}")]).is_ok();
        ranges.push(if known {
            vec![format!("{rsha}..{lsha}")]
        } else {
            vec![lsha.to_owned(), "--not".into(), "--remotes".into()]
        });
    }
    (ranges, tags)
}

/// The leak check over every range, as one report.
pub fn scan_all(repo: &Path, ranges: &[Vec<String>], rules: &Rules) -> Result<Report> {
    let mut all = Report::default();
    for revs in ranges {
        let r = scan(repo, revs, rules)?;
        all.commits += r.commits;
        for f in r.findings {
            if !all.findings.contains(&f) {
                all.findings.push(f);
            }
        }
        for w in r.warnings {
            if !all.warnings.contains(&w) {
                all.warnings.push(w);
            }
        }
    }
    Ok(all)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn rules() -> Rules {
        Rules {
            home: vec!["/home/alice".into()],
            own_emails: vec!["alice@example.org".into()],
            allowed_emails: vec!["noreply@github.com".into()],
            terms: vec!["maple street".into(), "alice".into()],
            login: Some("alice-codes".into()),
            require_signed: false,
        }
    }

    #[test]
    fn text_checks() {
        let r = rules();
        let kinds = |t: &str| {
            check_text(&r, t)
                .into_iter()
                .map(|(k, _)| k)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            kinds("see /home/alice/Work/x"),
            vec![LeakKind::HomePath, LeakKind::Term]
        );
        assert_eq!(
            kinds("mail Alice@Example.org"),
            vec![LeakKind::Email, LeakKind::Term]
        );
        assert_eq!(kinds("12 Maple Street"), vec![LeakKind::Term]);
        // The login is public even though it contains a private word.
        assert!(kinds("https://github.com/alice-codes/app").is_empty());
        assert!(kinds("/home/you/Applications and you@example.com").is_empty());
        let token = format!("ghp_{}", "a1B2".repeat(9));
        let found = check_text(&r, &format!("token = \"{token}\""));
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0, LeakKind::Secret);
        assert!(
            !found[0].1.contains(&token),
            "secret must be cut: {}",
            found[0].1
        );
        // Built at run time, so this file does not trip the leak check itself.
        assert_eq!(
            kinds(&format!("-----BEGIN OPENSSH {}-----", "PRIVATE KEY")),
            vec![LeakKind::Secret]
        );
        let aws = format!("AKIA{}", "IOSFODNN7EXAMPLE");
        assert!(kinds(&aws[..19]).is_empty(), "19 characters is not a key");
        assert_eq!(kinds(&aws), vec![LeakKind::Secret]);
    }

    #[test]
    fn home_forms_by_system() {
        assert_eq!(home_forms(Path::new("/home/alice")), vec!["/home/alice"]);
        assert!(home_forms(Path::new("/root")).is_empty());
        // On Linux a backslash is part of a file name, so this test is for Windows only.
        if cfg!(windows) {
            assert_eq!(
                home_forms(Path::new(r"C:\Users\alice")),
                vec![r"C:\Users\alice".to_owned(), "C:/Users/alice".to_owned()]
            );
        }
    }

    #[test]
    fn patch_lines() {
        let patch = "\u{1}abc1234\nAdd it\n\nBody line\n\u{2}\n\
                     diff --git a/x.txt b/x.txt\nnew file mode 100644\nindex 0..1\n--- /dev/null\n+++ b/x.txt\n\
                     @@ -0,0 +1,2 @@\n+one\n+++two\n@@ -9 +10 @@\n-old\n+ten\n\
                     diff --git a/gone b/gone\ndeleted file mode 100644\n--- a/gone\n+++ /dev/null\n@@ -1 +0,0 @@\n-bye\n";
        let got = added_lines(patch);
        assert_eq!(
            got,
            vec![
                ("abc1234", None, None, "Add it"),
                ("abc1234", None, None, ""),
                ("abc1234", None, None, "Body line"),
                ("abc1234", Some("x.txt"), Some(1), "one"),
                ("abc1234", Some("x.txt"), Some(2), "++two"),
                ("abc1234", Some("x.txt"), Some(10), "ten"),
            ]
        );
    }

    fn git_in(dir: &Path, email: &str, args: &[&str]) {
        let ok = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", email)
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", email)
            .status()
            .unwrap()
            .success();
        assert!(ok, "git {args:?}");
    }

    #[test]
    fn scans_unpushed_commits() {
        let t = tempfile::tempdir().unwrap();
        let d = t.path();
        let good = "1+t@users.noreply.github.com";
        git_in(d, good, &["init", "-q", "-b", "main"]);
        std::fs::write(d.join("a.txt"), "fine\n").unwrap();
        git_in(d, good, &["add", "."]);
        git_in(d, good, &["commit", "-qm", "clean"]);
        // Pretend the first commit is on GitHub already.
        git_in(d, good, &["update-ref", "refs/remotes/origin/main", "HEAD"]);
        std::fs::write(d.join("a.txt"), "fine\nlog at /home/alice/x\n").unwrap();
        git_in(
            d,
            "alice@example.org",
            &["commit", "-qam", "near Maple Street"],
        );

        let revs = vec!["HEAD".into(), "--not".into(), "--remotes".into()];
        let mut r = rules();
        r.own_emails.clear();
        r.terms = vec!["maple street".into()];
        let report = scan(d, &revs, &r).unwrap();
        assert_eq!(report.commits, 1);
        let kinds: Vec<_> = report.findings.iter().map(|f| f.kind).collect();
        assert_eq!(
            kinds,
            vec![LeakKind::Email, LeakKind::HomePath, LeakKind::Term]
        );
        let path = &report.findings[1];
        assert_eq!((path.file.as_deref(), path.line), (Some("a.txt"), Some(2)));
        assert_eq!(report.findings[2].file, None, "found in the message");

        let head = git(d, &["rev-parse", "HEAD"]).unwrap();
        let base = git(d, &["rev-parse", "origin/main"]).unwrap();
        let zero = "0".repeat(40);
        let stdin = format!(
            "refs/heads/main {h} refs/heads/main {b}\nrefs/tags/v1 {h} refs/tags/v1 {zero}\nrefs/heads/old {zero} refs/heads/old {b}\n",
            h = head.trim(),
            b = base.trim()
        );
        let (ranges, tags) = push_ranges(d, &stdin);
        assert!(tags);
        assert_eq!(ranges.len(), 2);
        assert_eq!(ranges[0], vec![format!("{}..{}", base.trim(), head.trim())]);
        let all = scan_all(d, &ranges, &r).unwrap();
        assert_eq!(all.commits, 2, "the same commit through two refs");
        assert_eq!(all.findings.len(), 3, "and its findings once");
    }
}
