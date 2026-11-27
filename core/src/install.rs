//! Installing an app for this user only, with no admin password.
//!
//! The steps: download the release file, check its SHA-256 against GitHub's, unpack it beside
//! the installed copy, then swap the two folders. The old copy stays as `<folder>.previous` for
//! "roll back". Nothing is replaced while the app is running. A copy installed some other way
//! (found in `~/.local/lib`, say) is updated where it is, so there are never two.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::apps::{self, Here, Kind};
use crate::config::{Config, data_dir, home};
use crate::{Error, Result};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Previous {
    pub tag: String,
    pub path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Installed {
    pub repo: String,
    pub tag: String,
    pub asset: String,
    pub kind: Kind,
    /// The app's folder (or the AppImage itself).
    pub path: PathBuf,
    pub launcher: Option<PathBuf>,
    pub sha256: String,
    pub installed_at: i64,
    pub previous: Option<Previous>,
    /// The menu entry Sangraha wrote, if it wrote one.
    pub menu_entry: Option<PathBuf>,
}

/// What Sangraha installed, in `apps.json` beside the cache.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Registry {
    apps: BTreeMap<String, Installed>,
}

impl Registry {
    pub fn path() -> PathBuf {
        data_dir().join("apps.json")
    }

    pub fn load() -> Registry {
        Self::load_from(&Self::path())
    }

    pub fn load_from(path: &Path) -> Registry {
        std::fs::read(path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> Result<()> {
        self.save_to(&Self::path())
    }

    pub fn save_to(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        std::fs::rename(tmp, path)?;
        Ok(())
    }

    pub fn get(&self, key: &str) -> Option<&Installed> {
        self.apps.get(&key.to_ascii_lowercase())
    }

    pub fn set(&mut self, key: &str, i: Installed) {
        self.apps.insert(key.to_ascii_lowercase(), i);
    }
}

// ---------------------------------------------------------------------------------------------
// Copies installed some other way

fn plain(s: &str) -> String {
    s.chars()
        .filter(char::is_ascii_alphanumeric)
        .collect::<String>()
        .to_ascii_lowercase()
}

/// Names a hand-made install might use: "graha-anveshana", then "anveshana".
fn ids(name: &str) -> Vec<String> {
    let id = apps::app_id(name);
    let mut out = vec![id.clone()];
    if let Some(last) = id.rsplit('-').next() {
        out.push(last.to_owned());
    }
    out.dedup();
    out.retain(|i| !i.is_empty());
    out
}

/// The version an install reports about itself: Flutter's `version.json`, or the number in an
/// AppImage's file name.
fn version_of(path: &Path) -> Option<String> {
    if path.is_dir() {
        let raw = std::fs::read(path.join("data/flutter_assets/version.json")).ok()?;
        let v: serde_json::Value = serde_json::from_slice(&raw).ok()?;
        return v["version"].as_str().map(str::to_owned);
    }
    let name = path.file_name()?.to_string_lossy().into_owned();
    name.split(['_', '-'])
        .find(|p| p.starts_with(|c: char| c.is_ascii_digit()) && apps::compare(p, p).is_some())
        .map(str::to_owned)
}

/// A copy of `key` installed without Sangraha, if one is where installs usually go.
pub fn detect(cfg: &Config, name: &str) -> Option<Here> {
    let apps_dir = cfg.apps_dir();
    let mut places: Vec<PathBuf> = vec![apps_dir.join(name)];
    for id in ids(name) {
        if cfg!(windows) {
            let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
            places.push(apps_dir.join(&id));
            if let Some(l) = &local {
                places.push(l.join(name));
            }
        } else {
            let h = home();
            places.push(apps_dir.join(&id));
            places.push(h.join(".local/lib").join(&id));
            places.push(h.join(".local/opt").join(&id));
            // A launcher on the PATH: follow it to the folder or AppImage it runs.
            if let Ok(real) = std::fs::canonicalize(h.join(".local/bin").join(&id)) {
                let lib = h.join(".local/lib");
                let owner = real
                    .ancestors()
                    .find(|a| a.parent() == Some(lib.as_path()))
                    .map(Path::to_path_buf);
                places.push(owner.unwrap_or(real));
            }
        }
    }
    if !cfg!(windows) {
        places.push(apps_dir.join(format!("{name}.AppImage")));
    }
    let path = places.into_iter().find(|p| {
        if p.is_dir() {
            find_launcher(p, name).is_some()
        } else {
            p.is_file()
        }
    })?;
    let launcher = if path.is_dir() {
        find_launcher(&path, name)
    } else {
        Some(path.clone())
    };
    Some(Here {
        version: version_of(&path),
        path,
        launcher,
        by_sangraha: false,
        previous: None,
    })
}

#[cfg(unix)]
fn executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(windows)]
fn executable(p: &Path) -> bool {
    p.extension().is_some_and(|e| e.eq_ignore_ascii_case("exe")) && p.is_file()
}

/// The program to start inside an app folder: the one named like the app, else the only one.
pub fn find_launcher(dir: &Path, name: &str) -> Option<PathBuf> {
    let want = plain(name);
    for d in [dir.to_path_buf(), dir.join("bin")] {
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue;
        };
        let mut found: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| executable(p))
            .filter(|p| {
                let n = p
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_ascii_lowercase();
                !n.ends_with(".so")
                    && !n.contains(".so.")
                    && !n.starts_with("unins")
                    && !n.starts_with("uninstall")
            })
            .collect();
        found.sort();
        let stem = |p: &PathBuf| plain(&p.file_stem().unwrap_or_default().to_string_lossy());
        if let Some(p) = found.iter().find(|p| stem(p) == want) {
            return Some(p.clone());
        }
        if let Some(p) = found
            .iter()
            .find(|p| want.contains(&stem(p)) || stem(p).contains(&want))
        {
            return Some(p.clone());
        }
        if found.len() == 1 {
            return found.pop();
        }
    }
    None
}

// ---------------------------------------------------------------------------------------------
// Running apps

/// True when a running program lives at `path` (a folder or a single file).
pub fn running(path: &Path) -> bool {
    if cfg!(windows) {
        let p = path.to_string_lossy().replace('\'', "''");
        let script = format!(
            "@(Get-Process | Where-Object {{ $_.Path -and $_.Path.StartsWith('{p}', 'OrdinalIgnoreCase') }}).Count"
        );
        return Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .output()
            .ok()
            .and_then(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .trim()
                    .parse::<u32>()
                    .ok()
            })
            .is_some_and(|n| n > 0);
    }
    let Ok(procs) = std::fs::read_dir("/proc") else {
        return false;
    };
    procs.flatten().any(|e| {
        e.file_name()
            .to_string_lossy()
            .bytes()
            .all(|b| b.is_ascii_digit())
            && std::fs::read_link(e.path().join("exe")).is_ok_and(|exe| {
                let exe = PathBuf::from(exe.to_string_lossy().trim_end_matches(" (deleted)"));
                exe.starts_with(path)
            })
    })
}

// ---------------------------------------------------------------------------------------------
// Download and check

struct RestAsset {
    name: String,
    api_url: String,
    digest: Option<String>,
}

fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn download_agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(30)))
        .timeout_recv_response(Some(Duration::from_secs(60)))
        .timeout_recv_body(Some(Duration::from_secs(60 * 30)))
        .user_agent("sangraha")
        .build()
        .into()
}

fn http_err(e: ureq::Error) -> Error {
    match e {
        ureq::Error::StatusCode(401) => {
            Error::Auth("GitHub refused the login. Run: gh auth login".into())
        }
        ureq::Error::StatusCode(404) => Error::Install("GitHub no longer has that release".into()),
        e => Error::GitHub(e.to_string()),
    }
}

/// The release's files as the REST API lists them: their API address works for private
/// repositories too.
fn release_assets(
    agent: &ureq::Agent,
    token: &str,
    repo: &str,
    tag: &str,
) -> Result<Vec<RestAsset>> {
    let url = format!(
        "{}/repos/{repo}/releases/tags/{}",
        crate::github::API,
        encode(tag)
    );
    let v: serde_json::Value = agent
        .get(&url)
        .header("Authorization", &format!("Bearer {token}"))
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(http_err)?
        .body_mut()
        .read_json()
        .map_err(|e| Error::GitHub(e.to_string()))?;
    Ok(v["assets"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|a| RestAsset {
            name: a["name"].as_str().unwrap_or_default().to_owned(),
            api_url: a["url"].as_str().unwrap_or_default().to_owned(),
            digest: a["digest"].as_str().map(str::to_owned),
        })
        .collect())
}

/// Streams a release file to `dest` and returns its SHA-256. The token is sent to GitHub only:
/// the storage server it redirects to gets a signed link instead.
fn download(
    agent: &ureq::Agent,
    token: &str,
    api_url: &str,
    dest: &Path,
    total: u64,
    progress: &(dyn Fn(u64, u64) + Sync),
) -> Result<String> {
    if !api_url.starts_with(crate::github::API) {
        return Err(Error::Install("unexpected download address".into()));
    }
    let res = agent
        .get(api_url)
        .header("Authorization", &format!("Bearer {token}"))
        .header("Accept", "application/octet-stream")
        .call()
        .map_err(http_err)?;
    let mut reader = res.into_body().into_reader();
    let mut file = std::fs::File::create(dest)?;
    let mut hash = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    let mut done = 0u64;
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hash.update(&buf[..n]);
        file.write_all(&buf[..n])?;
        done += n as u64;
        progress(done, total);
    }
    file.sync_all()?;
    Ok(hex(&hash.finalize()))
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

pub fn sha256_file(p: &Path) -> Result<String> {
    let mut f = std::fs::File::open(p)?;
    let mut hash = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hash.update(&buf[..n]);
    }
    Ok(hex(&hash.finalize()))
}

/// The SHA-256 the publisher gave: GitHub's own digest, else a checksum file in the release.
fn expected_sum(
    agent: &ureq::Agent,
    token: &str,
    assets: &[RestAsset],
    asset: &RestAsset,
) -> Result<Option<String>> {
    if let Some(d) = asset
        .digest
        .as_deref()
        .and_then(|d| d.strip_prefix("sha256:"))
    {
        return Ok(Some(d.to_ascii_lowercase()));
    }
    for sums in assets
        .iter()
        .filter(|a| apps::is_checksum_file(&a.name, &asset.name))
    {
        let text = agent
            .get(&sums.api_url)
            .header("Authorization", &format!("Bearer {token}"))
            .header("Accept", "application/octet-stream")
            .call()
            .map_err(http_err)?
            .body_mut()
            .read_to_string()
            .map_err(|e| Error::GitHub(e.to_string()))?;
        if let Some(s) = apps::sum_for(&text, &asset.name) {
            return Ok(Some(s));
        }
    }
    Ok(None)
}

// ---------------------------------------------------------------------------------------------
// Putting files in place

fn remove(p: &Path) -> std::io::Result<()> {
    match std::fs::symlink_metadata(p) {
        Ok(m) if m.is_dir() => std::fs::remove_dir_all(p),
        Ok(_) => std::fs::remove_file(p),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

fn with_suffix(p: &Path, suffix: &str) -> PathBuf {
    let mut s = p.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}

/// Moves `new` to `target`. The copy that was there becomes `<target>.previous`, replacing an
/// older one. If the last move fails, the old copy goes back.
pub fn swap_in(new: &Path, target: &Path) -> Result<Option<PathBuf>> {
    let prev = with_suffix(target, ".previous");
    let had = std::fs::symlink_metadata(target).is_ok();
    if had {
        remove(&prev)?;
        std::fs::rename(target, &prev)?;
    }
    if let Err(e) = std::fs::rename(new, target) {
        if had {
            let _ = std::fs::rename(&prev, target);
        }
        return Err(e.into());
    }
    Ok(had.then_some(prev))
}

/// Swaps the installed copy and the kept one.
pub fn swap_back(target: &Path, prev: &Path) -> Result<()> {
    let tmp = with_suffix(target, ".sangraha-swap");
    remove(&tmp)?;
    std::fs::rename(target, &tmp)?;
    if let Err(e) = std::fs::rename(prev, target) {
        let _ = std::fs::rename(&tmp, target);
        return Err(e.into());
    }
    std::fs::rename(&tmp, prev)?;
    Ok(())
}

/// Unpacks an archive into a new folder beside `target` and returns that folder (the archive's
/// single top folder, if it has one).
pub fn unpack(file: &Path, target: &Path) -> Result<PathBuf> {
    let stage = with_suffix(target, ".sangraha-new");
    remove(&stage)?;
    std::fs::create_dir_all(&stage)?;
    // GNU tar and Windows' tar (bsdtar, which also reads zip) both refuse `..` and absolute paths.
    let out = Command::new("tar")
        .arg("-xf")
        .arg(file)
        .arg("-C")
        .arg(&stage)
        .output()?;
    if !out.status.success() {
        let _ = remove(&stage);
        return Err(Error::Install(format!(
            "could not unpack {}: {}",
            file.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    let entries: Vec<_> = std::fs::read_dir(&stage)?.flatten().collect();
    if let [only] = entries.as_slice()
        && only.path().is_dir()
    {
        let inner = with_suffix(target, ".sangraha-root");
        remove(&inner)?;
        std::fs::rename(only.path(), &inner)?;
        remove(&stage)?;
        std::fs::rename(&inner, &stage)?;
    }
    Ok(stage)
}

/// Copies a single-file app (AppImage, portable exe) beside `target`, ready to swap in.
fn stage_file(file: &Path, target: &Path, folder: bool, exe_name: &str) -> Result<PathBuf> {
    let stage = with_suffix(target, ".sangraha-new");
    remove(&stage)?;
    let dest = if folder {
        std::fs::create_dir_all(&stage)?;
        stage.join(exe_name)
    } else {
        stage.clone()
    };
    std::fs::copy(file, &dest)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(stage)
}

fn menu_entry(
    id: &str,
    name: &str,
    repo: &str,
    launcher: &Path,
    dir: &Path,
) -> Result<Option<PathBuf>> {
    menu_entry_in(
        &home().join(".local/share/applications"),
        id,
        name,
        repo,
        launcher,
        dir,
    )
}

/// Writes the menu entry (a `.desktop` file in `menu`, or a Start menu shortcut on Windows).
fn menu_entry_in(
    menu: &Path,
    id: &str,
    name: &str,
    repo: &str,
    launcher: &Path,
    dir: &Path,
) -> Result<Option<PathBuf>> {
    if cfg!(windows) {
        let Some(appdata) = std::env::var_os("APPDATA") else {
            return Ok(None);
        };
        let lnk = PathBuf::from(appdata).join(format!(
            "Microsoft\\Windows\\Start Menu\\Programs\\{name}.lnk"
        ));
        let q = |p: &Path| p.to_string_lossy().replace('\'', "''");
        let script = format!(
            "$s=(New-Object -ComObject WScript.Shell).CreateShortcut('{}');$s.TargetPath='{}';$s.WorkingDirectory='{}';$s.Save()",
            q(&lnk),
            q(launcher),
            q(dir)
        );
        let ok = Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .status()
            .is_ok_and(|s| s.success());
        return Ok(ok.then_some(lnk));
    }
    std::fs::create_dir_all(menu)?;
    let file = menu.join(format!("sangraha-{id}.desktop"));
    let icon = find_icon(dir, id)
        .map(|i| format!("Icon={}\n", i.display()))
        .unwrap_or_default();
    let quote = |p: &Path| {
        p.to_string_lossy()
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
    };
    std::fs::write(
        &file,
        format!(
            "[Desktop Entry]\nType=Application\nName={name}\nComment=Installed by Sangraha from {repo}\nExec=\"{}\"\nPath={}\n{icon}Terminal=false\nCategories=Utility;\n",
            quote(launcher),
            dir.display()
        ),
    )?;
    Ok(Some(file))
}

/// A PNG named like an icon or like the app, a few folders deep.
fn find_icon(dir: &Path, id: &str) -> Option<PathBuf> {
    if !dir.is_dir() {
        return None;
    }
    let mut best: Option<(u64, PathBuf)> = None;
    let mut stack = vec![(dir.to_path_buf(), 0)];
    while let Some((d, depth)) = stack.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() && depth < 4 {
                stack.push((p, depth + 1));
            } else if p.extension().is_some_and(|x| x == "png") {
                let n = p
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_ascii_lowercase();
                let size = e.metadata().map(|m| m.len()).unwrap_or(0);
                if (n.contains("icon") || n == id || n.contains("logo"))
                    && best.as_ref().is_none_or(|(s, _)| size > *s)
                {
                    best = Some((size, p));
                }
            }
        }
    }
    best.map(|(_, p)| p)
}

// ---------------------------------------------------------------------------------------------
// Install, update, roll back

/// Where an install of this kind goes: the existing copy if its shape fits, else a new place.
fn target_for(kind: Kind, existing: Option<&Here>, apps_dir: &Path, name: &str) -> PathBuf {
    let wants_dir = matches!(kind, Kind::Archive | Kind::Exe);
    if let Some(h) = existing
        && h.path.is_dir() == wants_dir
    {
        return h.path.clone();
    }
    if kind == Kind::AppImage {
        apps_dir.join(format!("{name}.AppImage"))
    } else {
        apps_dir.join(name)
    }
}

/// Installs or updates `key` to the release the Apps tab offers. With `allow_unverified`, a
/// file without a published SHA-256 is installed anyway (the owner was asked first).
pub fn install(
    cfg: &Config,
    snap: &apps::AppsSnapshot,
    key: &str,
    allow_unverified: bool,
    progress: &(dyn Fn(u64, u64) + Sync),
) -> Result<String> {
    let app = snap
        .apps
        .iter()
        .find(|a| a.full_name.eq_ignore_ascii_case(key))
        .ok_or_else(|| Error::Install(format!("{key} is not in the Apps list; refresh first")))?;
    let release = apps::offered(app, cfg, crate::now())
        .ok_or_else(|| Error::Install("no release to install yet".into()))?;
    let rule = cfg
        .asset_rules
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(key))
        .map(|(_, v)| v.as_str());
    let (asset, build) = apps::pick(&release.assets, rule, apps::this_os(), apps::this_arch())
        .ok_or_else(|| {
            Error::Install(format!("{} has no download for this computer", release.tag))
        })?;
    let kind = build.kind;
    if matches!(kind, Kind::SystemPackage | Kind::Apk | Kind::Mac) {
        return Err(Error::Install(format!(
            "{} is not something Sangraha installs here",
            asset.name
        )));
    }
    let repo = key.split('/').nth(1).unwrap_or(key);
    let name = apps::display_name(&asset.name, repo);
    let id = apps::app_id(&name);

    let mut registry = Registry::load();
    let existing = registry
        .get(key)
        .map(|i| Here {
            version: Some(i.tag.clone()),
            path: i.path.clone(),
            launcher: i.launcher.clone(),
            by_sangraha: true,
            previous: None,
        })
        .or_else(|| detect(cfg, &name));
    let target = target_for(kind, existing.as_ref(), &cfg.apps_dir(), &name);
    if let Some(h) = &existing
        && running(&h.path)
    {
        return Err(Error::Install(format!(
            "{name} is running. Close it, then update."
        )));
    }

    let token = crate::github::token()?;
    let agent = download_agent();
    let assets = release_assets(&agent, &token, &app.full_name, &release.tag)?;
    let rest = assets
        .iter()
        .find(|a| a.name == asset.name)
        .ok_or_else(|| Error::Install(format!("{} is gone from the release", asset.name)))?;
    let expected = expected_sum(&agent, &token, &assets, rest)?;
    if expected.is_none() && !allow_unverified {
        return Err(Error::Unverified(format!(
            "{} has no published SHA-256, so Sangraha cannot check the download",
            asset.name
        )));
    }

    let downloads = data_dir().join("downloads");
    std::fs::create_dir_all(&downloads)?;
    let file = downloads.join(&asset.name);
    let part = with_suffix(&file, ".part");
    let got = download(&agent, &token, &rest.api_url, &part, asset.size, progress);
    let got = match got {
        Ok(g) => g,
        Err(e) => {
            let _ = remove(&part);
            return Err(e);
        }
    };
    if let Some(want) = &expected
        && &got != want
    {
        let _ = remove(&part);
        return Err(Error::Install(format!(
            "{}: the download does not match its published SHA-256. Nothing was installed.",
            asset.name
        )));
    }
    std::fs::rename(&part, &file)?;

    let result = place(kind, &file, &target, &name, &id);
    let _ = remove(&file);
    let (launcher, previous_path) = result?;

    let was = registry.get(key).cloned();
    let previous_tag = was
        .as_ref()
        .map(|w| w.tag.clone())
        .or_else(|| existing.as_ref().and_then(|h| h.version.clone()))
        .unwrap_or_else(|| "earlier copy".into());
    // A menu entry only for a new install: a copy found on disk already has its own.
    let menu = match (&was, &existing) {
        (Some(w), _) => w.menu_entry.clone(),
        (None, Some(h)) if h.path == target => None,
        _ => match (&launcher, kind) {
            (Some(l), Kind::Archive | Kind::Exe | Kind::AppImage) => {
                let dir = if target.is_dir() {
                    target.clone()
                } else {
                    target.parent().unwrap_or(&target).to_path_buf()
                };
                menu_entry(&id, &name, &app.full_name, l, &dir)
                    .ok()
                    .flatten()
            }
            _ => None,
        },
    };
    registry.set(
        key,
        Installed {
            repo: app.full_name.clone(),
            tag: release.tag.clone(),
            asset: asset.name.clone(),
            kind,
            path: target.clone(),
            launcher,
            sha256: got,
            installed_at: crate::now(),
            previous: previous_path.map(|p| Previous {
                tag: previous_tag,
                path: p,
            }),
            menu_entry: menu,
        },
    );
    registry.save()?;
    let checked = if expected.is_some() {
        "SHA-256 checked"
    } else {
        "not checked: no SHA-256 published"
    };
    Ok(format!(
        "{name} {} installed in {} ({checked})",
        release.tag,
        target.display()
    ))
}

/// Puts the downloaded file in place; returns the launcher and where the old copy went.
fn place(
    kind: Kind,
    file: &Path,
    target: &Path,
    name: &str,
    id: &str,
) -> Result<(Option<PathBuf>, Option<PathBuf>)> {
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    match kind {
        Kind::Archive => {
            let stage = unpack(file, target)?;
            if find_launcher(&stage, name).is_none() {
                let _ = remove(&stage);
                return Err(Error::Install(format!(
                    "found no program to start in {}; add a download rule in settings if this is the wrong file",
                    file.display()
                )));
            }
            let prev = swap_in(&stage, target)?;
            Ok((find_launcher(target, name), prev))
        }
        Kind::AppImage => {
            let stage = stage_file(file, target, false, "")?;
            let prev = swap_in(&stage, target)?;
            Ok((Some(target.to_path_buf()), prev))
        }
        Kind::Exe => {
            let exe = format!("{name}.exe");
            let stage = stage_file(file, target, true, &exe)?;
            let prev = swap_in(&stage, target)?;
            Ok((Some(target.join(exe)), prev))
        }
        Kind::Setup => {
            // The installer is the publisher's; it shows its own steps and chooses its own folder.
            let is_msi = file
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("msi"));
            let status = if is_msi {
                Command::new("msiexec").arg("/i").arg(file).status()?
            } else {
                Command::new(file).status()?
            };
            if !status.success() {
                return Err(Error::Install(format!(
                    "the {name} installer stopped ({status})"
                )));
            }
            let _ = id;
            Ok((None, None))
        }
        _ => Err(Error::Install("not installable here".into())),
    }
}

/// Swaps the installed copy with the one kept from before the last update.
pub fn rollback(key: &str) -> Result<String> {
    let mut registry = Registry::load();
    let mut i = registry
        .get(key)
        .cloned()
        .ok_or_else(|| Error::Install(format!("Sangraha did not install {key}")))?;
    let prev = i
        .previous
        .clone()
        .ok_or_else(|| Error::Install("no earlier version is kept".into()))?;
    if !std::fs::symlink_metadata(&prev.path).is_ok() {
        return Err(Error::Install(format!("{} is gone", prev.path.display())));
    }
    if running(&i.path) {
        return Err(Error::Install(
            "The app is running. Close it, then roll back.".into(),
        ));
    }
    swap_back(&i.path, &prev.path)?;
    let name = i
        .launcher
        .as_ref()
        .and_then(|l| l.file_stem())
        .map(|s| s.to_string_lossy().into_owned());
    let newer = std::mem::replace(&mut i.tag, prev.tag.clone());
    i.previous = Some(Previous {
        tag: newer,
        path: prev.path,
    });
    if i.path.is_dir() {
        i.launcher = find_launcher(&i.path, name.as_deref().unwrap_or(""));
    }
    i.sha256.clear();
    registry.set(key, i.clone());
    registry.save()?;
    Ok(format!("Back to {} in {}", i.tag, i.path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    fn make_exe(p: &Path) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::write(p, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn archive_update_keeps_the_old_copy_and_rolls_back() {
        let t = tempfile::tempdir().unwrap();
        // A release archive with one top folder, as Flutter bundles are shipped.
        let src = t.path().join("src/Chanda-0.5.1");
        std::fs::create_dir_all(src.join("lib")).unwrap();
        make_exe(&src.join("chanda"));
        make_exe(&src.join("lib/libapp.so"));
        std::fs::write(src.join("VERSION"), "new").unwrap();
        let tar = t.path().join("Chanda-0.5.1-linux-x64.tar.gz");
        assert!(
            Command::new("tar")
                .arg("-czf")
                .arg(&tar)
                .arg("-C")
                .arg(t.path().join("src"))
                .arg("Chanda-0.5.1")
                .status()
                .unwrap()
                .success()
        );

        let target = t.path().join("apps/Chanda");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("VERSION"), "old").unwrap();

        let (launcher, prev) = place(Kind::Archive, &tar, &target, "Chanda", "chanda").unwrap();
        assert_eq!(launcher, Some(target.join("chanda")));
        let prev = prev.unwrap();
        assert_eq!(
            std::fs::read_to_string(target.join("VERSION")).unwrap(),
            "new"
        );
        assert_eq!(
            std::fs::read_to_string(prev.join("VERSION")).unwrap(),
            "old"
        );
        assert!(!with_suffix(&target, ".sangraha-new").exists());

        swap_back(&target, &prev).unwrap();
        assert_eq!(
            std::fs::read_to_string(target.join("VERSION")).unwrap(),
            "old"
        );
        assert_eq!(
            std::fs::read_to_string(prev.join("VERSION")).unwrap(),
            "new"
        );
    }

    #[cfg(unix)]
    #[test]
    fn appimage_and_bad_archives() {
        let t = tempfile::tempdir().unwrap();
        let img = t.path().join("dl.AppImage");
        std::fs::write(&img, "v2").unwrap();
        let target = t.path().join("apps/App.AppImage");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&target, "v1").unwrap();
        let (l, prev) = place(Kind::AppImage, &img, &target, "App", "app").unwrap();
        assert_eq!(l.as_deref(), Some(target.as_path()));
        assert!(executable(&target));
        assert_eq!(std::fs::read_to_string(prev.unwrap()).unwrap(), "v1");

        // An archive with no program in it is refused and leaves the old copy alone.
        let src = t.path().join("docs");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("README"), "x").unwrap();
        let tar = t.path().join("docs-linux.tar.gz");
        assert!(
            Command::new("tar")
                .arg("-czf")
                .arg(&tar)
                .arg("-C")
                .arg(&src)
                .arg(".")
                .status()
                .unwrap()
                .success()
        );
        let dir = t.path().join("apps/Docs");
        std::fs::create_dir_all(&dir).unwrap();
        assert!(place(Kind::Archive, &tar, &dir, "Docs", "docs").is_err());
        assert!(dir.exists() && !with_suffix(&dir, ".sangraha-new").exists());
    }

    #[cfg(unix)]
    #[test]
    fn launcher_and_version_on_disk() {
        let t = tempfile::tempdir().unwrap();
        let d = t.path().join("anveshana");
        std::fs::create_dir_all(d.join("data/flutter_assets")).unwrap();
        make_exe(&d.join("anveshana"));
        make_exe(&d.join("helper"));
        std::fs::write(
            d.join("data/flutter_assets/version.json"),
            r#"{"version":"0.14.0"}"#,
        )
        .unwrap();
        assert_eq!(
            find_launcher(&d, "Graha Anveshana"),
            Some(d.join("anveshana"))
        );
        assert_eq!(version_of(&d).as_deref(), Some("0.14.0"));
        assert_eq!(
            version_of(Path::new("/x/Ganana_0.1.0_amd64.AppImage")).as_deref(),
            Some("0.1.0")
        );
        assert_eq!(ids("Graha Anveshana"), vec!["graha-anveshana", "anveshana"]);
        assert_eq!(ids("Chanda"), vec!["chanda"]);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn menu_entry_points_at_the_launcher() {
        let t = tempfile::tempdir().unwrap();
        let dir = t.path().join("Arogya Swasthya");
        std::fs::create_dir_all(dir.join("data")).unwrap();
        std::fs::write(dir.join("data/app_icon.png"), [0u8; 10]).unwrap();
        let launcher = dir.join("arogya");
        let menu = t.path().join("menu");
        let f = menu_entry_in(
            &menu,
            "arogya-swasthya",
            "Arogya Swasthya",
            "me/arogya",
            &launcher,
            &dir,
        )
        .unwrap()
        .unwrap();
        assert_eq!(f, menu.join("sangraha-arogya-swasthya.desktop"));
        let text = std::fs::read_to_string(f).unwrap();
        assert!(
            text.contains(&format!("Exec=\"{}\"\n", launcher.display())),
            "{text}"
        );
        assert!(text.contains("Name=Arogya Swasthya\n"));
        assert!(text.contains(&format!("Icon={}", dir.join("data/app_icon.png").display())));
    }

    #[test]
    fn registry_round_trip() {
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("apps.json");
        let mut r = Registry::load_from(&p);
        r.set(
            "Me/App",
            Installed {
                repo: "me/app".into(),
                tag: "v1".into(),
                asset: "a".into(),
                kind: Kind::Archive,
                path: "/x".into(),
                launcher: None,
                sha256: String::new(),
                installed_at: 1,
                previous: None,
                menu_entry: None,
            },
        );
        r.save_to(&p).unwrap();
        assert_eq!(Registry::load_from(&p).get("me/app").unwrap().tag, "v1");
    }

    #[test]
    fn running_sees_this_test() {
        let exe = std::env::current_exe().unwrap();
        if cfg!(target_os = "linux") {
            assert!(running(exe.parent().unwrap()));
            assert!(!running(Path::new("/nowhere/at/all")));
        }
    }
}
