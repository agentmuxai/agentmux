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
//! in this version's `updates/<version>/`, for the launcher to run (phase 3d);
//! a Linux portable's tarball is unpacked into `AgentMux-<version>` next to the
//! running folder, and an AppImage's next version is put beside it, executable
//! (phase 3e). Progress and the result go out as the `app:updatestatus` event
//! (`update_check`).
//!
//! The download is kept in `updates/<version>/` in this version's data folder
//! (`AGENTMUX_DATA_HOME`, `channels/<ch>/versions/<v>/data`) while it runs and
//! deleted once unpacked. Its file and the unpacking folder carry this
//! process' id, so two AgentMux instances downloading the same release never
//! write over each other; whichever stages the folder first, the other uses.
//! On Windows unpacking uses its own `tar.exe` (libarchive, which reads the
//! release's LZMA2 `.7z`, and refuses `..` and absolute paths), by its full
//! path so no other `tar` on `PATH` is run; a Windows too old for that fails
//! with a message, and the release page still works. On Linux it is the
//! system's `tar` (with `xz` for the `.tar.xz`).

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

/// Record where the download of `version` stands; `false` when that release
/// is no longer the one on offer (`update_check::set_download`).
fn set(version: &str, state: &str, received: u64, total: u64, error: Option<String>, staged: Staged) -> bool {
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
    }))
}

/// What a download stages, by install.
enum Job {
    /// A new portable folder beside `root`'s.
    Portable(PathBuf),
    /// A `setup.exe` installer in `updates/<version>/`.
    Installer,
    /// The next AppImage beside this one.
    AppImage(PathBuf),
}

/// Start downloading the release the update check found (the user's Download,
/// through the host).
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
    // install's installer is kept as is; an AppImage's next one goes beside it.
    let job = match update_check::install_kind() {
        InstallKind::WindowsPortable { root } | InstallKind::LinuxPortable { root } => Job::Portable(root),
        InstallKind::WindowsSetup => Job::Installer,
        InstallKind::LinuxAppImage => match std::env::current_exe()
            .ok()
            .and_then(|exe| agentmux_common::runtime_mode::running_appimage(&exe))
        {
            Some(appimage) => Job::AppImage(appimage),
            None => return Err("can't tell which AppImage this AgentMux runs from; use the release page".into()),
        },
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
    // A check that found a newer release since `status` was read wins: start
    // nothing for this one (the user downloads the newer one instead).
    if !set(&version, "downloading", 0, asset.size, None, Staged::None) {
        RUNNING.store(false, Ordering::SeqCst);
        return Err(format!("a newer release than {version} was found just now; download that one"));
    }
    tokio::spawn(async move {
        let result = match &job {
            Job::Portable(root) => run(&asset, &version, root).await.map(|f| (f, true)),
            Job::Installer => run_installer(&asset, &version).await.map(|i| (i, false)),
            Job::AppImage(appimage) => run_appimage(&asset, &version, appimage).await.map(|i| (i, false)),
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

/// `updates/` in this version's data folder: `AGENTMUX_DATA_DIR`, the very
/// folder the launcher checks a staged installer against
/// (`crates/launcher/src/upgrade_restart.rs`), not a root of srv's own.
fn updates_root() -> PathBuf {
    agentmux_common::DataPaths::from_env()
        .map(|p| p.data_dir)
        .unwrap_or_else(crate::backend::base::get_mux_data_dir)
        .join("updates")
}

async fn run(asset: &ReleaseAsset, version: &str, root: &Path) -> Result<PathBuf, String> {
    let updates = updates_root();
    remove_abandoned_downloads(&updates).await;
    let dir = updates.join(version);
    tokio::fs::create_dir_all(&dir).await.map_err(|e| format!("can't create {}: {e}", dir.display()))?;
    let file = dir.join(format!("{}-{}", std::process::id(), asset.name));
    if let Err(e) = download(asset, &file, version).await {
        let _ = tokio::fs::remove_file(&file).await;
        return Err(e);
    }
    set(version, "unpacking", asset.size, asset.size, None, Staged::None);
    // The new folder keeps this AgentMux's data channel, a local build's too.
    let channel = crate::backend::reactive::registry::local_channel_id();
    // Next to the running folder, so it's where the user expects; under
    // this version's `updates/` when that folder's parent can't be written.
    let mut last = String::new();
    for parent in [root.parent().map(Path::to_path_buf), Some(dir.clone())].into_iter().flatten() {
        match unpack_portable(&file, version, &parent, &channel).await {
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

/// `name`, or with `-n` before its extension for n > 1
/// (`AgentMux_0.59.30_amd64-2.AppImage`): a copy beside one that isn't ours.
fn release_file_variant(name: &str, n: u32) -> String {
    match (n, name.rsplit_once('.')) {
        (1, _) => name.to_string(),
        (_, Some((stem, ext))) => format!("{stem}-{n}.{ext}"),
        (_, None) => format!("{name}-{n}"),
    }
}

/// Whether `path` is that release's installer (or AppImage), whole.
fn is_staged_installer(path: &Path, asset: &ReleaseAsset) -> bool {
    path.file_name().is_some_and(|n| (1..100).any(|i| n.to_string_lossy() == release_file_variant(&asset.name, i)))
        && std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.len() == asset.size)
}

/// Whether the file at `path` hashes to the release's digest. A file found
/// already in place is only reused if it does: the name and size alone say
/// nothing about who wrote it.
fn has_release_digest(path: &Path, asset: &ReleaseAsset) -> bool {
    let Ok(mut file) = std::fs::File::open(path) else { return false };
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher).is_ok() && hex::encode(hasher.finalize()).eq_ignore_ascii_case(&asset.sha256)
}

/// [`is_staged_installer`], and whole by its digest too; off the async threads.
async fn is_verified_installer(path: &Path, asset: &ReleaseAsset) -> bool {
    if !is_staged_installer(path, asset) {
        return false;
    }
    let (path, asset) = (path.to_path_buf(), asset.clone());
    tokio::task::spawn_blocking(move || has_release_digest(&path, &asset)).await.unwrap_or(false)
}

/// Download a `setup.exe` install's installer into `updates/<version>/`,
/// verified, under the release's own name: what the launcher runs.
async fn run_installer(asset: &ReleaseAsset, version: &str) -> Result<PathBuf, String> {
    let updates = updates_root();
    remove_abandoned_downloads(&updates).await;
    fetch_file(asset, version, &updates.join(version), None).await
}

/// Put the next AppImage beside this one, or in `updates/<version>/` when that
/// folder can't be written; executable, with this AgentMux's channel recorded
/// beside it, so the new version opens this one's data.
async fn run_appimage(asset: &ReleaseAsset, version: &str, appimage: &Path) -> Result<PathBuf, String> {
    let updates = updates_root();
    remove_abandoned_downloads(&updates).await;
    let channel = crate::backend::reactive::registry::local_channel_id();
    let mut last = String::new();
    for dir in [appimage.parent().map(Path::to_path_buf), Some(updates.join(version))].into_iter().flatten() {
        match fetch_file(asset, version, &dir, Some(&channel)).await {
            Ok(file) => return Ok(file),
            Err(e) => {
                tracing::info!(error = %e, dir = %dir.display(), "update: couldn't put the AppImage here");
                last = e;
            }
        }
    }
    Err(last)
}

/// The data channel recorded beside the AppImage at `file`, if any.
fn appimage_channel(file: &Path) -> Option<String> {
    let record = agentmux_common::runtime_mode::appimage_channel_file(file);
    std::fs::read_to_string(record).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

/// An AppImage at `file`: executable, its channel recorded beside it.
fn finish_appimage(file: &Path, channel: &str) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| format!("can't make {} executable: {e}", file.display()))?;
    }
    let record = agentmux_common::runtime_mode::appimage_channel_file(file);
    std::fs::write(&record, format!("{channel}\n")).map_err(|e| format!("can't write {}: {e}", record.display()))
}

/// Download the release file into `dir` under the release's own name,
/// verified. With `appimage_channel` it is an AppImage: made executable with
/// that channel recorded beside it, and never one recorded for another channel
/// (a copy `…-2.AppImage` is made beside it instead).
async fn fetch_file(asset: &ReleaseAsset, version: &str, dir: &Path, appimage_channel_of: Option<&str>) -> Result<PathBuf, String> {
    tokio::fs::create_dir_all(dir).await.map_err(|e| format!("can't create {}: {e}", dir.display()))?;
    let ours = |p: &Path| appimage_channel_of.is_none_or(|ch| appimage_channel(p).is_none_or(|c| c == ch));
    let target = (1..100)
        .map(|n| dir.join(release_file_variant(&asset.name, n)))
        .find(|p| !p.exists() || ours(p))
        .unwrap_or_else(|| installer_path(dir, asset));
    let finish = |file: PathBuf| match appimage_channel_of {
        Some(channel) => finish_appimage(&file, channel).map(|()| file),
        None => Ok(file),
    };
    // Downloaded before, or by another AgentMux meanwhile.
    if is_verified_installer(&target, asset).await {
        return finish(target);
    }
    let file = dir.join(format!("{}-{}", std::process::id(), asset.name));
    if let Err(e) = download(asset, &file, version).await {
        let _ = tokio::fs::remove_file(&file).await;
        return Err(e);
    }
    match tokio::fs::rename(&file, &target).await {
        Ok(()) => finish(target),
        Err(_) if is_verified_installer(&target, asset).await => {
            let _ = tokio::fs::remove_file(&file).await;
            finish(target)
        }
        Err(e) => {
            let _ = tokio::fs::remove_file(&file).await;
            Err(format!("can't put the release file in place: {e}"))
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

/// Whether `dir` is an unpacked AgentMux `version` portable: on Windows the
/// release's folder (`agentmux.exe`, `runtime/` with the marker and that
/// version's host); on Linux the tarball's `AgentMux/` (`agentmux.sh`, that
/// version's srv in `usr/bin/`, and `usr/share/agentmux/VERSION` saying so).
pub(crate) fn is_staged_portable(dir: &Path, version: &str) -> bool {
    if cfg!(windows) {
        dir.join("agentmux.exe").is_file()
            && dir.join("runtime").join("agentmux-portable.marker").is_file()
            && dir.join("runtime").join(format!("agentmux-{version}.exe")).is_file()
    } else {
        dir.join("agentmux.sh").is_file()
            && dir.join("usr/bin").join(format!("agentmux-srv-{version}-linux.x64")).is_file()
            && std::fs::read_to_string(dir.join("usr/share/agentmux/VERSION")).is_ok_and(|v| v.trim() == version)
    }
}

/// A staged portable of `version` at `dir`, laid out as this platform's
/// release is (tests).
#[cfg(test)]
pub(crate) fn fake_portable(dir: &Path, version: &str) {
    if cfg!(windows) {
        std::fs::create_dir_all(dir.join("runtime")).unwrap();
        std::fs::write(dir.join("agentmux.exe"), b"").unwrap();
        std::fs::write(dir.join("runtime/agentmux-portable.marker"), b"").unwrap();
        std::fs::write(dir.join(format!("runtime/agentmux-{version}.exe")), b"").unwrap();
    } else {
        std::fs::create_dir_all(dir.join("usr/bin")).unwrap();
        std::fs::create_dir_all(dir.join("usr/share/agentmux")).unwrap();
        std::fs::write(dir.join("agentmux.sh"), b"").unwrap();
        std::fs::write(dir.join(format!("usr/bin/agentmux-srv-{version}-linux.x64")), b"").unwrap();
        std::fs::write(dir.join("usr/share/agentmux/VERSION"), format!("{version}
")).unwrap();
    }
}

/// The folder a release's portable is unpacked into, beside the running one.
fn portable_folder_name(version: &str) -> String {
    if cfg!(windows) {
        format!("agentmux-{version}-x64-portable")
    } else {
        format!("AgentMux-{version}")
    }
}

/// Where in what `tar` unpacked into `partial` the portable is: the Windows
/// archive holds the folder's contents, the Linux tarball an `AgentMux/` folder.
fn unpacked_root(partial: &Path) -> PathBuf {
    if cfg!(windows) {
        partial.to_path_buf()
    } else {
        partial.join("AgentMux")
    }
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

/// Whether the staged portable `folder` may be used by the AgentMux on
/// `channel`: it records no channel yet, or that one. A folder another
/// AgentMux staged for a different channel isn't reused; a copy is made beside it.
fn for_channel(folder: &Path, channel: &str) -> bool {
    agentmux_common::runtime_mode::recorded_channel_in(folder).is_none_or(|c| c == channel)
}

/// Record `channel` in the portable `folder`: the data the new version opens.
/// In `runtime/` on Windows, at the top on Linux, where its binaries look
/// (`runtime_mode::recorded_channel_in`).
fn record_channel(folder: &Path, channel: &str) -> Result<(), String> {
    let at = if cfg!(windows) { folder.join("runtime") } else { folder.to_path_buf() };
    let file = at.join(agentmux_common::runtime_mode::RECORDED_CHANNEL_FILE);
    std::fs::write(&file, format!("{channel}\n")).map_err(|e| format!("can't write {}: {e}", file.display()))
}

/// Unpack the release into `parent`, recording `channel` in it.
async fn unpack_portable(file: &Path, version: &str, parent: &Path, channel: &str) -> Result<PathBuf, String> {
    let name = portable_folder_name(version);
    let fits = |dir: &Path| is_staged_portable(dir, version) && for_channel(dir, channel);
    // Staged by an earlier download that was then interrupted.
    if fits(&parent.join(&name)) {
        record_channel(&parent.join(&name), channel)?;
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
            let hint = if cfg!(windows) { "this Windows may be too old to read .7z files" } else { "is `xz` installed?" };
            Err(format!("tar couldn't unpack it ({why}); {hint}"))
        }
        Ok(Ok(_)) if !is_staged_portable(&unpacked_root(&partial), version) => {
            Err(format!("what was unpacked isn't an AgentMux {version} portable"))
        }
        Ok(Ok(_)) => record_channel(&unpacked_root(&partial), channel),
    };
    let canonical = parent.join(&name);
    let result = match checked {
        Err(e) => Err(e),
        // Another AgentMux on this channel staged the same release while this
        // one unpacked: use it rather than make a copy beside it.
        Ok(()) if fits(&canonical) => record_channel(&canonical, channel).map(|()| canonical),
        Ok(()) => {
            let target = free_folder(parent, &name);
            match tokio::fs::rename(unpacked_root(&partial), &target).await {
                Ok(()) => Ok(target),
                // The other one's move won the race.
                Err(_) if fits(&canonical) => record_channel(&canonical, channel).map(|()| canonical),
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
    fn an_installer_found_in_place_is_reused_only_if_it_hashes_right() {
        let dir = tempfile::tempdir().unwrap();
        let data = b"agentmux installer bytes";
        let asset = ReleaseAsset {
            name: "AgentMux-0.59.27-x64-setup.exe".into(),
            size: data.len() as u64,
            url: String::new(),
            sha256: hex::encode(Sha256::digest(data)),
        };
        let path = dir.path().join(&asset.name);
        std::fs::write(&path, data).unwrap();
        assert!(is_staged_installer(&path, &asset) && has_release_digest(&path, &asset));
        // The same name and size, other bytes.
        std::fs::write(&path, b"agentmux installer BYTES").unwrap();
        assert!(is_staged_installer(&path, &asset) && !has_release_digest(&path, &asset));
        assert!(!has_release_digest(&dir.path().join("missing.exe"), &asset));
    }

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
        let folder = unpack_portable(&file, "0.59.27", dir.path(), "stable").await.unwrap();
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
        let local = "local-main-b28b7a-f62415cd";
        let folder = unpack_portable(&archive, "0.59.27", &parent, local).await.unwrap();
        assert_eq!(folder, parent.join("agentmux-0.59.27-x64-portable"));
        assert!(is_staged_portable(&folder, "0.59.27"));
        // It opens this AgentMux's data, a local build's channel included.
        assert_eq!(agentmux_common::runtime_mode::recorded_channel_in(&folder).as_deref(), Some(local));
        let leftovers = || std::fs::read_dir(&parent).unwrap().filter(|e| e.as_ref().unwrap().file_name().to_string_lossy().starts_with('.')).count();
        assert_eq!(leftovers(), 0);
        // Asked again, the staged folder is reused.
        assert_eq!(unpack_portable(&archive, "0.59.27", &parent, local).await.unwrap(), folder);
        // Not by an AgentMux on another channel: it gets a copy of its own.
        let other = unpack_portable(&archive, "0.59.27", &parent, "stable").await.unwrap();
        assert_eq!(other, parent.join("agentmux-0.59.27-x64-portable-2"));
        assert_eq!(agentmux_common::runtime_mode::recorded_channel_in(&other).as_deref(), Some("stable"));
        assert_eq!(agentmux_common::runtime_mode::recorded_channel_in(&folder).as_deref(), Some(local));
        // An archive of another version is refused and leaves nothing.
        let err = unpack_portable(&archive, "0.59.28", &parent, local).await.unwrap_err();
        assert!(err.contains("isn't an AgentMux 0.59.28 portable"), "{err}");
        assert_eq!(leftovers(), 0);
        assert!(!parent.join("agentmux-0.59.28-x64-portable").exists());
    }

    #[test]
    fn a_copy_beside_a_release_file_keeps_its_extension() {
        assert_eq!(release_file_variant("AgentMux_0.59.30_amd64.AppImage", 1), "AgentMux_0.59.30_amd64.AppImage");
        assert_eq!(release_file_variant("AgentMux_0.59.30_amd64.AppImage", 2), "AgentMux_0.59.30_amd64-2.AppImage");
        let dir = tempfile::tempdir().unwrap();
        let asset = ReleaseAsset { name: "AgentMux_0.59.30_amd64.AppImage".into(), size: 3, url: String::new(), sha256: String::new() };
        let copy = dir.path().join("AgentMux_0.59.30_amd64-2.AppImage");
        std::fs::write(&copy, b"abc").unwrap();
        assert!(is_staged_installer(&copy, &asset));
        let other = dir.path().join("AgentMux_0.59.31_amd64.AppImage");
        std::fs::write(&other, b"abc").unwrap();
        assert!(!is_staged_installer(&other, &asset));
    }

    #[test]
    fn an_appimage_is_made_executable_with_its_channel_beside_it() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("AgentMux_0.59.30_amd64.AppImage");
        std::fs::write(&file, b"x").unwrap();
        assert_eq!(appimage_channel(&file), None);
        finish_appimage(&file, "local-main-b28b7a-f62415cd").unwrap();
        assert_eq!(appimage_channel(&file).as_deref(), Some("local-main-b28b7a-f62415cd"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o755);
        }
    }

    /// The system `tar` unpacks a release-shaped tarball (`AgentMux/` with
    /// `agentmux.sh` above `usr/bin/`) into `AgentMux-<v>` beside the running
    /// folder, recording the channel at its top.
    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn unpacks_a_linux_tarball_beside_the_running_folder() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        let top = src.join("AgentMux");
        std::fs::create_dir_all(top.join("usr/bin")).unwrap();
        std::fs::create_dir_all(top.join("usr/share/agentmux")).unwrap();
        std::fs::write(top.join("agentmux.sh"), b"#!/bin/sh
").unwrap();
        std::fs::write(top.join("usr/bin/agentmux-srv-0.59.30-linux.x64"), b"").unwrap();
        std::fs::write(top.join("usr/share/agentmux/VERSION"), b"0.59.30
").unwrap();
        let archive = dir.path().join("AgentMux_0.59.30_amd64-portable.tar.xz");
        let made = std::process::Command::new(tar_exe()).arg("-cJf").arg(&archive).arg("-C").arg(&src).arg("AgentMux").status().unwrap();
        assert!(made.success());
        let parent = dir.path().join("apps");
        std::fs::create_dir_all(&parent).unwrap();
        let local = "local-main-b28b7a-f62415cd";
        let folder = unpack_portable(&archive, "0.59.30", &parent, local).await.unwrap();
        assert_eq!(folder, parent.join("AgentMux-0.59.30"));
        assert!(is_staged_portable(&folder, "0.59.30"));
        assert_eq!(agentmux_common::runtime_mode::recorded_channel_in(&folder.join("usr/bin")).as_deref(), Some(local));
        let err = unpack_portable(&archive, "0.59.31", &parent, local).await.unwrap_err();
        assert!(err.contains("isn't an AgentMux 0.59.31 portable"), "{err}");
    }

}
