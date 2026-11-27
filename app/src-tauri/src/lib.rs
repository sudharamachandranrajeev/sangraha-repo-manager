//! The Sangraha window: the boards from `sangraha-core`, plus opening links, folders and
//! terminals, and installing apps.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use sangraha_core::{AppsBoard, Board, Config, SecurityBoard};
use serde::Serialize;
use tauri::Emitter;

/// Runs blocking work (git, GitHub, SQLite) off the window's thread.
async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| e.to_string())?
}

/// The board from the last GitHub answer and a fresh look at the clones. No network.
#[tauri::command]
async fn board() -> Result<Board, String> {
    blocking(|| sangraha_core::load(&Config::load()).map_err(|e| e.to_string())).await
}

/// Reads GitHub again; with `fetch`, runs `git fetch` in every clone first.
#[tauri::command]
async fn refresh(fetch: bool) -> Result<Board, String> {
    blocking(move || sangraha_core::refresh(&Config::load(), fetch).map_err(|e| e.to_string()))
        .await
}

#[tauri::command]
fn settings() -> Config {
    Config::load()
}

#[tauri::command]
fn set_roots(roots: Vec<String>) -> Result<Config, String> {
    let mut cfg = Config::load();
    let mut out = Vec::new();
    for r in roots.iter().map(|r| r.trim()).filter(|r| !r.is_empty()) {
        let p = PathBuf::from(r);
        if !p.is_dir() {
            return Err(format!("{r} is not a folder"));
        }
        out.push(p);
    }
    if out.is_empty() {
        return Err("Choose at least one folder".into());
    }
    cfg.roots = out;
    cfg.save().map_err(|e| e.to_string())?;
    Ok(cfg)
}

#[tauri::command]
fn set_hidden(key: String, hidden: bool) -> Result<Config, String> {
    let mut cfg = Config::load();
    cfg.hidden.retain(|k| k != &key);
    if hidden {
        cfg.hidden.push(key);
    }
    cfg.save().map_err(|e| e.to_string())?;
    Ok(cfg)
}

/// Opens a GitHub page in the browser. Only https links to github.com are opened.
#[tauri::command]
fn open_url(url: String) -> Result<(), String> {
    if !url.starts_with("https://github.com/") {
        return Err("Only GitHub links are opened".into());
    }
    spawn(
        if cfg!(windows) {
            "explorer"
        } else {
            "xdg-open"
        },
        &[url.as_str()],
    )
}

fn existing_dir(path: &str) -> Result<&Path, String> {
    let p = Path::new(path);
    if p.is_dir() {
        Ok(p)
    } else {
        Err(format!("{path} is not a folder"))
    }
}

#[tauri::command]
fn open_folder(path: String) -> Result<(), String> {
    let p = existing_dir(&path)?;
    spawn(
        if cfg!(windows) {
            "explorer"
        } else {
            "xdg-open"
        },
        &[&p.to_string_lossy()],
    )
}

/// Opens a terminal in the clone's folder.
#[tauri::command]
fn open_terminal(path: String) -> Result<(), String> {
    let p = existing_dir(&path)?;
    if cfg!(windows) {
        let mut wt = Command::new("wt");
        wt.arg("-d").arg(p);
        if wt.spawn().is_ok() {
            return Ok(());
        }
        return Command::new("cmd")
            .args(["/C", "start", "cmd", "/K", "cd", "/d"])
            .arg(p)
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("cmd: {e}"));
    }
    // xdg-terminal-exec opens the person's own terminal; the folder is its working directory.
    let mut cmd = Command::new("xdg-terminal-exec");
    cmd.current_dir(p);
    if cmd.spawn().is_ok() {
        return Ok(());
    }
    let term = std::env::var("TERMINAL").unwrap_or_else(|_| "x-terminal-emulator".into());
    Command::new(&term)
        .current_dir(p)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("{term}: {e}"))
}

fn spawn(program: &str, args: &[&str]) -> Result<(), String> {
    Command::new(program)
        .args(args)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("{program}: {e}"))
}

/// The Apps tab from the last GitHub answer and what is installed here. No network.
#[tauri::command]
async fn apps_board() -> Result<AppsBoard, String> {
    blocking(|| sangraha_core::apps_load(&Config::load()).map_err(|e| e.to_string())).await
}

#[tauri::command]
async fn apps_refresh() -> Result<AppsBoard, String> {
    blocking(|| sangraha_core::apps_refresh(&Config::load()).map_err(|e| e.to_string())).await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Failure {
    /// The release has no SHA-256 for the file: the window asks before trying again.
    unverified: bool,
    message: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Progress {
    key: String,
    done: u64,
    total: u64,
}

/// Downloads, checks and installs one app; sends `install-progress` events while downloading.
#[tauri::command]
async fn install_app(
    app: tauri::AppHandle,
    key: String,
    allow_unverified: bool,
) -> Result<String, Failure> {
    tauri::async_runtime::spawn_blocking(move || {
        let shown = AtomicU64::new(u64::MAX);
        let progress = |done: u64, total: u64| {
            let pct = (done * 100).checked_div(total).unwrap_or(0);
            if shown.swap(pct, Ordering::Relaxed) != pct {
                let _ = app.emit(
                    "install-progress",
                    Progress {
                        key: key.clone(),
                        done,
                        total,
                    },
                );
            }
        };
        sangraha_core::install_app(&Config::load(), &key, allow_unverified, &progress).map_err(
            |e| Failure {
                unverified: matches!(e, sangraha_core::Error::Unverified(_)),
                message: e.to_string(),
            },
        )
    })
    .await
    .map_err(|e| Failure {
        unverified: false,
        message: e.to_string(),
    })?
}

#[tauri::command]
async fn rollback_app(key: String) -> Result<String, String> {
    blocking(move || sangraha_core::install::rollback(&key).map_err(|e| e.to_string())).await
}

/// Starts an installed app. Only a launcher the Apps tab itself found is started.
#[tauri::command]
async fn launch_app(key: String) -> Result<(), String> {
    blocking(move || {
        let board = sangraha_core::apps_load(&Config::load()).map_err(|e| e.to_string())?;
        let launcher = board
            .rows
            .iter()
            .find(|r| r.key == key)
            .and_then(|r| r.here.as_ref())
            .and_then(|h| h.launcher.clone())
            .ok_or("Sangraha does not know how to start this app")?;
        let mut cmd = Command::new(&launcher);
        if let Some(dir) = launcher.parent() {
            cmd.current_dir(dir);
        }
        cmd.spawn()
            .map(|_| ())
            .map_err(|e| format!("{}: {e}", launcher.display()))
    })
    .await
}

#[tauri::command]
fn set_app_hidden(key: String, hidden: bool) -> Result<Config, String> {
    let mut cfg = Config::load();
    cfg.hidden_apps.retain(|k| !k.eq_ignore_ascii_case(&key));
    if hidden {
        cfg.hidden_apps.push(key);
    }
    cfg.save().map_err(|e| e.to_string())?;
    Ok(cfg)
}

/// The Apps settings, checked: the folder must exist or be creatable, names must be owner/name,
/// rules are `owner/name = pattern` lines.
#[tauri::command]
fn save_apps_settings(
    apps_dir: String,
    prereleases: bool,
    delay_days: u32,
    extra_apps: Vec<String>,
    rules: Vec<String>,
) -> Result<Config, String> {
    let mut cfg = Config::load();
    let dir = apps_dir.trim();
    cfg.apps_dir = if dir.is_empty() {
        None
    } else {
        let p = PathBuf::from(dir);
        if !p.is_absolute() {
            return Err(format!("{dir}: use a full path"));
        }
        std::fs::create_dir_all(&p).map_err(|e| format!("{dir}: {e}"))?;
        Some(p)
    };
    cfg.prereleases = prereleases;
    cfg.delay_days = delay_days.min(365);
    let mut extra = Vec::new();
    for e in extra_apps
        .iter()
        .map(|e| e.trim())
        .filter(|e| !e.is_empty())
    {
        let e = e
            .trim_start_matches("https://github.com/")
            .trim_end_matches('/');
        if !sangraha_core::apps::valid_name(e) {
            return Err(format!("{e} is not owner/name"));
        }
        extra.push(e.to_owned());
    }
    cfg.extra_apps = extra;
    cfg.asset_rules.clear();
    for line in rules.iter().map(|l| l.trim()).filter(|l| !l.is_empty()) {
        let (k, v) = line
            .split_once('=')
            .ok_or_else(|| format!("{line}: write owner/name = pattern"))?;
        let (k, v) = (k.trim(), v.trim());
        if !sangraha_core::apps::valid_name(k) || v.is_empty() {
            return Err(format!("{line}: write owner/name = pattern"));
        }
        cfg.asset_rules.insert(k.to_owned(), v.to_owned());
    }
    cfg.save().map_err(|e| e.to_string())?;
    Ok(cfg)
}

/// The Security tab from the last GitHub answer, with each clone's hook. No network.
#[tauri::command]
async fn security_board() -> Result<SecurityBoard, String> {
    blocking(|| sangraha_core::security_load(&Config::load()).map_err(|e| e.to_string())).await
}

#[tauri::command]
async fn security_refresh() -> Result<SecurityBoard, String> {
    blocking(|| sangraha_core::security_refresh(&Config::load()).map_err(|e| e.to_string())).await
}

/// Installs or removes the leak check hook in one clone. Only clones Sangraha found under the
/// configured folders are touched.
#[tauri::command]
async fn set_hook(path: String, on: bool) -> Result<String, String> {
    blocking(move || {
        let path = PathBuf::from(path);
        if !sangraha_core::local::find(&Config::load().roots).contains(&path) {
            return Err(format!("{} is not one of the clones Sangraha lists", path.display()));
        }
        let r = if on {
            let cli = sangraha_core::hooks::cli_path().ok_or(
                "The sangraha command line program was not found. Install it first (see the README)",
            )?;
            sangraha_core::hooks::install(&path, &cli)
        } else {
            sangraha_core::hooks::remove(&path)
        };
        r.map_err(|e| e.to_string())
    })
    .await
}

#[tauri::command]
fn save_leak_settings(
    terms_file: String,
    allowed_emails: Vec<String>,
    require_signed: bool,
) -> Result<Config, String> {
    let mut cfg = Config::load();
    let f = terms_file.trim();
    cfg.terms_file = if f.is_empty() {
        None
    } else {
        let p = PathBuf::from(f);
        if !p.is_absolute() || !p.is_file() {
            return Err(format!("{f}: not a file (use a full path)"));
        }
        Some(p)
    };
    let mut emails = Vec::new();
    for e in allowed_emails
        .iter()
        .map(|e| e.trim())
        .filter(|e| !e.is_empty())
    {
        if !e.contains('@') || e.contains(char::is_whitespace) {
            return Err(format!("{e} is not an email address"));
        }
        emails.push(e.to_lowercase());
    }
    cfg.allowed_emails = emails;
    cfg.require_signed = require_signed;
    cfg.save().map_err(|e| e.to_string())?;
    Ok(cfg)
}

#[tauri::command]
fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            board,
            refresh,
            settings,
            set_roots,
            set_hidden,
            open_url,
            open_folder,
            open_terminal,
            apps_board,
            apps_refresh,
            install_app,
            rollback_app,
            launch_app,
            set_app_hidden,
            save_apps_settings,
            security_board,
            security_refresh,
            set_hook,
            save_leak_settings,
            version
        ])
        .run(tauri::generate_context!())
        .expect("Sangraha could not start");
}
