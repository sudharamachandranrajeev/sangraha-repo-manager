//! Where Sangraha looks for clones, and where it keeps its own files.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    /// Folders searched for git clones (three levels deep).
    pub roots: Vec<PathBuf>,
    /// Row keys (`owner/name` or a folder path) the owner chose to hide from the board.
    #[serde(default)]
    pub hidden: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        let home = home();
        let work = home.join("Work");
        let roots = if work.is_dir() {
            vec![work]
        } else {
            vec![home]
        };
        Config {
            roots,
            hidden: Vec::new(),
        }
    }
}

impl Config {
    pub fn load() -> Config {
        Self::load_from(&config_dir().join("config.json"))
    }

    pub fn load_from(path: &Path) -> Config {
        std::fs::read(path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> crate::Result<()> {
        let dir = config_dir();
        std::fs::create_dir_all(&dir)?;
        let tmp = dir.join("config.json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        std::fs::rename(tmp, dir.join("config.json"))?;
        Ok(())
    }
}

pub fn home() -> PathBuf {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// `$XDG_CONFIG_HOME/sangraha`, `~/.config/sangraha`, or `%APPDATA%\Sangraha`.
pub fn config_dir() -> PathBuf {
    if cfg!(windows) {
        return std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(home)
            .join("Sangraha");
    }
    xdg("XDG_CONFIG_HOME", ".config").join("sangraha")
}

/// `$XDG_DATA_HOME/sangraha`, `~/.local/share/sangraha`, or `%LOCALAPPDATA%\Sangraha`.
pub fn data_dir() -> PathBuf {
    if cfg!(windows) {
        return std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(home)
            .join("Sangraha");
    }
    xdg("XDG_DATA_HOME", ".local/share").join("sangraha")
}

fn xdg(var: &str, fallback: &str) -> PathBuf {
    std::env::var_os(var)
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home().join(fallback))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_or_broken_file_gives_defaults() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            Config::load_from(&dir.path().join("nope.json")),
            Config::default()
        );
        std::fs::write(dir.path().join("bad.json"), b"{not json").unwrap();
        assert_eq!(
            Config::load_from(&dir.path().join("bad.json")),
            Config::default()
        );
    }
}
