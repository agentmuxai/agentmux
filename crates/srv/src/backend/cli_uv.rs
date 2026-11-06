// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! AgentMux's own copy of a Python agent CLI (Aider, Kimi): a pinned `uv`, a
//! private managed Python, and a venv holding exactly the wheels in a
//! hash-locked requirements file, all inside the provider's install folder
//! (`<shared>/cli/<provider>/<pin>/`, same lock and completion marker as the
//! npm installs and the release downloads). The vendor's installer script is
//! never run. SPEC_ISOLATED_INSTALL_FOR_NON_NPM_CLIS_2026_11_05.md §3.
//!
//! `uv` itself is a pinned release download (`backend::cli_download`) kept in
//! `<shared>/tools/uv/<version>/`, outside `cli/` so the old-install pruner does
//! not treat it as a leftover. Every file the venv gets is checked against a
//! SHA-256 in the lock (`--require-hashes`, wheels only), so PyPI cannot swap a
//! package; the Python itself is checked by uv against hashes built into the
//! pinned uv.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use agentmux_common::win32::NoWindow;
use agentmux_common::DataPaths;
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};

use crate::backend::cli_download::{
    download_bin, install_asset, Archive, Asset, DownloadError, DownloadSpec,
};
use crate::backend::cli_install::{self, InstallOutcome};

/// One uv step (the Python download, the install) may take this long.
const STEP_TIMEOUT: Duration = Duration::from_secs(20 * 60);
const TAIL_LINES: usize = 12;
const PYPI: &str = "https://pypi.org/simple";

#[derive(Debug)]
pub struct UvSpec {
    pub provider_id: &'static str,
    /// The PyPI package the lock pins at `version`.
    pub package: &'static str,
    /// Equals the provider's `pinned_version` (a registry test holds it).
    pub version: &'static str,
    /// The interpreter uv fetches and the lock was resolved for.
    pub python: &'static str,
    /// Hash-locked requirements for the whole tree, `uv pip compile --universal --generate-hashes`.
    pub lock: &'static str,
    /// Packages with no wheel on PyPI, built from their hash-checked sdist
    /// (without build isolation: setuptools and wheel come from the same lock,
    /// installed first, since uv builds an sdist before it installs the rest).
    pub build_from_sdist: &'static [&'static str],
}

pub static UV_SPECS: &[UvSpec] = &[
    UvSpec {
        provider_id: "aider",
        package: "aider-chat",
        version: "0.86.2",
        python: "3.12",
        lock: include_str!("cli_locks/aider-0.86.2.txt"),
        build_from_sdist: &[],
    },
    // The Python `kimi-cli` is archived upstream; 1.52.0 is the last release and
    // what the harness's flags were checked against (spec §7.1).
    UvSpec {
        provider_id: "kimi",
        package: "kimi-cli",
        version: "1.52.0",
        python: "3.12",
        lock: include_str!("cli_locks/kimi-1.52.0.txt"),
        build_from_sdist: &["ripgrepy"],
    },
];

pub fn uv_spec(provider_id: &str) -> Option<&'static UvSpec> {
    UV_SPECS.iter().find(|s| s.provider_id == provider_id)
}

/// The pinned `uv`. Hashes are the `digest` GitHub lists for each release asset
/// (the Windows x64 zip was also downloaded and hashed on 2026-11-06).
pub static UV: DownloadSpec = DownloadSpec {
    provider_id: "uv",
    version: "0.13.7",
    url_prefix: "https://github.com/astral-sh/uv/releases/download/0.13.7/",
    assets: &[
        Asset {
            os: "windows",
            arch: "x86_64",
            url: "https://github.com/astral-sh/uv/releases/download/0.13.7/uv-x86_64-pc-windows-msvc.zip",
            sha256: "0cdc5b38fe5a8168ed6a25814f5ba43c21d313dd3187314fccb37b7cec46e021",
            size: 15_236_317,
            archive: Archive::Zip,
            exe_in_archive: "uv.exe",
        },
        Asset {
            os: "windows",
            arch: "aarch64",
            url: "https://github.com/astral-sh/uv/releases/download/0.13.7/uv-aarch64-pc-windows-msvc.zip",
            sha256: "505d9e65e7692bac3ddf98c66754aa26c5050c409c184af30a881a0a72cc85e3",
            size: 15_538_425,
            archive: Archive::Zip,
            exe_in_archive: "uv.exe",
        },
        Asset {
            os: "macos",
            arch: "aarch64",
            url: "https://github.com/astral-sh/uv/releases/download/0.13.7/uv-aarch64-apple-darwin.tar.gz",
            sha256: "81b811b57e4624df7ed3aac148e0359100d9962f3d9dce00aec533dc2e086731",
            size: 14_347_867,
            archive: Archive::TarGz,
            exe_in_archive: "uv-aarch64-apple-darwin/uv",
        },
        Asset {
            os: "macos",
            arch: "x86_64",
            url: "https://github.com/astral-sh/uv/releases/download/0.13.7/uv-x86_64-apple-darwin.tar.gz",
            sha256: "20c47e4c1f630b8abaff8c291fa9b9aa300fef92c4fbff95adf198b55287cda7",
            size: 17_026_382,
            archive: Archive::TarGz,
            exe_in_archive: "uv-x86_64-apple-darwin/uv",
        },
        Asset {
            os: "linux",
            arch: "x86_64",
            url: "https://github.com/astral-sh/uv/releases/download/0.13.7/uv-x86_64-unknown-linux-musl.tar.gz",
            sha256: "287491da03f2a785c70fa75b13e1f849918c2f2b5415fe84876b8214eeb792f9",
            size: 18_396_915,
            archive: Archive::TarGz,
            exe_in_archive: "uv-x86_64-unknown-linux-musl/uv",
        },
        Asset {
            os: "linux",
            arch: "aarch64",
            url: "https://github.com/astral-sh/uv/releases/download/0.13.7/uv-aarch64-unknown-linux-musl.tar.gz",
            sha256: "2d3ec076a0c4c0e8250bb6418c44080828b544f1a3be0b6e46adf9fb6923de45",
            size: 16_888_574,
            archive: Archive::TarGz,
            exe_in_archive: "uv-aarch64-unknown-linux-musl/uv",
        },
    ],
};

/// `<dir>/venv/Scripts/<cli>.exe` on Windows, `<dir>/venv/bin/<cli>` elsewhere.
pub fn venv_bin(dir: &Path, cli_command: &str) -> PathBuf {
    let (sub, name) = if cfg!(windows) {
        ("Scripts", format!("{cli_command}.exe"))
    } else {
        ("bin", cli_command.to_string())
    };
    dir.join("venv").join(sub).join(name)
}

fn venv_python(dir: &Path) -> PathBuf {
    venv_bin(dir, "python")
}

/// Where the pinned uv lives: `<shared>/tools/uv/<version>/`.
fn uv_dir(paths: &DataPaths) -> PathBuf {
    paths.shared_dir.join("tools").join("uv").join(UV.version)
}

/// The pinned uv, downloaded and verified on first use (shared by every
/// provider that needs it, under its own cross-instance lock).
async fn ensure_uv(
    paths: &DataPaths,
    on_line: &(dyn Fn(&'static str, &str) + Sync),
    cancel: &AtomicBool,
) -> Result<PathBuf, DownloadError> {
    let dir = uv_dir(paths);
    if cli_install::is_valid_install(&dir, "uv") {
        return Ok(download_bin(&dir, "uv"));
    }
    let lock_target = dir.clone();
    let _guard = tokio::task::spawn_blocking(move || cli_install::lock_dir(&lock_target))
        .await
        .map_err(|e| DownloadError::Lock(std::io::Error::other(e)))?
        .map_err(DownloadError::Lock)?;
    if cli_install::is_valid_install(&dir, "uv") {
        return Ok(download_bin(&dir, "uv"));
    }
    cli_install::clear_unless_valid(&dir, "uv").map_err(|e| DownloadError::Clear(dir.clone(), e))?;
    let asset = UV.asset_for_host().ok_or(DownloadError::NoAsset {
        os: std::env::consts::OS,
        arch: std::env::consts::ARCH,
    })?;
    on_line("stdout", &format!("installing uv {} (the Python installer AgentMux uses)", UV.version));
    let result = install_asset(&dir, &UV, asset, "uv", on_line, cancel).await;
    if result.is_err() {
        let _ = std::fs::remove_dir_all(&dir);
    }
    result.map(|_| download_bin(&dir, "uv"))
}

/// Install `spec` into its install dir: the caller holds the provider's install
/// lock and has cleared leftovers. A failure removes the partial dir.
pub async fn install_into(
    paths: &DataPaths,
    spec: &'static UvSpec,
    cli_command: &str,
    on_line: &(dyn Fn(&'static str, &str) + Sync),
    cancel: &AtomicBool,
) -> Result<InstallOutcome, DownloadError> {
    let dir = cli_install::install_dir(paths, spec.provider_id, spec.version);
    let result = install_venv(paths, &dir, spec, cli_command, on_line, cancel).await;
    if result.is_err() {
        let _ = std::fs::remove_dir_all(&dir);
    }
    result
}

/// What an sdist build needs in the venv before it starts.
const BUILD_TOOLS: &[&str] = &["setuptools", "wheel"];

/// The entries of a hash-locked requirements file for the named packages, each with its hash lines.
fn lock_entries(lock: &str, names: &[&str]) -> String {
    let mut out = String::new();
    let mut keep = false;
    for line in lock.lines() {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        if !t.starts_with("--hash=") {
            let name = t.split("==").next().unwrap_or("").trim();
            keep = names.iter().any(|n| n.eq_ignore_ascii_case(name));
        }
        if keep {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

async fn install_venv(
    paths: &DataPaths,
    dir: &Path,
    spec: &UvSpec,
    cli_command: &str,
    on_line: &(dyn Fn(&'static str, &str) + Sync),
    cancel: &AtomicBool,
) -> Result<InstallOutcome, DownloadError> {
    let uv = ensure_uv(paths, on_line, cancel).await?;
    std::fs::create_dir_all(dir).map_err(|e| DownloadError::Unpack(e.to_string()))?;
    let lock_file = dir.join("requirements.lock.txt");
    std::fs::write(&lock_file, spec.lock).map_err(|e| DownloadError::Unpack(e.to_string()))?;
    let venv = dir.join("venv");
    let uv_run = UvRun { uv: &uv, dir, on_line, cancel };

    on_line("stdout", &format!("creating a private Python {} environment", spec.python));
    uv_run
        .run(&["venv", "--managed-python", "--python", spec.python, "--no-config", &venv.to_string_lossy()])
        .await?;

    on_line("stdout", &format!("installing {} {} from hash-checked wheels", spec.package, spec.version));
    let python = venv_python(dir).to_string_lossy().into_owned();
    let lock = lock_file.to_string_lossy().into_owned();
    let mut args: Vec<&str> = vec![
        "pip", "install", "--no-config", "--python", &python, "--default-index", PYPI,
        "--require-hashes", "--only-binary", ":all:", "--no-deps",
    ];
    if !spec.build_from_sdist.is_empty() {
        // Without build isolation the build runs in the venv itself, so its tools must be there already.
        let tools_file = dir.join("build-tools.lock.txt");
        std::fs::write(&tools_file, lock_entries(spec.lock, BUILD_TOOLS)).map_err(|e| DownloadError::Unpack(e.to_string()))?;
        let tools = tools_file.to_string_lossy().into_owned();
        let mut first = args.clone();
        first.extend(["-r", &tools]);
        uv_run.run(&first).await?;
        for pkg in spec.build_from_sdist {
            args.extend(["--no-binary", pkg]);
        }
        args.push("--no-build-isolation");
    }
    args.extend(["-r", &lock]);
    uv_run.run(&args).await?;
    // The download cache is for speed on a second install; keeping it only costs disk.
    let _ = std::fs::remove_dir_all(dir.join(".uv-cache"));

    let bin = venv_bin(dir, cli_command);
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

/// Runs the pinned uv with everything it writes kept inside the install dir.
struct UvRun<'a> {
    uv: &'a Path,
    dir: &'a Path,
    on_line: &'a (dyn Fn(&'static str, &str) + Sync),
    cancel: &'a AtomicBool,
}

impl UvRun<'_> {
    async fn run(&self, args: &[&str]) -> Result<(), DownloadError> {
        let mut cmd = tokio::process::Command::new(self.uv);
        crate::backend::pane_env::sanitize_external_command(&mut cmd);
        cmd.args(args);
        // The person's own uv/pip settings must not decide where packages come from.
        for var in [
            "UV_INDEX_URL", "UV_DEFAULT_INDEX", "UV_EXTRA_INDEX_URL", "UV_INDEX", "UV_FIND_LINKS",
            "UV_PYTHON", "UV_PROJECT_ENVIRONMENT", "UV_REQUIRE_HASHES", "VIRTUAL_ENV",
            "PIP_INDEX_URL", "PIP_EXTRA_INDEX_URL", "PIP_FIND_LINKS",
        ] {
            cmd.env_remove(var);
        }
        cmd.env("UV_CACHE_DIR", self.dir.join(".uv-cache"))
            .env("UV_PYTHON_INSTALL_DIR", self.dir.join("python"))
            .env("UV_PYTHON_BIN_DIR", self.dir.join("python-bin"))
            .env("UV_NO_MODIFY_PATH", "1")
            .env("UV_PYTHON_DOWNLOADS", "automatic")
            .env("UV_NO_PROGRESS", "1")
            .env("NO_COLOR", "1")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        #[cfg(windows)]
        {
            cmd.no_window();
        }
        let mut child = cmd.spawn().map_err(|e| DownloadError::Tool(format!("cannot run uv: {e}")))?;
        let tail = parking_lot::Mutex::new(std::collections::VecDeque::<String>::new());
        let stdout = child.stdout.take().expect("piped");
        let stderr = child.stderr.take().expect("piped");
        let pump = async {
            tokio::join!(
                forward(stdout, "stdout", self.on_line, &tail),
                forward(stderr, "stderr", self.on_line, &tail)
            );
        };
        let cancelled = async {
            while !self.cancel.load(Ordering::Relaxed) {
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        };
        tokio::select! {
            _ = pump => {}
            _ = cancelled => {
                let _ = child.kill().await;
                return Err(DownloadError::Cancelled);
            }
            _ = tokio::time::sleep(STEP_TIMEOUT) => {
                let _ = child.kill().await;
                return Err(DownloadError::Tool("uv took too long and was stopped".into()));
            }
        }
        let status = child.wait().await.map_err(|e| DownloadError::Tool(e.to_string()))?;
        if status.success() {
            return Ok(());
        }
        let last = tail.lock().iter().cloned().collect::<Vec<_>>().join(" | ");
        Err(DownloadError::Tool(format!("uv exited {}: {last}", status.code().unwrap_or(-1))))
    }
}

async fn forward(
    stream: impl AsyncRead + Unpin,
    name: &'static str,
    on_line: &(dyn Fn(&'static str, &str) + Sync),
    tail: &parking_lot::Mutex<std::collections::VecDeque<String>>,
) {
    let mut lines = BufReader::new(stream).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let line = line.trim_end().to_string();
        if line.is_empty() {
            continue;
        }
        on_line(name, &line);
        let mut t = tail.lock();
        if t.len() == TAIL_LINES {
            t.pop_front();
        }
        t.push_back(line);
    }
}

#[cfg(test)]
#[path = "cli_uv_tests.rs"]
mod tests;
