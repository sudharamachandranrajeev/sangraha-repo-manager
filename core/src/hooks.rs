//! The `pre-push` hook that runs the leak check (and, for tags, the release gate) before git
//! sends anything to GitHub.
//!
//! A hook that was there before is kept as `pre-push.local` and still runs, after Sangraha's
//! check. Removing Sangraha's hook puts it back. A clone whose hooks live in a shared folder
//! (`core.hooksPath`) is left alone: a hook there would act on every clone that shares it.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::local::git;
use crate::{Error, Result};

const MARK: &str = "# Installed by Sangraha";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum HookState {
    /// No `pre-push` hook.
    None,
    /// Sangraha's hook.
    Sangraha,
    /// Some other `pre-push` hook.
    Other,
    /// Hooks come from a shared folder (`core.hooksPath`).
    Shared,
}

fn shared(repo: &Path) -> Option<String> {
    git(repo, &["config", "--get", "core.hooksPath"])
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
}

fn hook_file(repo: &Path) -> Result<PathBuf> {
    let out = git(repo, &["rev-parse", "--git-path", "hooks"]).map_err(Error::Git)?;
    let dir = PathBuf::from(out.trim());
    Ok(if dir.is_absolute() {
        dir
    } else {
        repo.join(dir)
    }
    .join("pre-push"))
}

pub fn state(repo: &Path) -> HookState {
    if shared(repo).is_some() {
        return HookState::Shared;
    }
    match hook_file(repo).and_then(|f| Ok(std::fs::read_to_string(f)?)) {
        Ok(text) if text.contains(MARK) => HookState::Sangraha,
        Ok(_) => HookState::Other,
        Err(_) => HookState::None,
    }
}

/// The `sangraha` command line program: this program if it is that, else one beside it, else
/// one on the `PATH`.
pub fn cli_path() -> Option<PathBuf> {
    let name = if cfg!(windows) {
        "sangraha.exe"
    } else {
        "sangraha"
    };
    let me = std::env::current_exe().ok();
    if let Some(me) = &me {
        if me.file_name().is_some_and(|n| n == name) {
            return Some(me.clone());
        }
        let beside = me.with_file_name(name);
        if beside.is_file() {
            return Some(beside);
        }
    }
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|d| d.join(name))
        .find(|p| p.is_file())
}

/// The hook's text, calling `cli` (an absolute path).
pub fn script(cli: &Path) -> String {
    // Git runs hooks with sh, also on Windows (Git for Windows' own), which wants `/`.
    let cli = cli
        .to_string_lossy()
        .replace('\\', "/")
        .replace('\'', r"'\''");
    format!(
        r#"#!/bin/sh
{MARK}: it checks what this push sends for leaks (private emails, home
# folder paths, private words, secrets) and, for a tag, runs the release gate.
# `sangraha hook remove` takes it out. A hook that was here before is kept as
# pre-push.local and runs after this check.
cli='{cli}'
if [ ! -x "$cli" ]; then
  echo "Sangraha's leak check is missing ($cli). Reinstall Sangraha, or run: sangraha hook remove" >&2
  exit 1
fi
input=$(cat)
printf '%s\n' "$input" | "$cli" check-push "$@" || exit 1
if [ -x "$0.local" ]; then
  printf '%s\n' "$input" | "$0.local" "$@" || exit 1
fi
exit 0
"#
    )
}

/// Installs (or updates) Sangraha's hook in one clone.
pub fn install(repo: &Path, cli: &Path) -> Result<String> {
    if let Some(dir) = shared(repo) {
        return Err(Error::Install(format!(
            "{} takes its hooks from {dir} (core.hooksPath), which other clones share. Add this line to the pre-push hook there instead: {} check-push \"$@\"",
            repo.display(),
            cli.display()
        )));
    }
    let file = hook_file(repo)?;
    let kept = file.with_file_name("pre-push.local");
    std::fs::create_dir_all(file.parent().unwrap_or(repo))?;
    let mut note = String::new();
    match std::fs::read_to_string(&file) {
        Ok(text) if text.contains(MARK) => {}
        Ok(_) => {
            if kept.exists() {
                return Err(Error::Install(format!(
                    "{} has a pre-push hook and a pre-push.local already; Sangraha will not move either",
                    repo.display()
                )));
            }
            std::fs::rename(&file, &kept)?;
            note = " (the hook that was there now runs after it, from pre-push.local)".into();
        }
        Err(_) => {}
    }
    std::fs::write(&file, script(cli))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(format!("Leak check installed in {}{note}", repo.display()))
}

/// Takes Sangraha's hook out of one clone and puts back the one it replaced, if any.
pub fn remove(repo: &Path) -> Result<String> {
    let file = hook_file(repo)?;
    match std::fs::read_to_string(&file) {
        Ok(text) if text.contains(MARK) => {}
        Ok(_) => {
            return Err(Error::Install(format!(
                "The pre-push hook in {} is not Sangraha's; left as it is",
                repo.display()
            )));
        }
        Err(_) => return Ok(format!("No leak check in {}", repo.display())),
    }
    std::fs::remove_file(&file)?;
    let kept = file.with_file_name("pre-push.local");
    if kept.exists() {
        std::fs::rename(&kept, &file)?;
        return Ok(format!(
            "Leak check removed from {}; its earlier hook is back",
            repo.display()
        ));
    }
    Ok(format!("Leak check removed from {}", repo.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn init(dir: &Path) {
        assert!(
            Command::new("git")
                .arg("-C")
                .arg(dir)
                .args(["init", "-q"])
                .status()
                .unwrap()
                .success()
        );
    }

    #[test]
    fn install_keeps_and_restores_an_earlier_hook() {
        let t = tempfile::tempdir().unwrap();
        let repo = t.path();
        init(repo);
        let hooks = repo.join(".git/hooks");
        let cli = Path::new("/opt/it's/sangraha");
        assert_eq!(state(repo), HookState::None);

        std::fs::create_dir_all(&hooks).unwrap();
        std::fs::write(hooks.join("pre-push"), "#!/bin/sh\nexit 0\n").unwrap();
        assert_eq!(state(repo), HookState::Other);
        let msg = install(repo, cli).unwrap();
        assert!(msg.contains("pre-push.local"), "{msg}");
        assert_eq!(state(repo), HookState::Sangraha);
        let text = std::fs::read_to_string(hooks.join("pre-push")).unwrap();
        assert!(text.contains(r"cli='/opt/it'\''s/sangraha'"), "{text}");
        assert_eq!(
            std::fs::read_to_string(hooks.join("pre-push.local")).unwrap(),
            "#!/bin/sh\nexit 0\n"
        );
        // Installing again only refreshes the hook.
        install(repo, cli).unwrap();
        assert!(hooks.join("pre-push.local").exists());

        remove(repo).unwrap();
        assert_eq!(state(repo), HookState::Other);
        assert!(!hooks.join("pre-push.local").exists());
        assert!(remove(repo).is_err(), "someone else's hook stays");
    }

    #[test]
    fn shared_hooks_are_left_alone() {
        let t = tempfile::tempdir().unwrap();
        init(t.path());
        Command::new("git")
            .arg("-C")
            .arg(t.path())
            .args(["config", "core.hooksPath", "/somewhere/shared"])
            .status()
            .unwrap();
        assert_eq!(state(t.path()), HookState::Shared);
        assert!(install(t.path(), Path::new("/x/sangraha")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn the_hook_runs_the_check_then_the_earlier_hook() {
        let t = tempfile::tempdir().unwrap();
        let repo = t.path().join("r");
        std::fs::create_dir_all(&repo).unwrap();
        init(&repo);
        let hooks = repo.join(".git/hooks");
        std::fs::create_dir_all(&hooks).unwrap();
        let log = t.path().join("log");
        // A stand-in for the command line and for an earlier hook: each notes what it got.
        let fake = t.path().join("sangraha");
        std::fs::write(
            &fake,
            format!(
                "#!/bin/sh\necho \"check $* $(cat)\" >> '{}'\n",
                log.display()
            ),
        )
        .unwrap();
        std::fs::write(
            hooks.join("pre-push"),
            format!(
                "#!/bin/sh\necho \"earlier $* $(cat)\" >> '{}'\n",
                log.display()
            ),
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        for f in [&fake, &hooks.join("pre-push")] {
            std::fs::set_permissions(f, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        install(&repo, &fake).unwrap();
        let mut child = Command::new(hooks.join("pre-push"))
            .args(["origin", "url"])
            .stdin(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        use std::io::Write;
        child
            .stdin
            .take()
            .unwrap()
            .write_all(b"refs/heads/main a refs/heads/main b\n")
            .unwrap();
        assert!(child.wait().unwrap().success());
        assert_eq!(
            std::fs::read_to_string(&log).unwrap(),
            "check check-push origin url refs/heads/main a refs/heads/main b\nearlier origin url refs/heads/main a refs/heads/main b\n"
        );
    }
}
