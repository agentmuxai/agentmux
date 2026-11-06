// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! AgentMux's own copy of an agent CLI that is not on npm: the pinned release
//! file is fetched straight from the vendor's origin, checked against a
//! SHA-256 compiled into this registry, and unpacked next to the npm installs
//! (`<shared>/cli/<provider>/<pin>/`, same lock, same completion marker).
//! SPEC_ISOLATED_INSTALL_FOR_NON_NPM_CLIS_2026_11_05.md.
//!
//! No vendor script is ever run, and a caller names only a provider id: the
//! URL, the hash and the size all come from [`SPECS`], so the vendor's origin
//! cannot swap the file and a request cannot point the install elsewhere.
//! The layout is `<dir>/bin/<cli>[.exe]`; [`crate::backend::cli_install`]
//! finds it the same way it finds an npm shim.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use agentmux_common::DataPaths;
use sha2::{Digest, Sha256};

use crate::backend::cli_install::{self, InstallOutcome};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// No answer, or no bytes, for this long ends the download.
const STALL_TIMEOUT: Duration = Duration::from_secs(60);
/// The unpacked executable is far smaller than this; the cap stops an archive
/// that claims otherwise from filling the disk.
const MAX_EXE_BYTES: u64 = 600 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Archive {
    Zip,
    TarGz,
}

/// One downloadable file for one platform.
#[derive(Debug)]
pub struct Asset {
    /// `std::env::consts::OS`: "windows", "macos", "linux".
    pub os: &'static str,
    /// `std::env::consts::ARCH`: "x86_64", "aarch64".
    pub arch: &'static str,
    pub url: &'static str,
    pub sha256: &'static str,
    pub size: u64,
    pub archive: Archive,
    /// The executable's path inside the archive, without a leading `./`.
    pub exe_in_archive: &'static str,
}

#[derive(Debug)]
pub struct DownloadSpec {
    pub provider_id: &'static str,
    /// Equals the provider's `pinned_version`; every `url` carries it.
    pub version: &'static str,
    /// Every asset URL starts with this (a registry test holds it).
    pub url_prefix: &'static str,
    pub assets: &'static [Asset],
}

/// Hashes are the `digest` GitHub lists for each release asset (the files were
/// also downloaded and hashed on 2026-11-05 for Windows x64). Linux uses the
/// static `musl` builds. goose.exe is not code-signed, so the hash is the only
/// check on Windows.
pub static SPECS: &[DownloadSpec] = &[DownloadSpec {
    provider_id: "goose",
    version: "1.54.0",
    url_prefix: "https://github.com/aaif-goose/goose/releases/download/v1.54.0/",
    assets: &[
        Asset {
            os: "windows",
            arch: "x86_64",
            url: "https://github.com/aaif-goose/goose/releases/download/v1.54.0/goose-x86_64-pc-windows-msvc.zip",
            sha256: "5c39bc91c3eadc231847e21a8a469e778431084437b233a63a950fee22db6239",
            size: 77_666_256,
            archive: Archive::Zip,
            exe_in_archive: "goose-package/goose.exe",
        },
        Asset {
            os: "macos",
            arch: "aarch64",
            url: "https://github.com/aaif-goose/goose/releases/download/v1.54.0/goose-aarch64-apple-darwin.tar.gz",
            sha256: "feda29744cd4e13f9a5211a968ed47c67fb8fa79647f4d271d92d564998a393c",
            size: 83_308_295,
            archive: Archive::TarGz,
            exe_in_archive: "goose",
        },
        Asset {
            os: "macos",
            arch: "x86_64",
            url: "https://github.com/aaif-goose/goose/releases/download/v1.54.0/goose-x86_64-apple-darwin.tar.gz",
            sha256: "7cf5bcd0c23983993900a4da49c91a0eb30ad492d2ff85af3e03bb718fb9b8af",
            size: 86_241_361,
            archive: Archive::TarGz,
            exe_in_archive: "goose",
        },
        Asset {
            os: "linux",
            arch: "x86_64",
            url: "https://github.com/aaif-goose/goose/releases/download/v1.54.0/goose-x86_64-unknown-linux-musl.tar.gz",
            sha256: "777e49b6ae128fae06600f94bf51ff05c81572fce63c1581e9ea94bc49d64e42",
            size: 51_779_009,
            archive: Archive::TarGz,
            exe_in_archive: "goose",
        },
        Asset {
            os: "linux",
            arch: "aarch64",
            url: "https://github.com/aaif-goose/goose/releases/download/v1.54.0/goose-aarch64-unknown-linux-musl.tar.gz",
            sha256: "baeb3e1b188fce75b444af35b5955f255e68fde87f60a5ba01f032227618a4ed",
            size: 50_389_197,
            archive: Archive::TarGz,
            exe_in_archive: "goose",
        },
    ],
}];

/// The download install for `provider_id`, if AgentMux has one.
pub fn download_spec(provider_id: &str) -> Option<&'static DownloadSpec> {
    SPECS.iter().find(|s| s.provider_id == provider_id)
}

impl DownloadSpec {
    /// The asset for this machine, or None (goose has no Windows arm64 build).
    pub fn asset_for_host(&self) -> Option<&'static Asset> {
        self.asset_for(std::env::consts::OS, std::env::consts::ARCH)
    }

    pub fn asset_for(&self, os: &str, arch: &str) -> Option<&'static Asset> {
        self.assets.iter().find(|a| a.os == os && a.arch == arch)
    }
}

/// `<dir>/bin/<cli>`, `.exe` on Windows.
pub fn download_bin(dir: &Path, cli_command: &str) -> PathBuf {
    let bin = dir.join("bin");
    if cfg!(windows) {
        bin.join(format!("{cli_command}.exe"))
    } else {
        bin.join(cli_command)
    }
}

#[derive(Debug)]
pub enum DownloadError {
    Lock(std::io::Error),
    Clear(PathBuf, std::io::Error),
    NoSource(String),
    NoAsset { os: &'static str, arch: &'static str },
    Fetch(String),
    TooLarge,
    Short { got: u64, want: u64 },
    HashMismatch { expected: String, actual: String },
    Unpack(String),
    NotRunnable(PathBuf),
    Mark(PathBuf, std::io::Error),
    Cancelled,
}

impl std::fmt::Display for DownloadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DownloadError::Lock(e) => write!(f, "cannot take the CLI install lock: {e}"),
            DownloadError::Clear(d, e) => {
                write!(f, "cannot clear an incomplete CLI install at {}: {e}", d.display())
            }
            DownloadError::NoSource(p) => write!(f, "AgentMux has no download for {p}"),
            DownloadError::NoAsset { os, arch } => {
                write!(f, "the vendor publishes no build for {os} on {arch}")
            }
            DownloadError::Fetch(e) => write!(f, "download failed: {e}"),
            DownloadError::TooLarge => write!(f, "the download is larger than the release lists it"),
            DownloadError::Short { got, want } => {
                write!(f, "the download stopped at {got} of {want} bytes")
            }
            DownloadError::HashMismatch { expected, actual } => write!(
                f,
                "the download does not match the checksum AgentMux expects (expected sha256 {expected}, got {actual}); nothing was installed"
            ),
            DownloadError::Unpack(e) => write!(f, "cannot unpack the download: {e}"),
            DownloadError::NotRunnable(p) => {
                write!(f, "the installed program does not run: {}", p.display())
            }
            DownloadError::Mark(d, e) => {
                write!(f, "cannot mark the CLI install complete at {}: {e}", d.display())
            }
            DownloadError::Cancelled => write!(f, "cancelled"),
        }
    }
}

/// Install `provider_id`'s pinned release, **async**: take the cross-instance
/// lock (blocking, off the runtime), reuse a finished install another instance
/// made while this one waited, otherwise clear leftovers and install.
/// `resolvecli`'s path; `install.start` polls the lock itself (it must stay
/// cancellable) and calls [`install_into`] once it holds it.
pub async fn install_download_cli(
    paths: &DataPaths,
    provider_id: &str,
    on_line: &(dyn Fn(&'static str, &str) + Sync),
    cancel: &AtomicBool,
) -> Result<InstallOutcome, DownloadError> {
    let spec = download_spec(provider_id).ok_or_else(|| DownloadError::NoSource(provider_id.to_string()))?;
    let provider = crate::backend::providers::get_provider(provider_id)
        .ok_or_else(|| DownloadError::NoSource(provider_id.to_string()))?;
    let (lock_paths, id, pin) = (paths.clone(), provider.id.to_string(), spec.version.to_string());
    let _guard = tokio::task::spawn_blocking(move || cli_install::lock_install(&lock_paths, &id, &pin))
        .await
        .map_err(|e| DownloadError::Lock(std::io::Error::other(e)))?
        .map_err(DownloadError::Lock)?;
    let dir = cli_install::install_dir(paths, provider.id, spec.version);
    if cli_install::is_valid_install(&dir, provider.cli_command) {
        return Ok(InstallOutcome::AlreadyInstalled(download_bin(&dir, provider.cli_command)));
    }
    cli_install::clear_unless_valid(&dir, provider.cli_command).map_err(|e| DownloadError::Clear(dir.clone(), e))?;
    install_into(paths, spec, provider.cli_command, on_line, cancel).await
}

/// Download, verify, unpack and publish into the install dir. The caller holds
/// the install lock and has cleared leftovers. A failure removes the partial
/// dir, so nothing unmarked is left behind.
pub async fn install_into(
    paths: &DataPaths,
    spec: &'static DownloadSpec,
    cli_command: &str,
    on_line: &(dyn Fn(&'static str, &str) + Sync),
    cancel: &AtomicBool,
) -> Result<InstallOutcome, DownloadError> {
    let asset = spec.asset_for_host().ok_or(DownloadError::NoAsset {
        os: std::env::consts::OS,
        arch: std::env::consts::ARCH,
    })?;
    let dir = cli_install::install_dir(paths, spec.provider_id, spec.version);
    let result = install_asset(&dir, spec, asset, cli_command, on_line, cancel).await;
    if result.is_err() {
        let _ = std::fs::remove_dir_all(&dir);
    }
    result
}

async fn install_asset(
    dir: &Path,
    spec: &DownloadSpec,
    asset: &Asset,
    cli_command: &str,
    on_line: &(dyn Fn(&'static str, &str) + Sync),
    cancel: &AtomicBool,
) -> Result<InstallOutcome, DownloadError> {
    let bin = download_bin(dir, cli_command);
    std::fs::create_dir_all(bin.parent().unwrap_or(dir)).map_err(|e| DownloadError::Unpack(e.to_string()))?;
    let archive = dir.join("download.part");
    on_line("stdout", &format!("downloading {} {} from {}", spec.provider_id, spec.version, asset.url));
    fetch_verified(asset, &archive, spec, on_line, cancel).await?;
    on_line("stdout", "checksum matches; unpacking");
    let (src, asset_exe, kind, out) = (archive.clone(), asset.exe_in_archive, asset.archive, bin.clone());
    tokio::task::spawn_blocking(move || extract_exe(&src, kind, asset_exe, &out))
        .await
        .map_err(|e| DownloadError::Unpack(e.to_string()))??;
    let _ = std::fs::remove_file(&archive);
    let probe = bin.to_string_lossy().into_owned();
    let version = tokio::task::spawn_blocking(move || {
        crate::backend::tool_store::probe_version(&probe, &Some("--version".to_string()))
    })
    .await
    .ok()
    .flatten();
    let Some(version) = version else {
        return Err(DownloadError::NotRunnable(bin));
    };
    on_line("stdout", &format!("{} {version} is installed", spec.provider_id));
    cli_install::mark_complete(dir, spec.version).map_err(|e| DownloadError::Mark(dir.to_path_buf(), e))?;
    Ok(InstallOutcome::Installed(bin))
}

/// Stream `asset.url` into `dest`, hashing as it arrives. Refuses more bytes
/// than the registry lists, fewer, or a different hash.
async fn fetch_verified(
    asset: &Asset,
    dest: &Path,
    spec: &DownloadSpec,
    on_line: &(dyn Fn(&'static str, &str) + Sync),
    cancel: &AtomicBool,
) -> Result<(), DownloadError> {
    use tokio::io::AsyncWriteExt;
    // Defence in depth: the registry's own URL must be under its own prefix.
    if !asset.url.starts_with(spec.url_prefix) {
        return Err(DownloadError::Fetch("the release file is outside its registered origin".into()));
    }
    let client = reqwest::Client::builder()
        .user_agent(format!("AgentMux/{}", env!("CARGO_PKG_VERSION")))
        .connect_timeout(CONNECT_TIMEOUT)
        .build()
        .map_err(|e| DownloadError::Fetch(e.to_string()))?;
    let mut resp = tokio::time::timeout(STALL_TIMEOUT, client.get(asset.url).send())
        .await
        .map_err(|_| DownloadError::Fetch("the server did not answer".into()))?
        .and_then(|r| r.error_for_status())
        .map_err(|e| DownloadError::Fetch(e.to_string()))?;
    let mut out = tokio::fs::File::create(dest).await.map_err(|e| DownloadError::Fetch(e.to_string()))?;
    let mut hasher = Sha256::new();
    let mut got = 0u64;
    let mut shown = 0u64;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(DownloadError::Cancelled);
        }
        let chunk = tokio::time::timeout(STALL_TIMEOUT, resp.chunk())
            .await
            .map_err(|_| DownloadError::Fetch("the download stalled".into()))?
            .map_err(|e| DownloadError::Fetch(e.to_string()))?;
        let Some(chunk) = chunk else { break };
        got += chunk.len() as u64;
        if got > asset.size {
            return Err(DownloadError::TooLarge);
        }
        hasher.update(&chunk);
        out.write_all(&chunk).await.map_err(|e| DownloadError::Fetch(e.to_string()))?;
        let tenth = got * 10 / asset.size.max(1);
        if tenth > shown {
            shown = tenth;
            on_line("stdout", &format!("downloaded {}%", tenth * 10));
        }
    }
    out.flush().await.map_err(|e| DownloadError::Fetch(e.to_string()))?;
    drop(out);
    verify_download(got, hasher.finalize().as_slice(), asset)
}

/// The size and the hash of what arrived against the registry's.
fn verify_download(got: u64, digest: &[u8], asset: &Asset) -> Result<(), DownloadError> {
    if got != asset.size {
        return Err(DownloadError::Short { got, want: asset.size });
    }
    let actual = hex::encode(digest);
    if !actual.eq_ignore_ascii_case(asset.sha256) {
        return Err(DownloadError::HashMismatch { expected: asset.sha256.to_string(), actual });
    }
    Ok(())
}

/// Copy the one executable named `exe_in_archive` out of `archive` into `out`
/// (write-then-rename). Nothing else in the archive is written, so no entry
/// name can place a file anywhere.
fn extract_exe(archive: &Path, kind: Archive, exe_in_archive: &str, out: &Path) -> Result<(), DownloadError> {
    let part = out.with_extension("part");
    let file = std::fs::File::open(archive).map_err(|e| DownloadError::Unpack(e.to_string()))?;
    let found = match kind {
        Archive::Zip => copy_from_zip(file, exe_in_archive, &part),
        Archive::TarGz => copy_from_tar_gz(file, exe_in_archive, &part),
    };
    let found = found.inspect_err(|_| {
        let _ = std::fs::remove_file(&part);
    })?;
    if !found {
        let _ = std::fs::remove_file(&part);
        return Err(DownloadError::Unpack(format!("{exe_in_archive} is not in the archive")));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&part, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| DownloadError::Unpack(e.to_string()))?;
    }
    std::fs::rename(&part, out).map_err(|e| DownloadError::Unpack(e.to_string()))
}

fn normalize(name: &str) -> String {
    name.replace('\\', "/").trim_start_matches("./").to_string()
}

fn copy_capped(src: &mut dyn Read, dest: &Path) -> Result<(), DownloadError> {
    let mut out = std::fs::File::create(dest).map_err(|e| DownloadError::Unpack(e.to_string()))?;
    let copied = std::io::copy(&mut src.take(MAX_EXE_BYTES + 1), &mut out).map_err(|e| DownloadError::Unpack(e.to_string()))?;
    if copied > MAX_EXE_BYTES {
        return Err(DownloadError::Unpack("the program in the archive is larger than allowed".into()));
    }
    out.flush().map_err(|e| DownloadError::Unpack(e.to_string()))
}

fn copy_from_zip(file: std::fs::File, want: &str, dest: &Path) -> Result<bool, DownloadError> {
    let mut zip = zip::ZipArchive::new(file).map_err(|e| DownloadError::Unpack(e.to_string()))?;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| DownloadError::Unpack(e.to_string()))?;
        if entry.is_file() && normalize(entry.name()) == want {
            copy_capped(&mut entry, dest)?;
            return Ok(true);
        }
    }
    Ok(false)
}

fn copy_from_tar_gz(file: std::fs::File, want: &str, dest: &Path) -> Result<bool, DownloadError> {
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(file));
    for entry in tar.entries().map_err(|e| DownloadError::Unpack(e.to_string()))? {
        let mut entry = entry.map_err(|e| DownloadError::Unpack(e.to_string()))?;
        if !entry.header().entry_type().is_file() {
            continue;
        }
        let name = entry.path().map_err(|e| DownloadError::Unpack(e.to_string()))?.to_string_lossy().into_owned();
        if normalize(&name) == want {
            copy_capped(&mut entry, dest)?;
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(test)]
#[path = "cli_download_tests.rs"]
mod tests;
