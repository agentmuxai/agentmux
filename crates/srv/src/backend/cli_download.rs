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
    /// The download is the program itself (`exe_in_archive` is empty).
    Raw,
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
/// also downloaded and hashed on 2026-11-09, all five). Linux uses the
/// static `musl` builds. goose.exe is not code-signed, so the hash is the only
/// check on Windows.
pub static SPECS: &[DownloadSpec] = &[DownloadSpec {
    provider_id: "goose",
    version: "1.58.0",
    url_prefix: "https://github.com/aaif-goose/goose/releases/download/v1.58.0/",
    assets: &[
        Asset {
            os: "windows",
            arch: "x86_64",
            url: "https://github.com/aaif-goose/goose/releases/download/v1.58.0/goose-x86_64-pc-windows-msvc.zip",
            sha256: "f3b6b1a550890874ab93d560c58d97b356dc625a58cfeba8fecca9bdb0ba3ffd",
            size: 53_196_217,
            archive: Archive::Zip,
            exe_in_archive: "goose-package/goose.exe",
        },
        Asset {
            os: "macos",
            arch: "aarch64",
            url: "https://github.com/aaif-goose/goose/releases/download/v1.58.0/goose-aarch64-apple-darwin.tar.gz",
            sha256: "c45201861811d65f79e51e90ba649744aa017dbdc800efa1e1d36955c5c2eb48",
            size: 56_573_635,
            archive: Archive::TarGz,
            exe_in_archive: "goose",
        },
        Asset {
            os: "macos",
            arch: "x86_64",
            url: "https://github.com/aaif-goose/goose/releases/download/v1.58.0/goose-x86_64-apple-darwin.tar.gz",
            sha256: "6068beadf8e053bc14a601dddb5c83617fb5e5c084575a38749de62d4b0b15d9",
            size: 58_811_628,
            archive: Archive::TarGz,
            exe_in_archive: "goose",
        },
        Asset {
            os: "linux",
            arch: "x86_64",
            url: "https://github.com/aaif-goose/goose/releases/download/v1.58.0/goose-x86_64-unknown-linux-musl.tar.gz",
            sha256: "52f0f31a4a4f987f504b568e84102b8b8e7d6d5f9e961fd380c3ff99f7b01c26",
            size: 50_135_549,
            archive: Archive::TarGz,
            exe_in_archive: "goose",
        },
        Asset {
            os: "linux",
            arch: "aarch64",
            url: "https://github.com/aaif-goose/goose/releases/download/v1.58.0/goose-aarch64-unknown-linux-musl.tar.gz",
            sha256: "f660ca19ea4765dece34f56d8a09d42667d49fc1fd9a904b4755ede02d5327d7",
            size: 48_732_595,
            archive: Archive::TarGz,
            exe_in_archive: "goose",
        },
    ],
},
// xAI publishes no checksum for Grok, so these were taken once on 2026-11-09
// from x.ai itself and are the pin: a changed file fails the install. The Windows
// builds are Authenticode-signed (X.AI LLC). The installer's extras
// (`grove*` and a MinGit on Windows) are not fetched: grove falls back to the
// git on PATH.
DownloadSpec {
    provider_id: "grok",
    version: "1.0.77",
    url_prefix: "https://x.ai/cli/grok-1.0.77-",
    assets: &[
        Asset {
            os: "windows",
            arch: "x86_64",
            url: "https://x.ai/cli/grok-1.0.77-windows-x86_64.exe",
            sha256: "6b757512d240a6439535ca75aad039e581d00f17ba8e937112fd0dbaadf1bc81",
            size: 181_617_808,
            archive: Archive::Raw,
            exe_in_archive: "",
        },
        Asset {
            os: "windows",
            arch: "aarch64",
            url: "https://x.ai/cli/grok-1.0.77-windows-aarch64.exe",
            sha256: "ab4e5fddcd9f189fb379405a8fbfda43edf38dbbe893d302c0df697d180a6034",
            size: 154_323_088,
            archive: Archive::Raw,
            exe_in_archive: "",
        },
        Asset {
            os: "macos",
            arch: "aarch64",
            url: "https://x.ai/cli/grok-1.0.77-macos-aarch64",
            sha256: "197c66a63e5ac46e07450b0ca21d141ce665050a412180f185dadfae1c7a5b2a",
            size: 184_995_488,
            archive: Archive::Raw,
            exe_in_archive: "",
        },
        Asset {
            os: "macos",
            arch: "x86_64",
            url: "https://x.ai/cli/grok-1.0.77-macos-x86_64",
            sha256: "7728f79ea713ae67a4fa4736dcf808f6b386d9642202aea1df695073fa56675a",
            size: 199_654_320,
            archive: Archive::Raw,
            exe_in_archive: "",
        },
        Asset {
            os: "linux",
            arch: "x86_64",
            url: "https://x.ai/cli/grok-1.0.77-linux-x86_64",
            sha256: "b702950aa5827d855e493278ab6215a4e0f5bceb5bba6c6243a9f55cdd143b0a",
            size: 201_655_392,
            archive: Archive::Raw,
            exe_in_archive: "",
        },
        Asset {
            os: "linux",
            arch: "aarch64",
            url: "https://x.ai/cli/grok-1.0.77-linux-aarch64",
            sha256: "b30eee14a656124ef3c9da8577b5e783ef2cffabeb2b679c5c49633e77eed574",
            size: 171_935_112,
            archive: Archive::Raw,
            exe_in_archive: "",
        },
    ],
},
// Meta's own manifest for this build lists a SHA-256 and size per platform; they are
// copied here, so this is not trust on first use. The binary is the one the vendor's
// launcher would fetch, run directly: the launcher's hourly self-update never runs,
// and `MUSE_NO_AUTO_UPDATE=1` is set besides. The Windows build is Authenticode-signed
// (Meta Platforms, Inc.).
DownloadSpec {
    provider_id: "muse",
    version: "1.4.14-R7309.1",
    url_prefix: "https://lookaside.facebook.com/lookaside/muse/download/?channel=muse&version=1.4.14-R7309.1&file=",
    assets: &[
        Asset {
            os: "windows",
            arch: "x86_64",
            url: "https://lookaside.facebook.com/lookaside/muse/download/?channel=muse&version=1.4.14-R7309.1&file=muse-x86-windows.exe",
            sha256: "20d18c426cc26e9e4b3cc07008ec07ec97e12e466a2be8cd2797b2e0e2018382",
            size: 435_429_624,
            archive: Archive::Raw,
            exe_in_archive: "",
        },
        Asset {
            os: "windows",
            arch: "aarch64",
            url: "https://lookaside.facebook.com/lookaside/muse/download/?channel=muse&version=1.4.14-R7309.1&file=muse-aarch64-windows.exe",
            sha256: "64cef76cb210de993d31478ee582ef4639bbb212df8912b74f45d40431e66e82",
            size: 398_733_560,
            archive: Archive::Raw,
            exe_in_archive: "",
        },
        Asset {
            os: "macos",
            arch: "aarch64",
            url: "https://lookaside.facebook.com/lookaside/muse/download/?channel=muse&version=1.4.14-R7309.1&file=muse-aarch64-macos",
            sha256: "1c96114408acbeade9a8fb79e3b4425e5b64d50425938568aef01bdc04ce2e03",
            size: 423_687_488,
            archive: Archive::Raw,
            exe_in_archive: "",
        },
        Asset {
            os: "macos",
            arch: "x86_64",
            url: "https://lookaside.facebook.com/lookaside/muse/download/?channel=muse&version=1.4.14-R7309.1&file=muse-x86-macos",
            sha256: "6da5a12d3298afba45afec929fc385b516084ed28e9504ae29354dbcdfc4089d",
            size: 443_142_560,
            archive: Archive::Raw,
            exe_in_archive: "",
        },
        Asset {
            os: "linux",
            arch: "x86_64",
            url: "https://lookaside.facebook.com/lookaside/muse/download/?channel=muse&version=1.4.14-R7309.1&file=muse-x86-linux",
            sha256: "0809aac7cb58d18f5022b6d05e02aa1974683c0ec430e5f04d25b60ed17de2da",
            size: 414_927_192,
            archive: Archive::Raw,
            exe_in_archive: "",
        },
        Asset {
            os: "linux",
            arch: "aarch64",
            url: "https://lookaside.facebook.com/lookaside/muse/download/?channel=muse&version=1.4.14-R7309.1&file=muse-aarch64-linux",
            sha256: "3dc39b8fa26f9963cc2038f2966c0932ed5a39f0f9efb39cc0266a261cbbcf76",
            size: 371_529_960,
            archive: Archive::Raw,
            exe_in_archive: "",
        },
    ],
},
// Google serves only the latest build through its manifests, so the URL of the build
// current on 2026-11-06 is pinned (an older build, 1.5.1, still downloaded a day after
// it was replaced). Each file's SHA-512 matched Google's manifest when taken; the
// SHA-256 here is of the same file. Linux uses the static musl builds. The Windows
// build is Authenticode-signed (Google LLC). Where the vendor's installer would run
// `agy install` to edit shell profiles, nothing is run here.
DownloadSpec {
    provider_id: "antigravity",
    version: "1.5.2",
    url_prefix: "https://storage.googleapis.com/antigravity-public/antigravity-cli/1.5.2-4951595792859136/",
    assets: &[
        Asset {
            os: "windows",
            arch: "x86_64",
            url: "https://storage.googleapis.com/antigravity-public/antigravity-cli/1.5.2-4951595792859136/windows-x64/cli_windows_x64.exe",
            sha256: "3f6f64a331fcd75c4ea3a25f89fdb0a9192feaa6e49bb828510b65dc0030b61e",
            size: 222_561_944,
            archive: Archive::Raw,
            exe_in_archive: "",
        },
        Asset {
            os: "windows",
            arch: "aarch64",
            url: "https://storage.googleapis.com/antigravity-public/antigravity-cli/1.5.2-4951595792859136/windows-arm/cli_windows_arm64.exe",
            sha256: "ab49a39c1fc2496093d7d4486c03f3813a9982401834e4593c93b68b68a44300",
            size: 211_127_448,
            archive: Archive::Raw,
            exe_in_archive: "",
        },
        Asset {
            os: "macos",
            arch: "aarch64",
            url: "https://storage.googleapis.com/antigravity-public/antigravity-cli/1.5.2-4951595792859136/darwin-arm/cli_mac_arm64.tar.gz",
            sha256: "c3f56b381c4755cdd02e5de67373c24788b2a2aeade156756b4419b67253bd0c",
            size: 64_486_606,
            archive: Archive::TarGz,
            exe_in_archive: "antigravity",
        },
        Asset {
            os: "macos",
            arch: "x86_64",
            url: "https://storage.googleapis.com/antigravity-public/antigravity-cli/1.5.2-4951595792859136/darwin-x64/cli_mac_x64.tar.gz",
            sha256: "fe99ebacc6bd46b4bf2d85c718420b28b699210cf7c2e3055242e2dad7a1beb3",
            size: 70_102_814,
            archive: Archive::TarGz,
            exe_in_archive: "antigravity",
        },
        Asset {
            os: "linux",
            arch: "x86_64",
            url: "https://storage.googleapis.com/antigravity-public/antigravity-cli/1.5.2-4951595792859136/linux-x64-musl/cli_linux_x64_musl.tar.gz",
            sha256: "6671806b8e68b0c3cfb87ae99675a9b037b33d9d4cf556fc633a35d198bebf5c",
            size: 72_824_890,
            archive: Archive::TarGz,
            exe_in_archive: "antigravity",
        },
        Asset {
            os: "linux",
            arch: "aarch64",
            url: "https://storage.googleapis.com/antigravity-public/antigravity-cli/1.5.2-4951595792859136/linux-arm-musl/cli_linux_arm64_musl.tar.gz",
            sha256: "f83f4246f5de84f5d966564bd844a65895cb015439eb55a82293fc0ab7c81e78",
            size: 66_759_805,
            archive: Archive::TarGz,
            exe_in_archive: "antigravity",
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
    /// A helper program (uv) failed; carries its last output lines.
    Tool(String),
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
            DownloadError::Tool(e) => write!(f, "{e}"),
            DownloadError::Mark(d, e) => {
                write!(f, "cannot mark the CLI install complete at {}: {e}", d.display())
            }
            DownloadError::Cancelled => write!(f, "cancelled"),
        }
    }
}

/// Whether AgentMux installs `provider_id` itself without npm: a release
/// download (goose) or a hash-locked Python venv through uv (Aider, Kimi).
pub fn is_managed_install(provider_id: &str) -> bool {
    download_spec(provider_id).is_some() || crate::backend::cli_uv::uv_spec(provider_id).is_some()
}

/// The pin a managed install of `provider_id` is keyed by.
fn managed_version(provider_id: &str) -> Option<&'static str> {
    download_spec(provider_id)
        .map(|s| s.version)
        .or_else(|| crate::backend::cli_uv::uv_spec(provider_id).map(|s| s.version))
}

/// Install `provider_id`'s pinned copy, **async**: take the cross-instance
/// lock (blocking, off the runtime), reuse a finished install another instance
/// made while this one waited, otherwise clear leftovers and install.
/// `resolvecli`'s path; `install.start` polls the lock itself (it must stay
/// cancellable) and calls [`install_managed_into`] once it holds it.
pub async fn install_managed_cli(
    paths: &DataPaths,
    provider_id: &str,
    on_line: &(dyn Fn(&'static str, &str) + Sync),
    cancel: &AtomicBool,
) -> Result<InstallOutcome, DownloadError> {
    let version = managed_version(provider_id).ok_or_else(|| DownloadError::NoSource(provider_id.to_string()))?;
    let provider = crate::backend::providers::get_provider(provider_id)
        .ok_or_else(|| DownloadError::NoSource(provider_id.to_string()))?;
    let (lock_paths, id, pin) = (paths.clone(), provider.id.to_string(), version.to_string());
    let _guard = tokio::task::spawn_blocking(move || cli_install::lock_install(&lock_paths, &id, &pin))
        .await
        .map_err(|e| DownloadError::Lock(std::io::Error::other(e)))?
        .map_err(DownloadError::Lock)?;
    let dir = cli_install::install_dir(paths, provider.id, version);
    if cli_install::is_valid_install(&dir, provider.cli_command) {
        if let Some(bin) = cli_install::installed_bin(&dir, provider.cli_command) {
            return Ok(InstallOutcome::AlreadyInstalled(bin));
        }
    }
    cli_install::clear_unless_valid(&dir, provider.cli_command).map_err(|e| DownloadError::Clear(dir.clone(), e))?;
    install_managed_into(paths, provider.id, provider.cli_command, on_line, cancel).await
}

/// Install into the install dir; the caller holds the lock and has cleared
/// leftovers. A failure removes the partial dir, so nothing unmarked is left.
pub async fn install_managed_into(
    paths: &DataPaths,
    provider_id: &str,
    cli_command: &str,
    on_line: &(dyn Fn(&'static str, &str) + Sync),
    cancel: &AtomicBool,
) -> Result<InstallOutcome, DownloadError> {
    if let Some(spec) = crate::backend::cli_uv::uv_spec(provider_id) {
        return crate::backend::cli_uv::install_into(paths, spec, cli_command, on_line, cancel).await;
    }
    let spec = download_spec(provider_id).ok_or_else(|| DownloadError::NoSource(provider_id.to_string()))?;
    install_into(paths, spec, cli_command, on_line, cancel).await
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

pub(crate) async fn install_asset(
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
    let (probe, id) = (bin.clone(), spec.provider_id);
    let version = tokio::task::spawn_blocking(move || cli_install::probe_installed(&probe, id))
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
        Archive::Raw => copy_capped(&mut { file }, &part).map(|()| true),
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
