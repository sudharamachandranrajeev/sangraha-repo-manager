//! The Sangraha window: the board from `sangraha-core`, plus opening links, folders and terminals.

use std::path::{Path, PathBuf};
use std::process::Command;

use sangraha_core::{Board, Config};

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
            version
        ])
        .run(tauri::generate_context!())
        .expect("Sangraha could not start");
}
