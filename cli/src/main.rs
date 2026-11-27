//! `sangraha`: the board on the command line, for scripts and quick looks.

use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use sangraha_core::{AppStatus, AppsBoard, Board, Config, Level, Row};

const HELP: &str = "\
sangraha: every GitHub repository you own and every clone on this computer, worst first

Usage:
  sangraha status [--all] [--json] [--offline] [--fetch] [--check]
  sangraha roots [PATH...]
  sangraha hide KEY... | sangraha unhide KEY...
  sangraha apps [--all] [--json] [--offline]
  sangraha install OWNER/NAME... [--unverified] | sangraha install --updates
  sangraha rollback OWNER/NAME
  sangraha follow OWNER/NAME... | sangraha unfollow OWNER/NAME...
  sangraha security [--json] [--offline]
  sangraha leaks [PATH]
  sangraha hook install|remove [PATH... | --all] | sangraha hook status
  sangraha gate [PATH]
  sangraha bulk --repos all|public|OWNER/NAME,... CHANGES... [--pr] [--apply]

status    What needs attention. Reads GitHub through your gh login (gh auth login).
  --all       every repository, not only those that need attention
  --json      the whole board as JSON
  --offline   use the last GitHub answer; only look at the clones
  --fetch     run git fetch in every clone first, so ahead/behind is current
  --check     exit with status 1 when something needs attention

roots     Show the folders searched for clones, or replace them with the PATHs given.
hide      Leave a repository (owner/name) or a clone folder off the board; unhide brings it back.

apps      Apps from GitHub releases: what is installed here and what has an update.
  --all       every app, not only those installed or with an update
install   Download, check the SHA-256 and install for this user (no admin password). The copy it
          replaces is kept for rollback. Refuses while the app is running.
  --unverified  install even when the release publishes no SHA-256
  --updates     install every available update
rollback  Go back to the copy kept from before the last update.
follow    Track another repository's releases in the Apps list; unfollow stops.

security  Open security alerts across your repositories, grouped by package, shipped
          dependencies first; and secret and code scanning alerts per repository.
leaks     Check the commits in PATH (default: here) that are not on GitHub yet for private
          emails, home folder paths, private words and secrets. Exit status 1 if any.
hook      Install the leak check as a pre-push hook in the clones named (or --all clones),
          so every push is checked first; pushing a tag also runs the release gate. A hook
          that was there before keeps running after it. remove takes it out.
gate      Should a release go out? No, while shipped dependencies have open high or
          critical alerts on GitHub, or npm audit finds some. Exit status 1 if not.

bulk      One change to many repositories. Shows what would change (a dry run) unless
          --apply is given. --repos all and public leave out archived repositories and forks.
  --set NAME=on|off   security-alerts, security-fixes, secret-scanning, push-protection,
                      delete-branch-on-merge, auto-merge, squash-merge, merge-commit,
                      rebase-merge
  --topic +NAME / --topic -NAME   add or remove a topic
  --dependabot        add .github/dependabot.yml, made from each repository's manifests
  --copy OWNER/NAME:PATH          copy a file from another repository (same path)
  --replace           replace files that exist and differ (otherwise they are left)
  --pr                files through a pull request, not a commit on the default branch
  --message TEXT      commit message and pull request title
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |f: &str| args.iter().any(|a| a == f);
    let result = match args.first().map(String::as_str) {
        Some("status") => status(
            flag("--all"),
            flag("--json"),
            flag("--offline"),
            flag("--fetch"),
            flag("--check"),
        ),
        Some("roots") => roots(&args[1..]),
        Some(cmd @ ("hide" | "unhide")) => hide(cmd == "hide", &args[1..]),
        Some("apps") => apps(flag("--all"), flag("--json"), flag("--offline")),
        Some("install") => install(&args[1..], flag("--unverified"), flag("--updates")),
        Some("rollback") => match args.get(1) {
            Some(k) => sangraha_core::install::rollback(k)
                .map(|m| {
                    println!("{m}");
                    ExitCode::SUCCESS
                })
                .map_err(|e| e.to_string()),
            None => Err("name the app: sangraha rollback owner/name".into()),
        },
        Some(cmd @ ("follow" | "unfollow")) => follow(cmd == "follow", &args[1..]),
        Some("security") => security(flag("--json"), flag("--offline")),
        Some("leaks") => leaks(args.get(1).map(PathBuf::from)),
        Some("check-push") => check_push(args.get(2).map(String::as_str).unwrap_or("")),
        Some("hook") => hook(args.get(1).map(String::as_str), &args[2..], flag("--all")),
        Some("gate") => gate(args.get(1).map(PathBuf::from)),
        Some("bulk") => bulk(&args[1..]),
        Some("-V" | "--version") => {
            println!("sangraha {}", env!("CARGO_PKG_VERSION"));
            Ok(ExitCode::SUCCESS)
        }
        None | Some("-h" | "--help" | "help") => {
            print!("{HELP}");
            Ok(ExitCode::SUCCESS)
        }
        Some(other) => Err(format!("unknown command '{other}'. Try: sangraha --help")),
    };
    result.unwrap_or_else(|e| {
        eprintln!("sangraha: {e}");
        ExitCode::from(2)
    })
}

fn status(
    all: bool,
    json: bool,
    offline: bool,
    fetch: bool,
    check: bool,
) -> Result<ExitCode, String> {
    let cfg = Config::load();
    let board = if offline {
        sangraha_core::load(&cfg)
    } else {
        sangraha_core::refresh(&cfg, fetch)
    }
    .map_err(|e| e.to_string())?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&board).map_err(|e| e.to_string())?
        );
    } else {
        print_board(&board, all);
    }
    Ok(if check && board.counts.attention > 0 {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    })
}

fn mark(level: Level) -> &'static str {
    match level {
        Level::High => "!!",
        Level::Medium => "! ",
        Level::Low => "· ",
        Level::Info => "  ",
    }
}

/// `println!` that stops quietly when the reader goes away (`sangraha status | head`).
macro_rules! out {
    ($($t:tt)*) => {
        if writeln!(std::io::stdout().lock(), $($t)*).is_err() {
            std::process::exit(0);
        }
    };
}

fn print_board(b: &Board, all: bool) {
    let c = &b.counts;
    out!(
        "{} repositories · {} need attention · {} unpushed · {} red checks · {} with alerts · {} not cloned",
        c.all,
        c.attention,
        c.unpushed,
        c.ci_red,
        c.alerts,
        c.not_cloned
    );
    out!("");
    let rows: Vec<&Row> = b
        .rows
        .iter()
        .filter(|r| all || r.needs_attention())
        .collect();
    if rows.is_empty() {
        out!("Nothing needs attention.");
    }
    let width = rows
        .iter()
        .map(|r| r.name.chars().count())
        .max()
        .unwrap_or(0)
        .min(36);
    for r in rows {
        let release = r.release.as_ref().map(|x| x.tag.as_str()).unwrap_or("");
        let worst = r.worst().map(|p| p.level).unwrap_or(Level::Info);
        let first = r.worst().map(|p| p.text.as_str()).unwrap_or("clean");
        out!(
            "{} {:width$}  {:14}  {}",
            mark(worst),
            r.name,
            release,
            first
        );
        for p in r.problems.iter().skip(1) {
            out!(
                "   {:width$}  {:14}  {} {}",
                "",
                "",
                mark(p.level).trim_end(),
                p.text
            );
        }
    }
    out!("");
    let checked = match b.checked_at {
        Some(t) => ago(sangraha_core::now() - t),
        None => "never".into(),
    };
    let budget = b
        .rate
        .as_ref()
        .map(|r| format!(" · GitHub budget {}/{}", r.remaining, r.limit))
        .unwrap_or_default();
    out!("Checked GitHub {checked}{budget} · {}", b.platform);
    for w in &b.warnings {
        eprintln!("note: {w}");
    }
}

fn ago(secs: i64) -> String {
    match secs {
        s if s < 60 => "just now".into(),
        s if s < 3600 => format!("{} min ago", s / 60),
        s if s < 86_400 => format!("{} h ago", s / 3600),
        s => format!("{} days ago", s / 86_400),
    }
}

fn roots(paths: &[String]) -> Result<ExitCode, String> {
    let mut cfg = Config::load();
    if !paths.is_empty() {
        let mut roots = Vec::new();
        for p in paths {
            let path = std::fs::canonicalize(PathBuf::from(p)).map_err(|e| format!("{p}: {e}"))?;
            if !path.is_dir() {
                return Err(format!("{p} is not a folder"));
            }
            roots.push(path);
        }
        cfg.roots = roots;
        cfg.save().map_err(|e| e.to_string())?;
    }
    for r in &cfg.roots {
        println!("{}", r.display());
    }
    Ok(ExitCode::SUCCESS)
}

fn hide(on: bool, keys: &[String]) -> Result<ExitCode, String> {
    if keys.is_empty() {
        return Err("name at least one owner/name or folder path".into());
    }
    let mut cfg = Config::load();
    for k in keys {
        cfg.hidden.retain(|h| h != k);
        if on {
            cfg.hidden.push(k.clone());
        }
    }
    cfg.save().map_err(|e| e.to_string())?;
    for h in &cfg.hidden {
        println!("hidden: {h}");
    }
    Ok(ExitCode::SUCCESS)
}

fn apps(all: bool, json: bool, offline: bool) -> Result<ExitCode, String> {
    let cfg = Config::load();
    let board = if offline {
        sangraha_core::apps_load(&cfg)
    } else {
        sangraha_core::apps_refresh(&cfg)
    }
    .map_err(|e| e.to_string())?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&board).map_err(|e| e.to_string())?
        );
    } else {
        print_apps(&board, all);
    }
    Ok(ExitCode::SUCCESS)
}

fn print_apps(b: &AppsBoard, all: bool) {
    let c = &b.counts;
    out!(
        "{} apps · {} installed · {} with an update · {} not installed",
        c.all,
        c.installed,
        c.updates,
        c.not_installed
    );
    out!("");
    let rows: Vec<_> = b
        .rows
        .iter()
        .filter(|r| !r.hidden && (all || r.here.is_some()))
        .collect();
    if rows.is_empty() {
        out!("No app from your releases is installed here. See them all: sangraha apps --all");
    }
    let width = rows
        .iter()
        .map(|r| r.name.chars().count())
        .max()
        .unwrap_or(0)
        .min(28);
    for r in rows {
        let here = r
            .here
            .as_ref()
            .map(|h| h.version.clone().unwrap_or_else(|| "?".into()))
            .unwrap_or_else(|| "-".into());
        let offer = r.offer.as_ref().map(|o| o.tag.as_str()).unwrap_or("-");
        let status = match r.status {
            AppStatus::Update => "update",
            AppStatus::Unknown => "installed, version unknown",
            AppStatus::Waiting => "waiting",
            AppStatus::UpToDate => "up to date",
            AppStatus::NotInstalled => "not installed",
            AppStatus::NoBuild => "no download for this computer",
        };
        out!(
            "{:width$}  {:>16} -> {:<16}  {status}  ({})",
            r.name,
            here,
            offer,
            r.key
        );
        for n in &r.notes {
            out!("{:width$}  · {n}", "");
        }
    }
    out!("");
    out!("New apps go to {}", b.apps_dir.display());
}

fn install(args: &[String], unverified: bool, updates: bool) -> Result<ExitCode, String> {
    let cfg = Config::load();
    let keys: Vec<String> = if updates {
        sangraha_core::apps_load(&cfg)
            .map_err(|e| e.to_string())?
            .rows
            .into_iter()
            .filter(|r| !r.hidden && r.status == AppStatus::Update)
            .map(|r| r.key)
            .collect()
    } else {
        args.iter()
            .filter(|a| !a.starts_with("--"))
            .cloned()
            .collect()
    };
    if keys.is_empty() {
        out!(
            "{}",
            if updates {
                "No updates."
            } else {
                "Name an app: sangraha install owner/name"
            }
        );
        return Ok(ExitCode::SUCCESS);
    }
    let mut failed = false;
    for k in keys {
        let shown = std::sync::atomic::AtomicU64::new(u64::MAX);
        let progress = |done: u64, total: u64| {
            let pct = (done * 100).checked_div(total).unwrap_or(0);
            if shown.swap(pct, std::sync::atomic::Ordering::Relaxed) != pct {
                eprint!("\r{k}: {pct:>3}%");
            }
        };
        match sangraha_core::install_app(&cfg, &k, unverified, &progress) {
            Ok(m) => {
                eprint!("\r");
                out!("{m}");
            }
            Err(e) => {
                eprintln!("\r{k}: {e}");
                if matches!(e, sangraha_core::Error::Unverified(_)) {
                    eprintln!("  To install it anyway: sangraha install {k} --unverified");
                }
                failed = true;
            }
        }
    }
    Ok(if failed {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    })
}

fn follow(on: bool, keys: &[String]) -> Result<ExitCode, String> {
    if keys.is_empty() {
        return Err("name at least one owner/name".into());
    }
    let mut cfg = Config::load();
    for k in keys {
        if !sangraha_core::apps::valid_name(k) {
            return Err(format!("{k} is not owner/name"));
        }
        cfg.extra_apps.retain(|e| !e.eq_ignore_ascii_case(k));
        if on {
            cfg.extra_apps.push(k.clone());
        }
    }
    cfg.save().map_err(|e| e.to_string())?;
    for e in &cfg.extra_apps {
        println!("following: {e}");
    }
    Ok(ExitCode::SUCCESS)
}

fn security(json: bool, offline: bool) -> Result<ExitCode, String> {
    let cfg = Config::load();
    let b = if offline {
        sangraha_core::security_load(&cfg)
    } else {
        sangraha_core::security_refresh(&cfg)
    }
    .map_err(|e| e.to_string())?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&b).map_err(|e| e.to_string())?
        );
        return Ok(ExitCode::SUCCESS);
    }
    let c = &b.counts;
    out!(
        "{} critical · {} high · {} moderate · {} low · {} in shipped dependencies · {} secrets · {} code scanning",
        c.critical,
        c.high,
        c.moderate,
        c.low,
        c.shipped,
        c.secrets,
        c.code
    );
    out!("");
    if b.groups.is_empty() {
        out!("No open security alerts.");
    }
    for g in &b.groups {
        let fix = match (&g.fix, g.unfixed) {
            (Some(f), 0) => format!("upgrade to {f} or later"),
            (Some(f), n) => format!("upgrade to {f} or later ({n} with no fix yet)"),
            (None, _) => "no fix yet".into(),
        };
        out!(
            "{:9} {} ({}){}: {} · {}",
            format!("{:?}", g.severity).to_lowercase(),
            g.package,
            g.ecosystem,
            if g.shipped { "" } else { " dev only" },
            g.repos
                .iter()
                .map(|r| short(r))
                .collect::<Vec<_>>()
                .join(", "),
            fix
        );
    }
    if !b.scans.is_empty() {
        out!("");
        for s in &b.scans {
            out!(
                "{}: {} secret{} · {} code scanning ({} high or critical)",
                short(&s.repo),
                s.secrets.unwrap_or(0),
                if s.secrets == Some(1) { "" } else { "s" },
                s.code.unwrap_or(0),
                s.code_serious
            );
        }
    }
    out!("");
    out!(
        "Secret scanning is off or not available in {} repositories; code scanning in {}.",
        c.secrets_off,
        c.code_off
    );
    let missing = b
        .hooks
        .iter()
        .filter(|h| h.state != sangraha_core::hooks::HookState::Sangraha)
        .count();
    out!("");
    out!(
        "Leak check before push: in {} of {} clones (sangraha hook install --all)",
        b.hooks.len() - missing,
        b.hooks.len()
    );
    for w in &b.warnings {
        eprintln!("note: {w}");
    }
    Ok(ExitCode::SUCCESS)
}

/// `name` from `owner/name`: the Security tab only lists the owner's own repositories.
fn short(full: &str) -> &str {
    full.split_once('/').map_or(full, |(_, n)| n)
}

fn print_leaks(r: &sangraha_core::leaks::Report) {
    use sangraha_core::leaks::LeakKind;
    for f in &r.findings {
        let what = match f.kind {
            LeakKind::Email => "private email",
            LeakKind::Secret => "secret",
            LeakKind::HomePath => "home folder path",
            LeakKind::Term => "private word",
            LeakKind::Unsigned => "unsigned commit",
        };
        let place = match (&f.file, f.line) {
            (Some(file), Some(l)) => format!("{file}:{l}"),
            (Some(file), None) => file.clone(),
            (None, _) if matches!(f.kind, LeakKind::Unsigned) => String::new(),
            (None, _) => "commit message or author".into(),
        };
        eprintln!("  {}  {what}: {}  {place}", f.commit, f.text);
    }
    for w in &r.warnings {
        eprintln!("note: {w}");
    }
}

fn leaks(path: Option<PathBuf>) -> Result<ExitCode, String> {
    let repo = path.unwrap_or_else(|| PathBuf::from("."));
    let cfg = Config::load();
    let login = sangraha_core::cache::Cache::open_default()
        .ok()
        .and_then(|c| c.snapshot().ok().flatten())
        .map(|s| s.login);
    let rules = sangraha_core::leaks::Rules::for_owner(&cfg, login, &repo);
    let revs: Vec<String> = ["--branches", "--tags", "--not", "--remotes"]
        .map(String::from)
        .to_vec();
    let r = sangraha_core::leaks::scan(&repo, &revs, &rules).map_err(|e| e.to_string())?;
    if r.findings.is_empty() {
        println!("{} unpushed commits checked: nothing found.", r.commits);
        print_leaks(&r);
        return Ok(ExitCode::SUCCESS);
    }
    eprintln!(
        "{} unpushed commits checked: {} problem{}:",
        r.commits,
        r.findings.len(),
        if r.findings.len() == 1 { "" } else { "s" }
    );
    print_leaks(&r);
    Ok(ExitCode::from(1))
}

/// Called by the pre-push hook, in the clone, with git's ref lines on standard input.
fn check_push(url: &str) -> Result<ExitCode, String> {
    let mut stdin = String::new();
    std::io::Read::read_to_string(&mut std::io::stdin(), &mut stdin).map_err(|e| e.to_string())?;
    let cfg = Config::load();
    let check = sangraha_core::check_push(&cfg, std::path::Path::new("."), &stdin, url)
        .map_err(|e| e.to_string())?;
    if !check.leaks.findings.is_empty() {
        eprintln!("Sangraha stopped this push. These would reach GitHub:");
    }
    print_leaks(&check.leaks);
    if let Some(g) = &check.gate {
        for r in &g.reasons {
            eprintln!("Release gate: {r}");
        }
        for w in &g.warnings {
            eprintln!("note: {w}");
        }
    }
    if check.blocked() {
        eprintln!(
            "Fix the commits (git commit --amend, or git rebase -i for older ones), then push again."
        );
        eprintln!("To push anyway: git push --no-verify");
        return Ok(ExitCode::from(1));
    }
    Ok(ExitCode::SUCCESS)
}

fn hook(action: Option<&str>, rest: &[String], all: bool) -> Result<ExitCode, String> {
    use sangraha_core::hooks;
    let cfg = Config::load();
    let paths: Vec<PathBuf> = if all {
        sangraha_core::local::find(&cfg.roots)
    } else {
        rest.iter()
            .filter(|a| !a.starts_with("--"))
            .map(|p| std::fs::canonicalize(p).unwrap_or_else(|_| PathBuf::from(p)))
            .collect()
    };
    match action {
        Some("status") => {
            for h in sangraha_core::security::hook_rows(&cfg.roots) {
                let state = match h.state {
                    hooks::HookState::Sangraha => "leak check",
                    hooks::HookState::Other => "other hook",
                    hooks::HookState::Shared => "shared hooks",
                    hooks::HookState::None => "none",
                };
                out!("{state:13} {}", h.path.display());
            }
            Ok(ExitCode::SUCCESS)
        }
        Some(a @ ("install" | "remove")) => {
            if paths.is_empty() {
                return Err(format!("name the clones, or --all: sangraha hook {a} PATH"));
            }
            let cli = hooks::cli_path().ok_or("cannot find the sangraha program itself")?;
            let mut failed = false;
            for p in paths {
                let r = if a == "install" {
                    hooks::install(&p, &cli)
                } else {
                    hooks::remove(&p)
                };
                match r {
                    Ok(m) => println!("{m}"),
                    Err(e) => {
                        failed = true;
                        eprintln!("{e}");
                    }
                }
            }
            Ok(if failed {
                ExitCode::from(1)
            } else {
                ExitCode::SUCCESS
            })
        }
        _ => Err(
            "try: sangraha hook install PATH, sangraha hook remove PATH, sangraha hook status"
                .into(),
        ),
    }
}

fn gate(path: Option<PathBuf>) -> Result<ExitCode, String> {
    let repo = path.unwrap_or_else(|| PathBuf::from("."));
    let url = std::process::Command::new("git")
        .arg("-C")
        .arg(&repo)
        .args(["config", "--get", "remote.origin.url"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    let full = sangraha_core::local::github_name(&url);
    let g = sangraha_core::security::gate(full.as_deref(), Some(&repo));
    for w in &g.warnings {
        eprintln!("note: {w}");
    }
    if g.blocked {
        for r in &g.reasons {
            eprintln!("Not yet: {r}");
        }
        return Ok(ExitCode::from(1));
    }
    println!(
        "Clear to release{}.",
        full.map(|f| format!(" {f}")).unwrap_or_default()
    );
    Ok(ExitCode::SUCCESS)
}

fn setting(name: &str) -> Option<sangraha_core::bulk::Setting> {
    use sangraha_core::bulk::Setting::*;
    Some(match name {
        "security-alerts" => SecurityAlerts,
        "security-fixes" => SecurityFixes,
        "secret-scanning" => SecretScanning,
        "push-protection" => PushProtection,
        "delete-branch-on-merge" => DeleteBranchOnMerge,
        "auto-merge" => AutoMerge,
        "squash-merge" => SquashMerge,
        "merge-commit" => MergeCommit,
        "rebase-merge" => RebaseMerge,
        _ => return None,
    })
}

fn bulk(args: &[String]) -> Result<ExitCode, String> {
    use sangraha_core::bulk::{FileChange, Request, Source};
    let mut req = Request::default();
    let mut apply = false;
    let mut replace = false;
    let mut repos = String::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut value = || it.next().cloned().ok_or(format!("{a} needs a value"));
        match a.as_str() {
            "--repos" => repos = value()?,
            "--set" => {
                let v = value()?;
                let (name, state) = v
                    .split_once('=')
                    .ok_or(format!("--set {v}: write NAME=on or NAME=off"))?;
                let s = setting(name).ok_or(format!(
                    "{name}: not a setting Sangraha knows (see sangraha --help)"
                ))?;
                let on = match state {
                    "on" => true,
                    "off" => false,
                    _ => return Err(format!("--set {v}: on or off")),
                };
                req.settings.insert(s, on);
            }
            "--topic" => {
                let v = value()?;
                match (v.strip_prefix('+'), v.strip_prefix('-')) {
                    (Some(t), _) => req.add_topics.push(t.to_owned()),
                    (_, Some(t)) => req.remove_topics.push(t.to_owned()),
                    _ => return Err(format!("--topic {v}: write +name or -name")),
                }
            }
            "--dependabot" => req.files.push(FileChange {
                path: ".github/dependabot.yml".into(),
                source: Source::Dependabot,
                replace: false,
            }),
            "--copy" => {
                let v = value()?;
                let (repo, path) = v
                    .split_once(':')
                    .ok_or(format!("--copy {v}: write OWNER/NAME:PATH"))?;
                req.files.push(FileChange {
                    path: path.to_owned(),
                    source: Source::Copy {
                        repo: repo.to_owned(),
                        path: path.to_owned(),
                    },
                    replace: false,
                });
            }
            "--replace" => replace = true,
            "--pr" => req.pull_request = true,
            "--message" => req.message = value()?,
            "--apply" => apply = true,
            other => return Err(format!("{other}: unknown option for bulk")),
        }
    }
    for f in &mut req.files {
        f.replace = replace;
    }
    let snap = sangraha_core::cache::Cache::open_default()
        .ok()
        .and_then(|c| c.snapshot().ok().flatten())
        .ok_or("run sangraha status once first, so Sangraha knows your repositories")?;
    req.repos =
        match repos.as_str() {
            "" => return Err(
                "say which repositories: --repos all, --repos public or --repos owner/a,owner/b"
                    .into(),
            ),
            "all" | "public" => snap
                .repos
                .iter()
                .filter(|r| !r.archived && !r.fork && (repos == "all" || !r.private))
                .map(|r| r.full_name.clone())
                .collect(),
            list => list
                .split(',')
                .map(|r| r.trim().to_owned())
                .filter(|r| !r.is_empty())
                .collect(),
        };
    let cfg = Config::load();
    let plan = if apply {
        sangraha_core::bulk_apply(&cfg, &req, &|repo| eprintln!("{repo}…"))
    } else {
        sangraha_core::bulk_plan(&cfg, &req)
    }
    .map_err(|e| e.to_string())?;
    for r in &plan.repos {
        out!("{}", r.repo);
        if let Some(e) = &r.error {
            out!("  ✗ {e}");
        }
        for s in &r.steps {
            match &s.note {
                None => out!(
                    "  {} {}: {} → {}",
                    if apply { "✓" } else { "•" },
                    s.label,
                    s.now,
                    s.then
                ),
                Some(n) => out!("    {}: {n}", s.label),
            }
        }
        if let Some(pr) = &r.pull_request {
            out!("  pull request: {pr}");
        }
    }
    out!("");
    if apply {
        let failed = plan.repos.iter().filter(|r| r.error.is_some()).count();
        out!(
            "{} changes made; {failed} repositories failed.",
            plan.changes
        );
        Ok(if failed > 0 {
            ExitCode::from(1)
        } else {
            ExitCode::SUCCESS
        })
    } else {
        out!(
            "Dry run: {} changes in {} repositories. Files would be committed as {}. Add --apply to make them.",
            plan.changes,
            plan.repos
                .iter()
                .filter(|r| r.steps.iter().any(|s| s.change))
                .count(),
            plan.author
        );
        Ok(ExitCode::SUCCESS)
    }
}
