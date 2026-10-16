// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Downloading and staging a newer release
//! (`docs/specs/SPEC_UPGRADE_FOR_USERS_AND_AGENTS_2026_10_15.md` §4, phase 3a).
//!
//! Only what the update check found: the release file for this install
//! (`update_install`), from `github.com/agentmuxai/agentmux/releases`, which
//! must match the SHA-256 GitHub lists for it. Nothing here names a URL or a
//! file of its own, and nothing is installed: a Windows portable release is
//! unpacked into a new folder next to the running one, ready for the launcher
//! to start (phase 3b); a `setup.exe` install's installer is kept, verified,
//! in this version's `updates/<version>/`, for the launcher to run (phase 3d). Progress and the result go out as the
//! `app:updatestatus` event (`update_check`).
//!
//! The download is kept in `updates/<version>/` in this version's data folder
//! (`AGENTMUX_DATA_HOME`, `channels/<ch>/versions/<v>/data`) while it runs and
//! deleted once unpacked. Its file and the unpacking folder carry this
//! process' id, so two AgentMux instances downloading the same release never
//! write over each other; whichever stages the folder first, the other uses. Unpacking uses Windows' own `tar.exe` (libarchive,
//! which reads the release's LZMA2 `.7z`, and refuses `..` and absolute
//! paths), by its full path so no other `tar` on `PATH` is run. A Windows too
//! old for that fails with a message, and the release page still works.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

use crate::backend::update_check::{self, AppUpdateStatus, DownloadStatus, ReleaseAsset};
use crate::backend::update_install::InstallKind;

/// Release files come only from here (`browser_download_url`).
pub(crate) const RELEASE_DOWNLOAD_PREFIX: &str = "https://github.com/agentmuxai/agentmux/releases/download/";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// No answer, or no bytes, for this long ends the download.
const STALL_TIMEOUT: Duration = Duration::from_secs(60);
/// Unpacking takes seconds; this only stops a hung `tar`.
const UNPACK_TIMEOUT: Duration = Duration::from_secs(10 * 60);
/// A download file or unpacking folder untouched this long was left by a
/// run that was cut short (a crash, a quit): a live run writes continuously.
const ABANDONED_AFTER: Duration = Duration::from_secs(60 * 60);

static RUNNING: AtomicBool = AtomicBool::new(false);

/// Hashes and counts a download as it arrives.
struct Received {
    hasher: Sha256,
    bytes: u64,
    expected: u64,
}

impl Received {
    fn new(expected: u64) -> Self {
        Self { hasher: Sha256::new(), bytes: 0, expected }
    }

    fn push(&mut self, chunk: &[u8]) -> Result<(), String> {
        self.bytes += chunk.len() as u64;
        if self.bytes > self.expected {
            return Err("the download is larger than GitHub lists it".into());
        }
        self.hasher.update(chunk);
        Ok(())
    }

    /// Whether what arrived is exactly the file GitHub lists.
    fn finish(self, sha256: &str) -> Result<(), String> {
        if self.bytes != self.expected {
            return Err(format!("the download stopped at {} of {} bytes", self.bytes, self.expected));
        }
        if !hex::encode(self.hasher.finalize()).eq_ignore_ascii_case(sha256) {
            return Err("the download doesn't match the checksum GitHub lists for it".into());
        }
        Ok(())
    }
}

/// What a download leaves ready: a portable's new folder, or an installer.
enum Staged<'a> {
    None,
    Folder(&'a Path),
    Installer(&'a Path),
}

fn set(version: &str, state: &str, received: u64, total: u64, error: Option<String>, staged: Staged) {
    let (folder, installer) = match staged {
        Staged::None => (None, None),
        Staged::Folder(f) => (Some(f), None),
        Staged::Installer(i) => (None, Some(i)),
    };
    update_check::set_download(Some(DownloadStatus {
        version: version.to_string(),
        state: state.to_string(),
        received,
        total,
        error,
        folder: folder.map(|f| f.display().to_string()),
        installer: installer.map(|i| i.display().to_string()),
        waiting_for: 0,
    }));
}

/// Start downloading the release the update check found (`app.update-download`).
/// Returns at once with the status; progress follows as events.
pub fn start() -> Result<AppUpdateStatus, String> {
    let status = update_check::current_status();
    let (Some(version), Some(asset)) = (status.latest.clone(), status.asset.clone()) else {
        return Err("no newer release is known yet".into());
    };
    if status.state != "available" || !status.can_download {
        return Err("AgentMux can't download this release itself; use the release page".into());
    }
    // A portable's new folder goes beside the running one; a setup.exe
    // install's installer is kept as is.
    let portable_root = match update_check::install_kind() {
        InstallKind::WindowsPortable { root } => Some(root),
        InstallKind::WindowsSetup => None,
        _ => return Err("AgentMux can't download this release itself; use the release page".into()),
    };
    if let Some(d) = &status.download {
        let staged = d.folder.as_deref().is_some_and(|f| is_staged_portable(Path::new(f), &version))
            || d.installer.as_deref().is_some_and(|i| is_staged_installer(Path::new(i), &asset));
        if d.version == version && matches!(d.state.as_str(), "ready" | "waiting" | "restarting") && staged {
            return Ok(status);
        }
    }
    if RUNNING.swap(true, Ordering::SeqCst) {
        return Ok(status);
    }
    set(&version, "downloading", 0, asset.size, None, Staged::None);
    tokio::spawn(async move {
        let result = match &portable_root {
            Some(root) => run(&asset, &version, root).await.map(|f| (f, true)),
            None => run_installer(&asset, &version).await.map(|i| (i, false)),
        };
        RUNNING.store(false, Ordering::SeqCst);
        match result {
            Ok((path, folder)) => {
                tracing::info!(path = %path.display(), "update: {version} is ready");
                let staged = if folder { Staged::Folder(&path) } else { Staged::Installer(&path) };
                set(&version, "ready", asset.size, asset.size, None, staged);
            }
            Err(e) => {
                tracing::warn!(error = %e, "update: downloading {version} failed");
                set(&version, "failed", 0, asset.size, Some(e), Staged::None);
            }
        }
    });
    Ok(update_check::current_status())
}

async fn run(asset: &ReleaseAsset, version: &str, root: &Path) -> Result<PathBuf, String> {
    let updates = crate::backend::base::get_mux_data_dir().join("updates");
    remove_abandoned_downloads(&updates).await;
    let dir = updates.join(version);
    tokio::fs::create_dir_all(&dir).await.map_err(|e| format!("can't create {}: {e}", dir.display()))?;
    let file = dir.join(format!("{}-{}", std::process::id(), asset.name));
    if let Err(e) = download(asset, &file, version).await {
        let _ = tokio::fs::remove_file(&file).await;
        return Err(e);
    }
    set(version, "unpacking", asset.size, asset.size, None, Staged::None);
    // Next to the running folder, so it's where the user expects; under
    // this version's `updates/` when that folder's parent can't be written.
    let mut last = String::new();
    for parent in [root.parent().map(Path::to_path_buf), Some(dir.clone())].into_iter().flatten() {
        match unpack_portable(&file, version, &parent).await {
            Ok(folder) => {
                let _ = tokio::fs::remove_file(&file).await;
                return Ok(folder);
            }
            Err(e) => {
                tracing::info!(error = %e, parent = %parent.display(), "update: couldn't unpack here");
                last = e;
            }
        }
    }
    // Nothing more can be done with it; a later download fetches it again.
    let _ = tokio::fs::remove_file(&file).await;
    Err(last)
}

/// The installer's name in `updates/<version>/` once verified: the release's.
fn installer_path(dir: &Path, asset: &ReleaseAsset) -> PathBuf {
    dir.join(&asset.name)
}

/// Whether `path` is that release's installer, whole.
fn is_staged_installer(path: &Path, asset: &ReleaseAsset) -> bool {
    path.file_name().is_some_and(|n| n == asset.name.as_str())
        && std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.len() == asset.size)
}

/// Download a `setup.exe` install's installer into `updates/<version>/`,
/// verified, under the release's own name: what the launcher runs.
async fn run_installer(asset: &ReleaseAsset, version: &str) -> Result<PathBuf, String> {
    let updates = crate::backend::base::get_mux_data_dir().join("updates");
    remove_abandoned_downloads(&updates).await;
    let dir = updates.join(version);
    tokio::fs::create_dir_all(&dir).await.map_err(|e| format!("can't create {}: {e}", dir.display()))?;
    let target = installer_path(&dir, asset);
    // Downloaded before, or by another AgentMux meanwhile.
    if is_staged_installer(&target, asset) {
        return Ok(target);
    }
    let file = dir.join(format!("{}-{}", std::process::id(), asset.name));
    if let Err(e) = download(asset, &file, version).await {
        let _ = tokio::fs::remove_file(&file).await;
        return Err(e);
    }
    match tokio::fs::rename(&file, &target).await {
        Ok(()) => Ok(target),
        Err(_) if is_staged_installer(&target, asset) => {
            let _ = tokio::fs::remove_file(&file).await;
            Ok(target)
        }
        Err(e) => {
            let _ = tokio::fs::remove_file(&file).await;
            Err(format!("can't put the installer in place: {e}"))
        }
    }
}

async fn download(asset: &ReleaseAsset, dest: &Path, version: &str) -> Result<(), String> {
    if !asset.url.starts_with(RELEASE_DOWNLOAD_PREFIX) {
        return Err("the release file isn't on AgentMux's GitHub releases".into());
    }
    let client = reqwest::Client::builder()
        .user_agent(format!("AgentMux/{}", env!("CARGO_PKG_VERSION")))
        .connect_timeout(CONNECT_TIMEOUT)
        .build()
        .map_err(|e| e.to_string())?;
    let mut resp = tokio::time::timeout(STALL_TIMEOUT, client.get(&asset.url).send())
        .await
        .map_err(|_| "GitHub didn't answer".to_string())?
        .and_then(|r| r.error_for_status())
        .map_err(|e| format!("download failed: {e}"))?;
    let mut part = dest.as_os_str().to_owned();
    part.push(".part");
    let part = PathBuf::from(part);
    let result = async {
        let mut out = tokio::fs::File::create(&part).await.map_err(|e| e.to_string())?;
        let mut received = Received::new(asset.size);
        let mut shown = u64::MAX;
        loop {
            let chunk = tokio::time::timeout(STALL_TIMEOUT, resp.chunk())
                .await
                .map_err(|_| "the download stalled".to_string())?
                .map_err(|e| format!("download failed: {e}"))?;
            let Some(chunk) = chunk else { break };
            received.push(&chunk)?;
            out.write_all(&chunk).await.map_err(|e| e.to_string())?;
            let percent = received.bytes * 100 / asset.size.max(1);
            if percent != shown {
                shown = percent;
                set(version, "downloading", received.bytes, asset.size, None, Staged::None);
            }
        }
        out.flush().await.map_err(|e| e.to_string())?;
        drop(out);
        received.finish(&asset.sha256)?;
        tokio::fs::rename(&part, dest).await.map_err(|e| e.to_string())
    }
    .await;
    if result.is_err() {
        let _ = tokio::fs::remove_file(&part).await;
    }
    result
}

/// Whether `dir` is an unpacked AgentMux `version` portable.
pub(crate) fn is_staged_portable(dir: &Path, version: &str) -> bool {
    dir.join("agentmux.exe").is_file()
        && dir.join("runtime").join("agentmux-portable.marker").is_file()
        && dir.join("runtime").join(format!("agentmux-{version}.exe")).is_file()
}

/// `parent/name`, or `name-2`, `name-3`… when that is taken by something else.
fn free_folder(parent: &Path, name: &str) -> PathBuf {
    let mut n = 1;
    loop {
        let candidate = if n == 1 { parent.join(name) } else { parent.join(format!("{name}-{n}")) };
        if !candidate.exists() {
            return candidate;
        }
        n += 1;
    }
}

/// Unpacking folders an earlier, interrupted run left behind (an hour old:
/// never one another AgentMux is still filling).
async fn remove_stale_partials(parent: &Path, name: &str) {
    let prefix = format!(".{name}.");
    let Ok(mut entries) = tokio::fs::read_dir(parent).await else { return };
    while let Ok(Some(e)) = entries.next_entry().await {
        let file_name = e.file_name().to_string_lossy().into_owned();
        if file_name.starts_with(&prefix) && file_name.ends_with(".partial") && abandoned(&e).await {
            let _ = tokio::fs::remove_dir_all(e.path()).await;
        }
    }
}

async fn abandoned(e: &tokio::fs::DirEntry) -> bool {
    e.metadata()
        .await
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.elapsed().ok())
        .is_some_and(|age| age > ABANDONED_AFTER)
}

/// Download files (each run's `<pid>-<file>` and its `.part`) left in
/// `updates/<version>/` by runs that were cut short. Only those: a verified
/// installer waiting there for its restart is kept, and so are folders (a
/// release staged there when its portable's folder couldn't be written;
/// unpacking folders are `remove_stale_partials`' job).
async fn remove_abandoned_downloads(updates: &Path) {
    let pid_prefixed = |name: &str| name.split_once('-').is_some_and(|(pid, _)| !pid.is_empty() && pid.bytes().all(|b| b.is_ascii_digit()));
    let Ok(mut versions) = tokio::fs::read_dir(updates).await else { return };
    while let Ok(Some(v)) = versions.next_entry().await {
        let Ok(mut files) = tokio::fs::read_dir(v.path()).await else { continue };
        while let Ok(Some(f)) = files.next_entry().await {
            let named = pid_prefixed(&f.file_name().to_string_lossy());
            if named && f.file_type().await.is_ok_and(|t| t.is_file()) && abandoned(&f).await {
                let _ = tokio::fs::remove_file(f.path()).await;
            }
        }
    }
}

fn tar_exe() -> PathBuf {
    if cfg!(windows) {
        let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
        PathBuf::from(root).join("System32").join("tar.exe")
    } else {
        PathBuf::from("tar")
    }
}

/// Unpack the portable archive `file` into a new folder in `parent`.
async fn unpack_portable(file: &Path, version: &str, parent: &Path) -> Result<PathBuf, String> {
    let name = format!("agentmux-{version}-x64-portable");
    // Staged by an earlier download that was then interrupted.
    if is_staged_portable(&parent.join(&name), version) {
        return Ok(parent.join(&name));
    }
    remove_stale_partials(parent, &name).await;
    let partial = parent.join(format!(".{name}.{}.partial", std::process::id()));
    let _ = tokio::fs::remove_dir_all(&partial).await;
    tokio::fs::create_dir_all(&partial).await.map_err(|e| format!("can't write to {}: {e}", parent.display()))?;
    use agentmux_common::win32::NoWindow;
    let mut tar = tokio::process::Command::new(tar_exe());
    // tar needs nothing of this instance: none of its AGENTMUX_* variables.
    crate::backend::pane_env::sanitize_external_command(&mut tar);
    let run = tar
        .arg("-xf")
        .arg(file)
        .arg("-C")
        .arg(&partial)
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true)
        .no_window()
        .output();
    let checked = match tokio::time::timeout(UNPACK_TIMEOUT, run).await {
        Err(_) => Err("unpacking took more than 10 minutes and was stopped".to_string()),
        Ok(Err(e)) => Err(format!("couldn't run tar to unpack it: {e}")),
        Ok(Ok(o)) if !o.status.success() => {
            let why = String::from_utf8_lossy(&o.stderr).lines().next().unwrap_or("").to_string();
            Err(format!("tar couldn't unpack it ({why}); this Windows may be too old to read .7z files"))
        }
        Ok(Ok(_)) if !is_staged_portable(&partial, version) => {
            Err(format!("what was unpacked isn't an AgentMux {version} portable"))
        }
        Ok(Ok(_)) => Ok(()),
    };
    let canonical = parent.join(&name);
    let result = match checked {
        Err(e) => Err(e),
        // Another AgentMux staged the same release while this one unpacked:
        // use it rather than make a copy beside it.
        Ok(()) if is_staged_portable(&canonical, version) => Ok(canonical),
        Ok(()) => {
            let target = free_folder(parent, &name);
            match tokio::fs::rename(&partial, &target).await {
                Ok(()) => Ok(target),
                // The other one's move won the race.
                Err(_) if is_staged_portable(&canonical, version) => Ok(canonical),
                Err(e) => Err(format!("can't move the unpacked folder into place: {e}")),
            }
        }
    };
    // Whatever wasn't moved into place.
    let _ = tokio::fs::remove_dir_all(&partial).await;
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_exact_file_github_lists_is_accepted() {
        let data = b"agentmux release bytes";
        let sha = hex::encode(Sha256::digest(data));
        let mut r = Received::new(data.len() as u64);
        r.push(&data[..5]).unwrap();
        r.push(&data[5..]).unwrap();
        r.finish(&sha.to_uppercase()).unwrap();

        let mut short = Received::new(data.len() as u64 + 1);
        short.push(data).unwrap();
        assert!(short.finish(&sha).unwrap_err().contains("stopped at"));

        let mut long = Received::new(4);
        assert!(long.push(data).unwrap_err().contains("larger"));

        let mut other = Received::new(data.len() as u64);
        other.push(b"agentmux release bytez").unwrap();
        assert!(other.finish(&sha).unwrap_err().contains("checksum"));
    }

    fn fake_portable(dir: &Path, version: &str) {
        std::fs::create_dir_all(dir.join("runtime")).unwrap();
        std::fs::write(dir.join("agentmux.exe"), b"").unwrap();
        std::fs::write(dir.join("runtime/agentmux-portable.marker"), b"").unwrap();
        std::fs::write(dir.join(format!("runtime/agentmux-{version}.exe")), b"").unwrap();
    }

    #[tokio::test]
    async fn downloads_left_by_an_interrupted_run_are_removed_and_folders_kept() {
        let updates = tempfile::tempdir().unwrap();
        let dir = updates.path().join("0.59.27");
        std::fs::create_dir_all(dir.join("agentmux-0.59.27-x64-portable")).unwrap();
        // A verified installer waits here for its restart, however long.
        let installer = dir.join("AgentMux-0.59.27-x64-setup.exe");
        std::fs::write(&installer, b"x").unwrap();
        let age = |p: &Path, secs: u64| {
            let t = std::time::SystemTime::now() - Duration::from_secs(secs);
            std::fs::File::options().write(true).open(p).unwrap().set_modified(t).unwrap();
        };
        let old = dir.join("4242-agentmux-0.59.27-x64-portable.7z.part");
        let live = dir.join("777-agentmux-0.59.27-x64-portable.7z.part");
        std::fs::write(&old, b"x").unwrap();
        std::fs::write(&live, b"x").unwrap();
        age(&old, 2 * 60 * 60);
        age(&live, 5);
        age(&installer, 2 * 60 * 60);
        remove_abandoned_downloads(updates.path()).await;
        assert!(!old.exists());
        assert!(live.exists());
        assert!(installer.exists());
        assert!(dir.join("agentmux-0.59.27-x64-portable").is_dir());
    }

    #[test]
    fn a_staged_folder_must_be_that_version_and_never_overwrites_another() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("agentmux-0.59.27-x64-portable");
        fake_portable(&folder, "0.59.27");
        assert!(is_staged_portable(&folder, "0.59.27"));
        assert!(!is_staged_portable(&folder, "0.59.28"));
        assert_eq!(free_folder(dir.path(), "agentmux-0.59.27-x64-portable"), dir.path().join("agentmux-0.59.27-x64-portable-2"));
        assert_eq!(free_folder(dir.path(), "other"), dir.path().join("other"));
    }

    /// The real thing, against a published release (125 MB; run by hand):
    /// `cargo test -p agentmux-srv --bin agentmux-srv -- --ignored real_portable_release`.
    #[cfg(windows)]
    #[tokio::test]
    #[ignore]
    async fn real_portable_release_downloads_verifies_and_unpacks() {
        let name = "agentmux-0.59.27-x64-portable.7z";
        let asset = ReleaseAsset {
            name: name.into(),
            size: 124_705_866,
            url: format!("{RELEASE_DOWNLOAD_PREFIX}v0.59.27/{name}"),
            sha256: "db750e1689752519ad21b730436f783493c63b59ad734677a433e413c9b47f2a".into(),
        };
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(name);
        download(&asset, &file, "0.59.27").await.unwrap();
        let folder = unpack_portable(&file, "0.59.27", dir.path()).await.unwrap();
        assert!(is_staged_portable(&folder, "0.59.27"));
        // One wrong byte of checksum and the download is refused and removed.
        let wrong = ReleaseAsset { sha256: "0".repeat(64), ..asset };
        let err = download(&wrong, &dir.path().join("again.7z"), "0.59.27").await.unwrap_err();
        assert!(err.contains("checksum"), "{err}");
        assert!(!dir.path().join("again.7z.part").exists());
    }

    /// Windows' tar.exe reads the archive and the folder lands in place;
    /// a zip stands in for the release `.7z` (both go through libarchive).
    #[cfg(windows)]
    #[tokio::test]
    async fn unpacks_with_windows_tar_and_checks_what_came_out() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        fake_portable(&src, "0.59.27");
        let archive = dir.path().join("release.zip");
        let made = std::process::Command::new(tar_exe())
            .args(["-a", "-cf"])
            .arg(&archive)
            .arg("-C")
            .arg(&src)
            .arg(".")
            .status()
            .unwrap();
        assert!(made.success());
        let parent = dir.path().join("apps");
        std::fs::create_dir_all(&parent).unwrap();
        let folder = unpack_portable(&archive, "0.59.27", &parent).await.unwrap();
        assert_eq!(folder, parent.join("agentmux-0.59.27-x64-portable"));
        assert!(is_staged_portable(&folder, "0.59.27"));
        let leftovers = || std::fs::read_dir(&parent).unwrap().filter(|e| e.as_ref().unwrap().file_name().to_string_lossy().starts_with('.')).count();
        assert_eq!(leftovers(), 0);
        // Asked again, the staged folder is reused.
        assert_eq!(unpack_portable(&archive, "0.59.27", &parent).await.unwrap(), folder);
        // An archive of another version is refused and leaves nothing.
        let err = unpack_portable(&archive, "0.59.28", &parent).await.unwrap_err();
        assert!(err.contains("isn't an AgentMux 0.59.28 portable"), "{err}");
        assert_eq!(leftovers(), 0);
        assert!(!parent.join("agentmux-0.59.28-x64-portable").exists());
    }
}
