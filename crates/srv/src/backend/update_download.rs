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
//! to start (phase 3b). Progress and the result go out as the
//! `app:updatestatus` event (`update_check`).
//!
//! The download is kept in `~/.agentmux/updates/<version>/` while it runs and
//! deleted once unpacked. Unpacking uses Windows' own `tar.exe` (libarchive,
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
/// No bytes for this long ends the download.
const STALL_TIMEOUT: Duration = Duration::from_secs(60);

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

fn set(version: &str, state: &str, received: u64, total: u64, error: Option<String>, folder: Option<&Path>) {
    update_check::set_download(Some(DownloadStatus {
        version: version.to_string(),
        state: state.to_string(),
        received,
        total,
        error,
        folder: folder.map(|f| f.display().to_string()),
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
    let InstallKind::WindowsPortable { root } = update_check::install_kind() else {
        return Err("AgentMux can't download this release itself; use the release page".into());
    };
    if let Some(d) = &status.download {
        let staged = d.folder.as_deref().is_some_and(|f| is_staged_portable(Path::new(f), &version));
        if d.version == version && d.state == "ready" && staged {
            return Ok(status);
        }
    }
    if RUNNING.swap(true, Ordering::SeqCst) {
        return Ok(status);
    }
    set(&version, "downloading", 0, asset.size, None, None);
    tokio::spawn(async move {
        let result = run(&asset, &version, &root).await;
        RUNNING.store(false, Ordering::SeqCst);
        match result {
            Ok(folder) => {
                tracing::info!(folder = %folder.display(), "update: {version} is ready");
                set(&version, "ready", asset.size, asset.size, None, Some(&folder));
            }
            Err(e) => {
                tracing::warn!(error = %e, "update: downloading {version} failed");
                set(&version, "failed", 0, asset.size, Some(e), None);
            }
        }
    });
    Ok(update_check::current_status())
}

async fn run(asset: &ReleaseAsset, version: &str, root: &Path) -> Result<PathBuf, String> {
    let dir = crate::backend::base::get_mux_data_dir().join("updates").join(version);
    tokio::fs::create_dir_all(&dir).await.map_err(|e| format!("can't create {}: {e}", dir.display()))?;
    let file = dir.join(&asset.name);
    if let Err(e) = download(asset, &file, version).await {
        let _ = tokio::fs::remove_file(&file).await;
        return Err(e);
    }
    set(version, "unpacking", asset.size, asset.size, None, None);
    // Next to the running folder, so it's where the user expects; under
    // ~/.agentmux/updates when that folder's parent can't be written.
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
    Err(last)
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
    let mut resp = client
        .get(&asset.url)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| format!("download failed: {e}"))?;
    let part = dest.with_file_name(format!("{}.part", asset.name));
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
                set(version, "downloading", received.bytes, asset.size, None, None);
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
fn is_staged_portable(dir: &Path, version: &str) -> bool {
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
    let partial = parent.join(format!(".{name}.partial"));
    let _ = tokio::fs::remove_dir_all(&partial).await;
    tokio::fs::create_dir_all(&partial).await.map_err(|e| format!("can't write to {}: {e}", parent.display()))?;
    use agentmux_common::win32::NoWindow;
    let output = tokio::process::Command::new(tar_exe())
        .arg("-xf")
        .arg(file)
        .arg("-C")
        .arg(&partial)
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true)
        .no_window()
        .output()
        .await;
    let checked = match output {
        Err(e) => Err(format!("couldn't run tar to unpack it: {e}")),
        Ok(o) if !o.status.success() => {
            let why = String::from_utf8_lossy(&o.stderr).lines().next().unwrap_or("").to_string();
            Err(format!("tar couldn't unpack it ({why}); this Windows may be too old to read .7z files"))
        }
        Ok(_) if !is_staged_portable(&partial, version) => {
            Err(format!("what was unpacked isn't an AgentMux {version} portable"))
        }
        Ok(_) => Ok(()),
    };
    let target = free_folder(parent, &name);
    let result = match checked {
        Ok(()) => tokio::fs::rename(&partial, &target)
            .await
            .map(|_| target)
            .map_err(|e| format!("can't move the unpacked folder into place: {e}")),
        Err(e) => Err(e),
    };
    if result.is_err() {
        let _ = tokio::fs::remove_dir_all(&partial).await;
    }
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
        assert!(!parent.join(".agentmux-0.59.27-x64-portable.partial").exists());
        // Asked again, the staged folder is reused.
        assert_eq!(unpack_portable(&archive, "0.59.27", &parent).await.unwrap(), folder);
        // An archive of another version is refused and leaves nothing.
        let err = unpack_portable(&archive, "0.59.28", &parent).await.unwrap_err();
        assert!(err.contains("isn't an AgentMux 0.59.28 portable"), "{err}");
        assert!(!parent.join(".agentmux-0.59.28-x64-portable.partial").exists());
        assert!(!parent.join("agentmux-0.59.28-x64-portable").exists());
    }
}
