//! The Apps tab: apps published as GitHub releases, the right download for this computer, and
//! whether the copy installed here is the newest one.
//!
//! The owner's repositories with a release are listed, plus any other repository the owner adds.
//! Reading only; installing is in `install`.

use std::cmp::Ordering;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::Config;
use crate::github::{Rate, Snapshot};
use crate::install::{Installed, Registry};
use crate::{Error, Result};

/// Repositories per GraphQL query.
const BATCH: usize = 25;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Asset {
    pub name: String,
    pub size: u64,
    /// `sha256:<hex>` as GitHub computed it on upload.
    pub digest: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AppRelease {
    pub tag: String,
    pub prerelease: bool,
    pub published_at: Option<String>,
    pub url: String,
    pub assets: Vec<Asset>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RemoteApp {
    pub full_name: String,
    pub description: Option<String>,
    pub url: String,
    pub archived: bool,
    /// Newest first, drafts left out.
    pub releases: Vec<AppRelease>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AppsSnapshot {
    pub apps: Vec<RemoteApp>,
    pub rate: Option<Rate>,
    pub fetched_at: i64,
    pub warnings: Vec<String>,
}

const QUERY_FRAGMENT: &str = r#"
fragment App on Repository {
  nameWithOwner description url isArchived
  releases(first: 3, orderBy: {field: CREATED_AT, direction: DESC}) {
    nodes { tagName isPrerelease isDraft publishedAt url
      releaseAssets(first: 60) { nodes { name size digest } } }
  }
}"#;

/// A valid `owner/name`. Names go into GraphQL variables, never into the query text, but a
/// clear message beats a GitHub error.
pub fn valid_name(full: &str) -> bool {
    let ok = |s: &str| {
        !s.is_empty()
            && s.len() <= 100
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
    };
    matches!(full.split_once('/'), Some((o, n)) if ok(o) && ok(n) && !n.contains('/'))
}

/// The repositories the Apps tab follows: the owner's own with a release, then the extras.
pub fn followed(snap: Option<&Snapshot>, cfg: &Config) -> Vec<String> {
    let mut out: Vec<String> = snap
        .map(|s| {
            s.repos
                .iter()
                .filter(|r| r.release.is_some() && !r.fork)
                .map(|r| r.full_name.clone())
                .collect()
        })
        .unwrap_or_default();
    for e in &cfg.extra_apps {
        if valid_name(e) && !out.iter().any(|o| o.eq_ignore_ascii_case(e)) {
            out.push(e.clone());
        }
    }
    out
}

/// Releases and their files for each repository, 25 to a query.
pub fn fetch(token: &str, names: &[String]) -> Result<AppsSnapshot> {
    let agent = crate::github::agent();
    let mut snap = AppsSnapshot {
        fetched_at: crate::now(),
        ..Default::default()
    };
    for batch in names.chunks(BATCH) {
        let page = crate::github::repos_batch(&agent, token, batch, "App", QUERY_FRAGMENT)?;
        parse(&page, batch, &mut snap)?;
    }
    Ok(snap)
}

pub fn parse(page: &Value, batch: &[String], snap: &mut AppsSnapshot) -> Result<()> {
    let data = &page["data"];
    if data.is_null() {
        let msg = page["errors"][0]["message"]
            .as_str()
            .unwrap_or("no data in the answer");
        return Err(Error::GitHub(msg.to_owned()));
    }
    for (i, full) in batch.iter().enumerate() {
        let r = &data[format!("r{i}")];
        if r.is_null() {
            snap.warnings.push(format!("{full}: not found on GitHub"));
            continue;
        }
        let releases = r["releases"]["nodes"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|n| !n["isDraft"].as_bool().unwrap_or(false))
            .map(|n| AppRelease {
                tag: n["tagName"].as_str().unwrap_or_default().to_owned(),
                prerelease: n["isPrerelease"].as_bool().unwrap_or(false),
                published_at: n["publishedAt"].as_str().map(str::to_owned),
                url: n["url"].as_str().unwrap_or_default().to_owned(),
                assets: n["releaseAssets"]["nodes"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|a| Asset {
                        name: a["name"].as_str().unwrap_or_default().to_owned(),
                        size: a["size"].as_u64().unwrap_or(0),
                        digest: a["digest"].as_str().map(str::to_owned),
                    })
                    .collect(),
            })
            .collect();
        snap.apps.push(RemoteApp {
            full_name: r["nameWithOwner"].as_str().unwrap_or(full).to_owned(),
            description: r["description"].as_str().map(str::to_owned),
            url: r["url"].as_str().unwrap_or_default().to_owned(),
            archived: r["isArchived"].as_bool().unwrap_or(false),
            releases,
        });
    }
    if let Some(rl) = data.get("rateLimit").filter(|v| !v.is_null()) {
        snap.rate = serde_json::from_value(rl.clone()).ok();
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Which file is for this computer

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Os {
    Linux,
    Windows,
    Mac,
    Android,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    /// `.tar.gz` (Linux) or `.zip` (Windows) holding the app folder.
    Archive,
    AppImage,
    /// A Windows `.exe` that is the app itself.
    Exe,
    /// A Windows installer (`setup.exe`, `.msi`).
    Setup,
    /// `.deb`, `.rpm`, `.pkg.tar.zst`: needs the system's package manager and an admin password.
    SystemPackage,
    Apk,
    Mac,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arch {
    X64,
    Arm64,
    Other,
    Any,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Build {
    pub os: Os,
    pub arch: Arch,
    pub kind: Kind,
}

fn words(lower: &str) -> Vec<&str> {
    lower
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect()
}

/// What a release file is, from its name. `None` for checksums, signatures, sources and
/// anything else that is not an app.
pub fn classify(name: &str) -> Option<Build> {
    let l = name.to_ascii_lowercase();
    let w = words(&l);
    let has = |t: &str| w.contains(&t);
    let arch = if has("aarch64") || has("arm64") || l.contains("arm64-v8a") {
        Arch::Arm64
    } else if has("armeabi")
        || has("armv7")
        || has("armhf")
        || has("i386")
        || has("x86") && !l.contains("x86_64")
    {
        Arch::Other
    } else if l.contains("x86_64") || has("amd64") || has("x64") {
        Arch::X64
    } else {
        Arch::Any
    };
    let build = |os, kind| Some(Build { os, arch, kind });
    let mac = has("mac") || has("macos") || has("darwin") || has("osx") || l.contains(".app.");
    let win = has("windows") || has("win") || has("win64") || has("msvc");
    if l.ends_with(".apk") {
        build(Os::Android, Kind::Apk)
    } else if l.ends_with(".appimage") {
        build(Os::Linux, Kind::AppImage)
    } else if l.ends_with(".dmg") || (mac && (l.ends_with(".tar.gz") || l.ends_with(".zip"))) {
        build(Os::Mac, Kind::Mac)
    } else if l.ends_with(".tar.gz") || l.ends_with(".tgz") || l.ends_with(".tar.xz") {
        // A tarball without "linux" in its name is the source code.
        has("linux").then_some(())?;
        build(Os::Linux, Kind::Archive)
    } else if l.ends_with(".deb") || l.ends_with(".rpm") || l.ends_with(".pkg.tar.zst") {
        build(Os::Linux, Kind::SystemPackage)
    } else if l.ends_with(".zip") {
        // Tauri names its Windows zip `App_1.0_x64_portable.zip`, without "windows".
        (win || (has("portable") && !has("linux"))).then_some(())?;
        build(Os::Windows, Kind::Archive)
    } else if l.ends_with(".msi") {
        build(Os::Windows, Kind::Setup)
    } else if l.ends_with(".exe") {
        let setup = has("setup") || has("installer") || has("install");
        build(Os::Windows, if setup { Kind::Setup } else { Kind::Exe })
    } else {
        None
    }
}

/// This computer, as release files name it.
pub fn this_os() -> Os {
    if cfg!(windows) {
        Os::Windows
    } else if cfg!(target_os = "macos") {
        Os::Mac
    } else {
        Os::Linux
    }
}

pub fn this_arch() -> Arch {
    match std::env::consts::ARCH {
        "x86_64" => Arch::X64,
        "aarch64" => Arch::Arm64,
        _ => Arch::Other,
    }
}

/// Lower is better. `None`: not for this computer.
fn rank(name: &str, b: Build, os: Os, arch: Arch) -> Option<u32> {
    if b.os != os {
        return None;
    }
    let arch_ok = b.arch == arch
        || b.arch == Arch::Any && arch == Arch::X64
        || b.arch == Arch::Any && os == Os::Mac;
    if !arch_ok {
        return None;
    }
    let l = name.to_ascii_lowercase();
    let w = words(&l);
    let base = match (os, b.kind) {
        (Os::Linux, Kind::Archive) if w.contains(&"portable") => 0,
        (Os::Linux, Kind::Archive) => 1,
        (Os::Linux, Kind::AppImage) => 2,
        (Os::Windows, Kind::Archive) => 0,
        (Os::Windows, Kind::Exe) => 1,
        (Os::Windows, Kind::Setup) if l.ends_with(".exe") => 2,
        (Os::Windows, Kind::Setup) => 3,
        (_, Kind::SystemPackage) => 20,
        (Os::Mac, Kind::Mac) => 0,
        _ => return None,
    };
    // Variants (a debug build, another renderer) come after the plain build.
    let variant = ["debug", "wpe", "dbg", "symbols"]
        .iter()
        .any(|v| w.contains(v));
    Some(base + if variant { 10 } else { 0 })
}

/// `*` matches any text; the rest is compared ignoring case.
pub fn glob(pattern: &str, name: &str) -> bool {
    let (p, n) = (pattern.to_ascii_lowercase(), name.to_ascii_lowercase());
    let parts: Vec<&str> = p.split('*').collect();
    if parts.len() == 1 {
        return p == n;
    }
    let mut rest = n.as_str();
    for (i, part) in parts.iter().enumerate() {
        if i == 0 {
            let Some(r) = rest.strip_prefix(part) else {
                return false;
            };
            rest = r;
        } else if i == parts.len() - 1 {
            return rest.ends_with(part);
        } else {
            let Some(at) = rest.find(part) else {
                return false;
            };
            rest = &rest[at + part.len()..];
        }
    }
    true
}

/// The file to install on this computer, if the release has one. A rule from the settings wins.
pub fn pick<'a>(
    assets: &'a [Asset],
    rule: Option<&str>,
    os: Os,
    arch: Arch,
) -> Option<(&'a Asset, Build)> {
    if let Some(rule) = rule {
        let a = assets.iter().find(|a| glob(rule, &a.name))?;
        let b = classify(&a.name).unwrap_or(Build {
            os,
            arch,
            kind: if os == Os::Windows {
                Kind::Exe
            } else {
                Kind::AppImage
            },
        });
        return Some((a, b));
    }
    assets
        .iter()
        .filter_map(|a| {
            let b = classify(&a.name)?;
            Some((rank(&a.name, b, os, arch)?, a, b))
        })
        .min_by_key(|(r, a, _)| (*r, a.name.len()))
        .map(|(_, a, b)| (a, b))
}

/// "Chanda" from `Chanda-0.5.1-linux-x64.tar.gz`, "Arogya Swasthya" from
/// `Arogya.Swasthya_0.1.0_amd64.AppImage`: the words before the version number.
pub fn display_name(asset: &str, repo: &str) -> String {
    let b = asset.as_bytes();
    let mut cut = None;
    for i in 1..b.len() {
        let sep = matches!(b[i - 1], b'-' | b'_' | b'.');
        let digit =
            b[i].is_ascii_digit() || (b[i] == b'v' && b.get(i + 1).is_some_and(u8::is_ascii_digit));
        // A version has a dot before the next separator: "0.5.1", not the "64" of "x86_64".
        let token = asset[i..].split(['-', '_']).next().unwrap_or("");
        if sep && digit && token.contains('.') {
            cut = Some(i - 1);
            break;
        }
    }
    let stem = match cut {
        Some(c) if c > 0 => &asset[..c],
        _ => repo.split('-').next().unwrap_or(repo),
    };
    stem.split(['-', '_', '.', ' '])
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut c = w.chars();
            c.next()
                .map(|f| f.to_uppercase().chain(c).collect::<String>())
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Folder and launcher name: "graha-anveshana" for "Graha Anveshana".
pub fn app_id(display: &str) -> String {
    display.to_ascii_lowercase().replace(' ', "-")
}

// ---------------------------------------------------------------------------------------------
// Versions

/// `v1.2.3-beta.4` → numbers `[1,2,3]` and pre-release parts `["beta", "4"]`.
fn parse_version(v: &str) -> Option<(Vec<u64>, Vec<String>)> {
    let v = v.trim().trim_start_matches(['v', 'V']);
    let (core, pre) = match v.split_once(['-', '+']) {
        Some((c, p)) => (c, p.split('+').next().unwrap_or(p)),
        None => (v, ""),
    };
    let nums = core
        .split('.')
        .map(|n| n.parse().ok())
        .collect::<Option<Vec<u64>>>()?;
    let pre = pre
        .split('.')
        .filter(|p| !p.is_empty())
        .map(str::to_owned)
        .collect();
    Some((nums, pre))
}

/// Semantic-version order where both parse; otherwise only "same" or "different" is known.
pub fn compare(a: &str, b: &str) -> Option<Ordering> {
    let (an, ap) = parse_version(a)?;
    let (bn, bp) = parse_version(b)?;
    let len = an.len().max(bn.len());
    for i in 0..len {
        let o = an.get(i).unwrap_or(&0).cmp(bn.get(i).unwrap_or(&0));
        if o != Ordering::Equal {
            return Some(o);
        }
    }
    // A release without a pre-release part is newer than any pre-release of it.
    match (ap.is_empty(), bp.is_empty()) {
        (true, true) => return Some(Ordering::Equal),
        (true, false) => return Some(Ordering::Greater),
        (false, true) => return Some(Ordering::Less),
        _ => {}
    }
    for (x, y) in ap.iter().zip(&bp) {
        let o = match (x.parse::<u64>(), y.parse::<u64>()) {
            (Ok(x), Ok(y)) => x.cmp(&y),
            (Ok(_), Err(_)) => Ordering::Less,
            (Err(_), Ok(_)) => Ordering::Greater,
            _ => x.cmp(y),
        };
        if o != Ordering::Equal {
            return Some(o);
        }
    }
    Some(ap.len().cmp(&bp.len()))
}

pub fn same_version(a: &str, b: &str) -> bool {
    compare(a, b).map_or_else(
        || a.trim_start_matches(['v', 'V']) == b.trim_start_matches(['v', 'V']),
        |o| o == Ordering::Equal,
    )
}

// ---------------------------------------------------------------------------------------------
// The Apps board

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "camelCase")]
pub enum AppStatus {
    Update,
    /// Installed, but Sangraha cannot tell which version.
    Unknown,
    /// A newer release exists but is younger than the chosen delay.
    Waiting,
    UpToDate,
    NotInstalled,
    /// No file in the newest release fits this computer.
    NoBuild,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Offer {
    pub tag: String,
    pub prerelease: bool,
    pub published_at: Option<String>,
    pub url: String,
    pub asset: String,
    pub size: u64,
    pub kind: Kind,
    /// GitHub has a SHA-256 for the file, or the release publishes one.
    pub checksum: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Here {
    pub version: Option<String>,
    pub path: PathBuf,
    pub launcher: Option<PathBuf>,
    /// Installed by Sangraha (false: found on disk, installed some other way).
    pub by_sangraha: bool,
    /// The version kept for "roll back", if any.
    pub previous: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AppRow {
    pub key: String,
    pub name: String,
    pub repo: String,
    pub description: Option<String>,
    pub url: String,
    pub own: bool,
    pub status: AppStatus,
    pub offer: Option<Offer>,
    /// Newest release of any kind, when the offer is an older one (delay or no pre-releases).
    pub newest: Option<String>,
    pub here: Option<Here>,
    /// Newest Android version, shown only: phones get apps through Brahma Suite or Obtainium.
    pub android: Option<String>,
    pub notes: Vec<String>,
    pub hidden: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AppCounts {
    pub all: u32,
    pub installed: u32,
    pub updates: u32,
    pub not_installed: u32,
    pub hidden: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AppsBoard {
    pub checked_at: Option<i64>,
    pub rate: Option<Rate>,
    pub apps_dir: PathBuf,
    pub rows: Vec<AppRow>,
    pub counts: AppCounts,
    pub warnings: Vec<String>,
}

fn days_since(iso: &str, now: i64) -> Option<i64> {
    Some((now - crate::parse_time(iso)?) / 86_400)
}

/// The release to offer: newest allowed by the pre-release and delay settings.
pub fn offered<'a>(app: &'a RemoteApp, cfg: &Config, now: i64) -> Option<&'a AppRelease> {
    app.releases.iter().find(|r| {
        (cfg.prereleases || !r.prerelease)
            && (cfg.delay_days == 0
                || r.published_at
                    .as_deref()
                    .and_then(|p| days_since(p, now))
                    .is_some_and(|d| d >= cfg.delay_days as i64))
    })
}

pub fn build(
    snap: Option<&AppsSnapshot>,
    own_login: Option<&str>,
    cfg: &Config,
    registry: &Registry,
    detect: &dyn Fn(&str) -> Option<Here>,
) -> AppsBoard {
    let (os, arch) = (this_os(), this_arch());
    let now = crate::now();
    let mut rows = Vec::new();
    for app in snap.map(|s| s.apps.as_slice()).unwrap_or_default() {
        let key = app.full_name.clone();
        let repo = key.split('/').nth(1).unwrap_or(&key).to_owned();
        // Only repositories that ship something a person installs belong here.
        let any_app = app
            .releases
            .first()
            .is_some_and(|r| r.assets.iter().any(|a| classify(&a.name).is_some()));
        let rule = cfg
            .asset_rules
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(&key))
            .map(|(_, v)| v.as_str());
        if !any_app && rule.is_none() {
            continue;
        }
        let release = offered(app, cfg, now);
        let picked = release.and_then(|r| pick(&r.assets, rule, os, arch).map(|(a, b)| (r, a, b)));
        let newest = app
            .releases
            .iter()
            .find(|r| cfg.prereleases || !r.prerelease);
        let android = app
            .releases
            .iter()
            .find(|r| {
                r.assets
                    .iter()
                    .any(|a| a.name.to_ascii_lowercase().ends_with(".apk"))
            })
            .map(|r| r.tag.clone());
        let name_source = picked
            .map(|(_, a, _)| a.name.clone())
            .or_else(|| {
                app.releases
                    .first()
                    .and_then(|r| r.assets.iter().find(|a| classify(&a.name).is_some()))
                    .map(|a| a.name.clone())
            })
            .unwrap_or_default();
        let name = display_name(&name_source, &repo);
        let offer = picked.map(|(r, a, b)| Offer {
            tag: r.tag.clone(),
            prerelease: r.prerelease,
            published_at: r.published_at.clone(),
            url: r.url.clone(),
            asset: a.name.clone(),
            size: a.size,
            kind: b.kind,
            checksum: a.digest.is_some()
                || r.assets.iter().any(|x| is_checksum_file(&x.name, &a.name)),
        });
        let here = registry.get(&key).map(|i: &Installed| Here {
            version: Some(i.tag.clone()),
            path: i.path.clone(),
            launcher: i.launcher.clone(),
            by_sangraha: true,
            previous: i.previous.as_ref().map(|p| p.tag.clone()),
        });
        let here = here.or_else(|| detect(&name));
        let mut notes = Vec::new();
        let status = match (&offer, &here) {
            (None, _) if newest.is_some() && release.is_none() => AppStatus::Waiting,
            (None, _) => AppStatus::NoBuild,
            (Some(_), None) => AppStatus::NotInstalled,
            (Some(o), Some(h)) => match &h.version {
                None => AppStatus::Unknown,
                Some(v) if same_version(v, &o.tag) => AppStatus::UpToDate,
                Some(v) if compare(v, &o.tag) == Some(Ordering::Greater) => {
                    notes.push(format!("{v} here is newer than {} on GitHub", o.tag));
                    AppStatus::UpToDate
                }
                Some(_) => AppStatus::Update,
            },
        };
        let newest_tag = newest.map(|r| r.tag.clone());
        let newest = newest_tag.filter(|t| offer.as_ref().is_none_or(|o| &o.tag != t));
        if let (Some(n), true) = (&newest, cfg.delay_days > 0) {
            notes.push(format!("{n} waits until it is {} days old", cfg.delay_days));
        }
        if status == AppStatus::NoBuild {
            let system = release.and_then(|r| {
                r.assets.iter().find(|a| {
                    classify(&a.name).is_some_and(|b| b.kind == Kind::SystemPackage && b.os == os)
                })
            });
            notes.push(match system {
                Some(a) => format!("Only a system package ({}) for this computer; Sangraha installs without an admin password, so open the release page", a.name),
                None => "No download for this computer in the newest release".to_owned(),
            });
        }
        if offer.as_ref().is_some_and(|o| !o.checksum) {
            notes.push("The release publishes no checksum for this file".to_owned());
        }
        if offer.is_some() && here.as_ref().is_some_and(|h| !h.by_sangraha) {
            notes.push(
                "Installed without Sangraha; an update replaces that copy where it is".to_owned(),
            );
        }
        let hidden = cfg.hidden_apps.iter().any(|h| h.eq_ignore_ascii_case(&key));
        let own = own_login.is_some_and(|l| {
            key.split('/')
                .next()
                .is_some_and(|o| o.eq_ignore_ascii_case(l))
        });
        rows.push(AppRow {
            key,
            name,
            repo,
            description: app.description.clone(),
            url: app.url.clone(),
            own,
            status,
            offer,
            newest,
            here,
            android,
            notes,
            hidden,
        });
    }
    release_shared_copies(&mut rows);
    rows.sort_by(|a, b| {
        a.status
            .cmp(&b.status)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    let counts = count(&rows);
    AppsBoard {
        checked_at: snap.map(|s| s.fetched_at),
        rate: snap.and_then(|s| s.rate.clone()),
        apps_dir: cfg.apps_dir(),
        rows,
        counts,
        warnings: snap.map(|s| s.warnings.clone()).unwrap_or_default(),
    }
}

/// Two repositories can ship the same app (an old repository and its successor). A copy found
/// on disk belongs to one of them: the one whose version it is, else the newest release.
fn release_shared_copies(rows: &mut [AppRow]) {
    let mut by_path: std::collections::HashMap<PathBuf, Vec<usize>> = Default::default();
    for (i, r) in rows.iter().enumerate() {
        if let Some(h) = r.here.as_ref().filter(|h| !h.by_sangraha) {
            by_path.entry(h.path.clone()).or_default().push(i);
        }
    }
    for idx in by_path.into_values().filter(|v| v.len() > 1) {
        let fits = |i: &usize| {
            let r = &rows[*i];
            let v = r.here.as_ref().and_then(|h| h.version.as_deref());
            (
                matches!((v, &r.offer), (Some(v), Some(o)) if same_version(v, &o.tag)),
                r.offer.as_ref().and_then(|o| o.published_at.clone()),
            )
        };
        let Some(keep) = idx.iter().copied().max_by_key(|i| fits(i)) else {
            continue;
        };
        for i in idx.into_iter().filter(|i| *i != keep) {
            let r = &mut rows[i];
            r.here = None;
            r.notes
                .retain(|n| !n.starts_with("Installed without Sangraha"));
            if r.offer.is_some() {
                r.status = AppStatus::NotInstalled;
            }
        }
    }
}

fn count(rows: &[AppRow]) -> AppCounts {
    let mut c = AppCounts::default();
    for r in rows {
        if r.hidden {
            c.hidden += 1;
            continue;
        }
        c.all += 1;
        if r.here.is_some() {
            c.installed += 1;
        }
        if r.status == AppStatus::Update {
            c.updates += 1;
        }
        if r.status == AppStatus::NotInstalled {
            c.not_installed += 1;
        }
    }
    c
}

/// `SHA256SUMS`, `SHA256SUMS.txt`, or `<asset>.sha256`.
pub fn is_checksum_file(name: &str, asset: &str) -> bool {
    let l = name.to_ascii_lowercase();
    l.starts_with("sha256sums") || l == format!("{}.sha256", asset.to_ascii_lowercase())
}

/// The SHA-256 for `asset` in a checksum file: `<hex>  <name>`, `<hex> *<name>`, or a lone hex.
pub fn sum_for(text: &str, asset: &str) -> Option<String> {
    let hex = |s: &str| s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit());
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    for l in &lines {
        let mut parts = l.split_whitespace();
        let (Some(h), name) = (parts.next(), parts.next()) else {
            continue;
        };
        let name = name.map(|n| n.trim_start_matches('*').rsplit('/').next().unwrap_or(n));
        if hex(h) && name.is_some_and(|n| n == asset) {
            return Some(h.to_ascii_lowercase());
        }
    }
    match lines.as_slice() {
        [one]
            if hex(one.split_whitespace().next().unwrap_or(""))
                && one.split_whitespace().count() == 1 =>
        {
            Some(one.to_ascii_lowercase())
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &str) -> Vec<Asset> {
        list.split(", ")
            .map(|n| Asset {
                name: n.to_owned(),
                size: 1,
                digest: Some("sha256:00".into()),
            })
            .collect()
    }

    fn pick_name(list: &str, os: Os) -> Option<String> {
        pick(&names(list), None, os, Arch::X64).map(|(a, _)| a.name.clone())
    }

    /// File lists copied from real releases.
    #[test]
    fn picks_the_right_file() {
        let flutter = "Brahma-0.3.1-windows-x64-setup.exe, SHA256SUMS.txt, Brahma-0.3.1-linux-x64.tar.gz, Brahma-0.3.1-android.apk, Brahma-0.3.1-windows-x64-portable.zip";
        assert_eq!(
            pick_name(flutter, Os::Linux).as_deref(),
            Some("Brahma-0.3.1-linux-x64.tar.gz")
        );
        assert_eq!(
            pick_name(flutter, Os::Windows).as_deref(),
            Some("Brahma-0.3.1-windows-x64-portable.zip")
        );

        let tauri = "Sachiva_0.5.0_aarch64.apk, Sachiva_0.5.0_amd64.AppImage, Sachiva_0.5.0_amd64.AppImage.sig, Sachiva_0.5.0_x64-setup.exe, Sachiva_0.5.0_x64-setup.exe.sig, Sachiva_0.5.0_x64_portable.zip, sachiva-0.5.0-1-x86_64.pkg.tar.zst.sig, sachiva-0.5.0-1-x86_64.pkg.tar.zst, SHA256SUMS.txt, latest.json";
        assert_eq!(
            pick_name(tauri, Os::Linux).as_deref(),
            Some("Sachiva_0.5.0_amd64.AppImage")
        );
        assert_eq!(
            pick_name(tauri, Os::Windows).as_deref(),
            Some("Sachiva_0.5.0_x64_portable.zip")
        );

        let two = "Graha-Anveshana-0.14.1-linux-x64.tar.gz, Graha-Anveshana-0.14.1-windows-x64-portable.zip, Graha-Anveshana-0.14.1-linux-x64-portable.tar.gz, Graha-Anveshana-0.14.1-windows-x64-setup.exe, Graha-Anveshana-0.14.1-android.apk";
        assert_eq!(
            pick_name(two, Os::Linux).as_deref(),
            Some("Graha-Anveshana-0.14.1-linux-x64-portable.tar.gz")
        );

        let wpe = "Chanda-0.5.1-linux-x64.tar.gz, Chanda-0.5.1-android-x86_64.apk, Chanda-0.5.1-linux-x64-wpe.tar.gz";
        assert_eq!(
            pick_name(wpe, Os::Linux).as_deref(),
            Some("Chanda-0.5.1-linux-x64.tar.gz")
        );

        let sys = "Ganana_0.1.0_amd64.deb, Ganana-0.1.0-1.x86_64.rpm, Ganana_0.1.0_amd64.AppImage, Ganana_0.1.0_x64_en-US.msi, Ganana_0.1.0_x64-setup.exe, Ganana_0.1.0_universal.dmg, Ganana_universal.app.tar.gz";
        assert_eq!(
            pick_name(sys, Os::Linux).as_deref(),
            Some("Ganana_0.1.0_amd64.AppImage")
        );
        assert_eq!(
            pick_name(sys, Os::Windows).as_deref(),
            Some("Ganana_0.1.0_x64-setup.exe")
        );

        let exe = "Shabda-0.1.1-scripts.zip, SHA256SUMS.txt, Shabda-0.1.1-win-x64.exe";
        assert_eq!(
            pick_name(exe, Os::Windows).as_deref(),
            Some("Shabda-0.1.1-win-x64.exe")
        );
        assert_eq!(pick_name(exe, Os::Linux), None);

        // Source archives and browser extensions are not apps.
        assert_eq!(
            pick_name(
                "SHA256SUMS, arogya-v0.5.0.tar.gz, arogya-v0.5.0.zip",
                Os::Linux
            ),
            None
        );
        assert_eq!(
            pick_name(
                "Nireekshana-0.3.1-firefox.zip, Nireekshana-0.3.1-chrome.zip",
                Os::Windows
            ),
            None
        );
        assert!(classify("griha-launcher-0.8.0.apk").is_some_and(|b| b.os == Os::Android));
        assert_eq!(classify("candy-gloss-4.griha-theme"), None);

        // A settings rule wins.
        let list = names(wpe);
        let ruled = pick(&list, Some("*-wpe.tar.gz"), Os::Linux, Arch::X64).unwrap();
        assert_eq!(ruled.0.name, "Chanda-0.5.1-linux-x64-wpe.tar.gz");
    }

    #[test]
    fn names_from_files() {
        assert_eq!(
            display_name("Chanda-0.5.1-linux-x64.tar.gz", "chanda-shopping-assistant"),
            "Chanda"
        );
        assert_eq!(
            display_name("Arogya.Swasthya_0.1.0_amd64.AppImage", "x"),
            "Arogya Swasthya"
        );
        assert_eq!(
            display_name("Graha-Anveshana-0.14.1-linux-x64.tar.gz", "x"),
            "Graha Anveshana"
        );
        assert_eq!(
            display_name("setu-0.1.0-alpha.8-linux-x64.tar.gz", "x"),
            "Setu"
        );
        assert_eq!(display_name("arogya-v0.5.0.tar.gz", "x"), "Arogya");
        assert_eq!(display_name("", "vani-social-media-manager"), "Vani");
        assert_eq!(
            display_name("libmpv-windows-x86_64-20261017.zip", "x"),
            "Libmpv Windows X86 64"
        );
        assert_eq!(app_id("Graha Anveshana"), "graha-anveshana");
    }

    #[test]
    fn version_order() {
        use Ordering::*;
        assert_eq!(compare("v0.5.1", "0.5.1"), Some(Equal));
        assert_eq!(compare("0.1.0-alpha.3", "0.1.0-alpha.10"), Some(Less));
        assert_eq!(compare("0.1.0-alpha.8", "0.1.0-beta.1"), Some(Less));
        assert_eq!(compare("0.1.0", "0.1.0-rc.1"), Some(Greater));
        assert_eq!(compare("1.0", "1.0.0"), Some(Equal));
        assert_eq!(compare("0.10.0", "0.9.9"), Some(Greater));
        assert_eq!(compare("themes-2026.11.01-4", "1.0"), None);
        assert!(same_version("themes-2026.11.01-4", "themes-2026.11.01-4"));
    }

    #[test]
    fn checksum_files() {
        let sums = "abc  other\n";
        assert_eq!(sum_for(sums, "x"), None);
        let h = "a".repeat(64);
        assert_eq!(
            sum_for(
                &format!("{h}  dist/App.tar.gz\n{}  B\n", "b".repeat(64)),
                "App.tar.gz"
            ),
            Some(h.clone())
        );
        assert_eq!(
            sum_for(&format!("{h} *App.zip"), "App.zip"),
            Some(h.clone())
        );
        assert_eq!(
            sum_for(&format!("{}\n", h.to_uppercase()), "App.zip"),
            Some(h.clone())
        );
        assert!(
            is_checksum_file("SHA256SUMS.txt", "a")
                && is_checksum_file("a.tar.gz.sha256", "a.tar.gz")
        );
        assert!(!is_checksum_file("b.sha256", "a"));
    }

    #[test]
    fn globs() {
        assert!(glob("*-wpe.tar.gz", "Chanda-1-linux-x64-WPE.tar.gz"));
        assert!(glob("app*linux*", "app-1.0-linux.tgz"));
        assert!(!glob("app*linux*", "app-1.0-win.zip"));
        assert!(glob("exact.zip", "EXACT.zip"));
    }

    #[test]
    fn names_are_checked() {
        assert!(valid_name("owner/repo.name_1"));
        assert!(!valid_name("owner"));
        assert!(!valid_name("o/r/x"));
        assert!(!valid_name("o/r\") { x }"));
    }
}
