//! `sangraha`: the board on the command line, for scripts and quick looks.

use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use sangraha_core::{Board, Config, Level, Row};

const HELP: &str = "\
sangraha: every GitHub repository you own and every clone on this computer, worst first

Usage:
  sangraha status [--all] [--json] [--offline] [--fetch] [--check]
  sangraha roots [PATH...]
  sangraha hide KEY... | sangraha unhide KEY...

status    What needs attention. Reads GitHub through your gh login (gh auth login).
  --all       every repository, not only those that need attention
  --json      the whole board as JSON
  --offline   use the last GitHub answer; only look at the clones
  --fetch     run git fetch in every clone first, so ahead/behind is current
  --check     exit with status 1 when something needs attention

roots     Show the folders searched for clones, or replace them with the PATHs given.
hide      Leave a repository (owner/name) or a clone folder off the board; unhide brings it back.
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
