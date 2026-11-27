//! Where Sangraha looks for clones, and where it keeps its own files.

use std::collections::BTreeMap;
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
    /// Apps tab: where new apps are installed. `None` means `~/Applications` on Linux and
    /// `%LOCALAPPDATA%\Programs` on Windows.
    #[serde(default)]
    pub apps_dir: Option<PathBuf>,
    /// Apps tab: other people's repositories (`owner/name`) whose releases to track.
    #[serde(default)]
    pub extra_apps: Vec<String>,
    /// Apps tab: apps (`owner/name`) hidden from the list.
    #[serde(default)]
    pub hidden_apps: Vec<String>,
    /// Apps tab: offer pre-releases (most of the owner's apps only have pre-releases).
    #[serde(default = "yes")]
    pub prereleases: bool,
    /// Apps tab: a release becomes an update only once it is this many days old.
    #[serde(default)]
    pub delay_days: u32,
    /// Apps tab: per app (`owner/name`), a pattern the download's file name must match, with `*`
    /// for any text. For apps whose file names Sangraha cannot read.
    #[serde(default)]
    pub asset_rules: BTreeMap<String, String>,
}

fn yes() -> bool {
    true
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
            apps_dir: None,
            extra_apps: Vec::new(),
            hidden_apps: Vec::new(),
            prereleases: true,
            delay_days: 0,
            asset_rules: BTreeMap::new(),
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

impl Config {
    /// Where new apps go.
    pub fn apps_dir(&self) -> PathBuf {
        self.apps_dir.clone().unwrap_or_else(|| {
            if cfg!(windows) {
                std::env::var_os("LOCALAPPDATA")
                    .map(PathBuf::from)
                    .unwrap_or_else(home)
                    .join("Programs")
            } else {
                home().join("Applications")
            }
        })
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
        // A file from an older version (no Apps settings) still loads, with pre-releases on.
        std::fs::write(dir.path().join("old.json"), br#"{"roots":["/x"]}"#).unwrap();
        let old = Config::load_from(&dir.path().join("old.json"));
        assert_eq!(old.roots, vec![PathBuf::from("/x")]);
        assert!(old.prereleases && old.extra_apps.is_empty());
        assert_eq!(
            Config::load_from(&dir.path().join("bad.json")),
            Config::default()
        );
    }
}
