//! The daily digest: one short summary of every tab, with what is new since the last one.
//!
//! It can go to a desktop notification, and to a command of the owner's choosing that gets the
//! digest as JSON on its standard input: a reminder system, an agent, a
//! mail script. A daily timer (systemd on Linux, Task Scheduler on Windows) runs it.

use std::collections::HashSet;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::{Deserialize, Serialize};

use crate::apps::{AppStatus, AppsBoard};
use crate::board::{Board, Level};
use crate::housekeeping::TidyBoard;
use crate::releases::{ReleaseBoard, Verdict};
use crate::security::{SecurityBoard, Severity};
use crate::subtrees::SubtreeState;
use crate::{Error, Result};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DigestItem {
    /// Stays the same while the thing it is about stays the same, so "new" can be told.
    pub id: String,
    /// "Repos", "Apps", "Security", "Releases".
    pub area: String,
    pub text: String,
    /// Not in the last digest.
    pub new: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Digest {
    /// Unix seconds.
    pub at: i64,
    /// One line: counts from every tab.
    pub summary: String,
    pub items: Vec<DigestItem>,
    pub new: u32,
}

/// What the boards say, as items. Housekeeping adds to the summary only.
pub fn build(
    board: Option<&Board>,
    apps: Option<&AppsBoard>,
    security: Option<&SecurityBoard>,
    releases: Option<&ReleaseBoard>,
    tidy: Option<&TidyBoard>,
    seen: &HashSet<String>,
) -> Digest {
    let mut items = Vec::new();
    let mut parts = Vec::new();
    let mut add = |id: String, area: &str, text: String| {
        items.push(DigestItem {
            new: !seen.contains(&id),
            id,
            area: area.to_owned(),
            text,
        });
    };
    if let Some(b) = board {
        if b.counts.attention > 0 {
            parts.push(format!("{} repos need attention", b.counts.attention));
        }
        for r in b.rows.iter().filter(|r| !r.hidden) {
            for p in r.problems.iter().filter(|p| p.level == Level::High) {
                add(
                    format!("repo:{}:{:?}", r.key, p.kind),
                    "Repos",
                    format!("{}: {}", r.name, p.text),
                );
            }
        }
    }
    if let Some(a) = apps {
        if a.counts.updates > 0 {
            parts.push(format!("{} app updates", a.counts.updates));
        }
        for r in a
            .rows
            .iter()
            .filter(|r| r.status == AppStatus::Update && !r.hidden)
        {
            let to = r.offer.as_ref().map(|o| o.tag.as_str()).unwrap_or("?");
            let from = r
                .here
                .as_ref()
                .and_then(|h| h.version.as_deref())
                .unwrap_or("?");
            add(
                format!("app:{}:{to}", r.key),
                "Apps",
                format!("{}: {from} → {to}", r.name),
            );
        }
    }
    if let Some(s) = security {
        let serious: Vec<_> = s
            .groups
            .iter()
            .filter(|g| g.shipped && matches!(g.severity, Severity::Critical | Severity::High))
            .collect();
        if !serious.is_empty() {
            parts.push(format!(
                "{} serious alerts in shipped packages",
                serious.len()
            ));
        }
        for g in serious {
            add(
                format!("sec:{}:{}:{:?}", g.ecosystem, g.package, g.severity),
                "Security",
                format!(
                    "{} ({:?}) in {}: {}",
                    g.package,
                    g.severity,
                    g.repos
                        .iter()
                        .map(|r| r.split_once('/').map_or(r.as_str(), |(_, n)| n))
                        .collect::<Vec<_>>()
                        .join(", "),
                    g.fix
                        .as_ref()
                        .map_or("no fix yet".to_owned(), |f| format!("fixed in {f}"))
                ),
            );
        }
        if s.counts.secrets > 0 {
            parts.push(format!("{} secret scanning alerts", s.counts.secrets));
        }
    }
    if let Some(r) = releases {
        if r.counts.ready > 0 {
            parts.push(format!("{} ready to tag", r.counts.ready));
        }
        if r.counts.drift > 0 {
            parts.push(format!("{} subtree copies behind", r.counts.drift));
        }
        for row in r
            .rows
            .iter()
            .filter(|x| x.verdict == Verdict::Ready || x.unreleased_tag)
        {
            let what = if row.verdict == Verdict::Ready {
                format!("ready to tag {}", row.version.as_deref().unwrap_or(""))
            } else {
                format!(
                    "{} is tagged but not released",
                    row.tag.as_deref().unwrap_or("")
                )
            };
            add(
                format!("rel:{}:{what}", row.repo),
                "Releases",
                format!("{}: {what}", row.repo),
            );
        }
        for s in r
            .subtrees
            .iter()
            .filter(|s| s.state == SubtreeState::Behind)
        {
            add(
                format!("sub:{}:{}:{}", s.host, s.dir, s.behind.unwrap_or(0)),
                "Releases",
                format!(
                    "{} in {}: {} commits behind {}",
                    s.dir,
                    s.host,
                    s.behind.unwrap_or(0),
                    s.source.as_deref().unwrap_or("its source")
                ),
            );
        }
    }
    if let Some(t) = tidy
        && t.counts.merged > 0
    {
        parts.push(format!("{} merged branches to delete", t.counts.merged));
    }
    let new = items.iter().filter(|i| i.new).count() as u32;
    Digest {
        at: crate::now(),
        summary: if parts.is_empty() {
            "Nothing needs you".into()
        } else {
            parts.join(" · ")
        },
        items,
        new,
    }
}

/// Plain text for a terminal, a notification body or a mail.
pub fn text(d: &Digest) -> String {
    let mut out = format!("{}\n", d.summary);
    let mut area = "";
    for i in &d.items {
        if i.area != area {
            area = &i.area;
            out.push_str(&format!("\n{area}\n"));
        }
        out.push_str(&format!(
            "  {} {}\n",
            if i.new { "new" } else { "   " },
            i.text
        ));
    }
    out
}

/// A desktop notification: the summary, and what is new.
pub fn notify(d: &Digest) -> Result<()> {
    let mut body = d.summary.clone();
    let new: Vec<&str> = d
        .items
        .iter()
        .filter(|i| i.new)
        .map(|i| i.text.as_str())
        .collect();
    if !new.is_empty() {
        body.push_str("\nNew: ");
        body.push_str(&new[..new.len().min(3)].join("; "));
        if new.len() > 3 {
            body.push_str(&format!(" and {} more", new.len() - 3));
        }
    }
    notify_rust::Notification::new()
        .appname("Sangraha")
        .summary("Sangraha")
        .body(&body)
        .show()
        .map(|_| ())
        .map_err(|e| Error::Install(format!("desktop notification: {e}")))
}

/// Runs the owner's command with the digest as JSON on its standard input.
pub fn run_hook(command: &str, d: &Digest) -> Result<()> {
    let mut cmd = if cfg!(windows) {
        let mut c = Command::new("cmd");
        c.args(["/C", command]);
        c
    } else {
        let mut c = Command::new("sh");
        c.args(["-c", command]);
        c
    };
    let mut child = cmd.stdin(Stdio::piped()).spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(&serde_json::to_vec(d).unwrap_or_default())?;
    }
    let status = child.wait()?;
    if status.success() {
        Ok(())
    } else {
        Err(Error::Install(format!(
            "the digest command failed ({status})"
        )))
    }
}

// ---------------------------------------------------------------------------------------------
// The daily timer

const TASK: &str = "Sangraha digest";

/// "09:00" and the like.
pub fn valid_time(t: &str) -> bool {
    let Some((h, m)) = t.split_once(':') else {
        return false;
    };
    h.len() == 2
        && m.len() == 2
        && h.parse::<u8>().is_ok_and(|h| h < 24)
        && m.parse::<u8>().is_ok_and(|m| m < 60)
}

fn unit_dir() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| crate::config::home().join(".config"))
        .join("systemd/user")
}

/// The systemd user service and timer that run the digest at `time` each day.
pub fn units(cli: &Path, time: &str) -> (String, String) {
    let service = format!(
        "[Unit]\nDescription=Sangraha daily digest\n\n[Service]\nType=oneshot\nExecStart=\"{}\" digest --refresh --notify --hook --mark\n",
        cli.display()
    );
    let timer = format!(
        "[Unit]\nDescription=Sangraha daily digest\n\n[Timer]\nOnCalendar=*-*-* {time}:00\nPersistent=true\n\n[Install]\nWantedBy=timers.target\n"
    );
    (service, timer)
}

fn run(program: &str, args: &[&str]) -> Result<()> {
    let out = Command::new(program).args(args).output()?;
    if out.status.success() {
        Ok(())
    } else {
        Err(Error::Install(format!(
            "{program} {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        )))
    }
}

/// Whether the daily digest is set up on this computer.
pub fn scheduled() -> bool {
    if cfg!(windows) {
        Command::new("schtasks")
            .args(["/Query", "/TN", TASK])
            .output()
            .is_ok_and(|o| o.status.success())
    } else {
        unit_dir().join("sangraha-digest.timer").is_file()
    }
}

/// Sets up (or takes out) the daily digest at `time`, calling the command line program `cli`.
pub fn schedule(on: bool, time: &str, cli: &Path) -> Result<String> {
    if on && !valid_time(time) {
        return Err(Error::Install(format!(
            "{time}: write the time as HH:MM, e.g. 09:00"
        )));
    }
    if cfg!(windows) {
        if on {
            let tr = format!(
                "\"{}\" digest --refresh --notify --hook --mark",
                cli.display()
            );
            run(
                "schtasks",
                &[
                    "/Create", "/F", "/SC", "DAILY", "/ST", time, "/TN", TASK, "/TR", &tr,
                ],
            )?;
            return Ok(format!(
                "The digest runs every day at {time} (Task Scheduler)"
            ));
        }
        if scheduled() {
            run("schtasks", &["/Delete", "/F", "/TN", TASK])?;
        }
        return Ok("The daily digest is off".into());
    }
    if cfg!(target_os = "macos") {
        return Err(Error::Install(
            "The daily digest is not set up on macOS yet".into(),
        ));
    }
    let dir = unit_dir();
    let (service, timer) = (
        dir.join("sangraha-digest.service"),
        dir.join("sangraha-digest.timer"),
    );
    if on {
        let (s, t) = units(cli, time);
        std::fs::create_dir_all(&dir)?;
        std::fs::write(&service, s)?;
        std::fs::write(&timer, t)?;
        run("systemctl", &["--user", "daemon-reload"])?;
        run(
            "systemctl",
            &["--user", "enable", "--now", "sangraha-digest.timer"],
        )?;
        Ok(format!(
            "The digest runs every day at {time} (systemd user timer)"
        ))
    } else {
        if timer.is_file() {
            let _ = run(
                "systemctl",
                &["--user", "disable", "--now", "sangraha-digest.timer"],
            );
        }
        for f in [&service, &timer] {
            if f.is_file() {
                std::fs::remove_file(f)?;
            }
        }
        let _ = run("systemctl", &["--user", "daemon-reload"]);
        Ok("The daily digest is off".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::{Counts, Problem, ProblemKind};

    #[test]
    fn new_items_and_summary() {
        let mut row = crate::board::empty_row("me/app".into(), "app".into());
        row.problems.push(Problem {
            kind: ProblemKind::CiFailing,
            level: Level::High,
            text: "checks failing on main".into(),
            path: None,
        });
        let board = Board {
            rows: vec![row],
            counts: Counts {
                attention: 1,
                ..Default::default()
            },
            ..Default::default()
        };
        let first = build(Some(&board), None, None, None, None, &HashSet::new());
        assert_eq!(first.summary, "1 repos need attention");
        assert_eq!(first.new, 1);
        let seen: HashSet<String> = first.items.iter().map(|i| i.id.clone()).collect();
        let again = build(Some(&board), None, None, None, None, &seen);
        assert_eq!(again.new, 0, "the same problem is not new the next day");
        assert!(text(&again).contains("app: checks failing on main"));
        assert_eq!(
            build(None, None, None, None, None, &seen).summary,
            "Nothing needs you"
        );
    }

    #[test]
    fn timer_units() {
        assert!(valid_time("09:00") && valid_time("23:59"));
        assert!(!valid_time("9:00") && !valid_time("24:00") && !valid_time("noon"));
        let (s, t) = units(Path::new("/home/me/.cargo/bin/sangraha"), "07:30");
        assert!(s.contains(
            "ExecStart=\"/home/me/.cargo/bin/sangraha\" digest --refresh --notify --hook --mark"
        ));
        assert!(t.contains("OnCalendar=*-*-* 07:30:00") && t.contains("Persistent=true"));
    }
}
