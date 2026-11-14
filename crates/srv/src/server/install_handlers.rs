// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `install.start` / `install.cancel` RPC handlers.
//!
//! Phase α of `SPEC_AGENT_INSTALL_STAGE_2026_05_17.md`. Spawns
//! `npm install <package>` for the requested provider, streams every
//! line of stdout+stderr to `install_chunk` MPS events scoped to
//! `install:<sessionId>`, and emits a terminal `{ op: "done", ok,
//! error? }` event when the child exits (or the user cancels).
//!
//! Phase α scope:
//!  - npm-only install (existing per-version layout at
//!    `~/.agentmux/<version>/cli/<provider>/`).
//!  - Plain piped stdio (no PTY) — npm doesn't isatty-gate its output
//!    line-by-line, so pipes are fine here. PTY would be required for
//!    interactive post-install steps (Phase δ).
//!  - Single in-flight install per session id. The frontend modal owns
//!    the session id and prevents parallel installs at the UI layer.
//!  - Cancel kills the child via `kill_on_drop` when the abort handle
//!    fires. The partial `node_modules` dir is rm-rf'd best-effort.
//!  - No verify / doctor / post-install steps yet — those land in
//!    Phase β.

use std::sync::Arc;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::mps::Broker;
use crate::server::AppState;

pub const COMMAND_INSTALL_START: &str = "install.start";
pub const COMMAND_INSTALL_CANCEL: &str = "install.cancel";
pub const COMMAND_INSTALL_CHECK: &str = "install.check";
pub const COMMAND_RESOLVE_PREREQS: &str = "resolve.prereqs";

pub use crate::backend::install_events::{InstallChunkEvent, InstallStream};

#[derive(Debug, Deserialize, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
#[serde(rename_all = "camelCase")]
pub struct InstallStartReq {
    /// The agent CLI to install. The package, its version and the command it
    /// provides come from srv's own provider registry, never from the caller:
    /// an older client that still sends them has them ignored.
    pub provider_id: String,
}

#[derive(Debug, Deserialize, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
#[serde(rename_all = "camelCase")]
pub struct InstallCancelReq {
    pub session_id: String,
}

#[derive(Debug, Deserialize, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
#[serde(rename_all = "camelCase")]
pub struct InstallCheckReq {
    pub provider_id: String,
    pub cli_command: String,
    /// npm package name, so the check can also report which version is
    /// installed (read from that package's `package.json` in the per-version
    /// cache). Optional: callers that only care about presence may omit it,
    /// and every provider whose catalog entry has no `npmPackage` never had a
    /// managed install to inspect in the first place.
    #[serde(default)]
    pub npm_package: Option<String>,
}

/// Request for `resolve.prereqs`. Was a function-local anonymous struct.
#[derive(Debug, Deserialize, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct ResolvePrereqsReq {
    pub tools: Vec<String>,
    /// Tools from `tools` whose version should also be read, for prereqs with
    /// a minimum version (e.g. OpenClaw needs Node 24.16+). Only these are
    /// executed (`<resolved path> --version`); every other tool stays a
    /// path-only lookup. Names not also in `tools` are ignored.
    #[serde(default)]
    #[ts(optional)]
    pub versions: Option<Vec<String>>,
}

/// Result of `install.start`. Was an inline `json!({ "sessionId": .. })`.
#[derive(Debug, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
#[serde(rename_all = "camelCase")]
pub struct InstallStartResult {
    pub session_id: String,
}

/// Result of `install.check`.
#[derive(Debug, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
#[serde(rename_all = "camelCase")]
pub struct InstallCheckResult {
    pub installed: bool,
    /// Version of the managed install, when it could be read. `None` means
    /// "not known" — never "current". The caller compares this against the
    /// provider's pin (see `frontend/app/view/agent/providers/version-drift.ts`),
    /// and an absent version must resolve to `unknown` rather than quietly
    /// looking up to date.
    ///
    /// Read from `package.json` rather than by running `<cli> --version`: the
    /// agent picker calls this once per card, and spawning a process per card
    /// to answer a question a file read already answers is not a trade worth
    /// making. The user's own copy of a provider AgentMux doesn't install
    /// (`source: "system_path"`) has no such file, so it is asked instead.
    pub version: Option<String>,
    /// The CLI a launch would run, when found.
    pub path: Option<String>,
    /// `"local_install"` (AgentMux's own copy) or `"system_path"` (the user's,
    /// for a provider AgentMux doesn't install), when found.
    pub source: Option<String>,
}

/// Result of `install.cancel`.
///
/// `error` is genuinely `string | null`, NOT an optional property: the handler
/// writes an explicit `Value::Null` on success, so the key is always present.
/// The hand-written stub declared it `error?: string`, which said the key could
/// be absent — it never is. Generating the type corrects that.
#[derive(Debug, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
#[serde(rename_all = "camelCase")]
pub struct InstallCancelResult {
    pub success: bool,
    pub error: Option<String>,
}

/// One tool's resolution in `resolve.prereqs`.
#[derive(Debug, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct PrereqToolResolution {
    pub tool: String,
    pub found: bool,
    /// Always present, null when the tool was not found.
    pub path: Option<String>,
    /// The tool's version, when the request asked for it (`versions`) and
    /// it could be read. Null means "not asked" or "unknown" — never
    /// "new enough"; the caller decides how to treat an unknown version.
    pub version: Option<String>,
}

/// Result of `resolve.prereqs`.
#[derive(Debug, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct ResolvePrereqsResult {
    pub results: Vec<PrereqToolResolution>,
}

/// Per-session abort handle so `install.cancel` can kill an in-flight
/// install. Also tracks `active_providers` so concurrent sessions for
/// the *same* provider directory are rejected — without that, cancel
/// of one would `rm_rf` the shared dir mid-install for the other.
/// `parking_lot::Mutex` since the engine is sync at the handler
/// boundary.
#[derive(Default)]
pub struct InstallSessionRegistry {
    sessions: Mutex<std::collections::HashMap<String, tokio::sync::oneshot::Sender<()>>>,
    active_providers: Mutex<std::collections::HashSet<String>>,
    /// Same idea as `active_providers`, but a separate set keyed by
    /// system-tool id (`"git"`, `"node"`, …) rather than provider id — a
    /// distinct namespace so a future provider literally named e.g. "git"
    /// can never collide with a system-tool claim. See
    /// `system_install_handlers.rs` / `SPEC_SYSTEM_TOOLCHAIN_INSTALLER_2026_08_24.md`.
    active_system_tools: Mutex<std::collections::HashSet<String>>,
}

impl InstallSessionRegistry {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub(crate) fn insert(&self, session_id: String, tx: tokio::sync::oneshot::Sender<()>) {
        self.sessions.lock().insert(session_id, tx);
    }

    /// Try to claim a provider; returns false if another session is
    /// already installing this provider.
    pub(crate) fn try_claim_provider(&self, provider_id: &str) -> bool {
        self.active_providers.lock().insert(provider_id.to_string())
    }

    pub(crate) fn release_provider(&self, provider_id: &str) {
        self.active_providers.lock().remove(provider_id);
    }

    /// Try to claim a system tool (git/node/npm/python); returns false if
    /// another session is already installing this tool.
    pub(crate) fn try_claim_system_tool(&self, tool_id: &str) -> bool {
        self.active_system_tools.lock().insert(tool_id.to_string())
    }

    pub(crate) fn release_system_tool(&self, tool_id: &str) {
        self.active_system_tools.lock().remove(tool_id);
    }

    pub(crate) fn cancel(&self, session_id: &str) -> bool {
        if let Some(tx) = self.sessions.lock().remove(session_id) {
            let _ = tx.send(());
            true
        } else {
            false
        }
    }

    pub(crate) fn drop_session(&self, session_id: &str) {
        self.sessions.lock().remove(session_id);
    }
}

/// Provider ids feed into the install dir path; reject anything that
/// could escape `~/.agentmux/<version>/cli/<provider>/`.
fn is_safe_provider_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// CLI command names are joined into the bin-resolution path; same
/// allowlist as provider ids, plus `.` (some real CLIs include dots,
/// e.g. `eslint.cmd`).
fn is_safe_cli_command(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
        && !s.contains("..")
}

/// The registry entry an install of `provider_id` may use, or why there is none.
/// What gets installed is decided here, from srv's own registry of known agent
/// CLIs: a caller names an id and nothing else. Shared by `install.start` and
/// `resolvecli`.
pub(crate) fn installable_provider(
    provider_id: &str,
) -> Result<&'static crate::backend::providers::ProviderConfig, String> {
    if !is_safe_provider_id(provider_id) {
        return Err(format!("invalid provider id {provider_id:?} — must match [a-zA-Z0-9_-]+"));
    }
    let provider = crate::backend::providers::get_provider(provider_id)
        .ok_or_else(|| format!("{provider_id:?} is not a known agent CLI"))?;
    if provider.npm_package.is_empty() && !crate::backend::cli_download::is_managed_install(provider.id) {
        return Err(format!("provider {} is not installed by AgentMux (no npm package or download)", provider.id));
    }
    Ok(provider)
}

/// The pinned CLI version an install of `provider_id` uses: the backend
/// provider registry's pin (the source of truth), else the caller's.
pub(super) fn effective_pin(provider_id: &str, requested: &str) -> String {
    crate::backend::providers::get_provider(provider_id)
        .map(|p| p.pinned_version.to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| requested.to_string())
}

/// Where a new install of `provider_id` goes: the shared, pinned-version dir
/// (`backend::cli_install`), which is also what `resolvecli` and the launch
/// path look in first. Honors portable / installed mode + the
/// `AGENTMUX_HOME_OVERRIDE` test override via `DataPaths::from_env()`.
pub(super) fn provider_install_dir(provider_id: &str, pinned_version: &str) -> Option<std::path::PathBuf> {
    let paths = agentmux_common::DataPaths::from_env()?;
    Some(crate::backend::cli_install::install_dir(
        &paths,
        provider_id,
        pinned_version,
    ))
}

/// Every dir an existing install of `provider_id` may be in, in lookup order:
/// the shared pinned-version dir, then the legacy per-AgentMux-version one.
fn provider_installed_dirs(provider_id: &str) -> Vec<std::path::PathBuf> {
    let Some(paths) = agentmux_common::DataPaths::from_env() else {
        return Vec::new();
    };
    let pin = effective_pin(provider_id, "");
    // The shared dir only once its install is marked complete — a shim alone
    // may be another instance's install in progress (Codex P2 on #3927).
    crate::backend::cli_install::shared_cli_dir(&paths, provider_id, &pin)
        .filter(|d| crate::backend::cli_install::is_complete(d))
        .into_iter()
        .chain(std::iter::once(
            crate::backend::cli_install::legacy_cli_dir(&paths, provider_id),
        ))
        .collect()
}

/// Returns the path to the installed CLI binary if present in the
/// per-version cache, else None. Used by `install.check`.
/// Locate a system tool (e.g. `git`, `gh`) on this process's PATH and return
/// its absolute path; None when it isn't there.
///
/// Used by `resolve.prereqs` to pre-launch-check whether a provider's
/// system dependencies are installed. The lookup is path-only — never
/// executes the tool. (`resolve.prereqs` runs `--version` separately, and
/// only for tools the caller lists in `versions`.)
/// See SPEC_PROVIDER_SYSTEM_PREREQS_2026_05_18.md.
pub(crate) async fn resolve_tool_path(tool: &str) -> Option<String> {
    // In-process PATH search (the `which` crate: PATHEXT-aware on Windows),
    // not a spawned `which`/`where`. The spawn made "the lookup program is
    // missing" indistinguishable from "the tool is missing": on a minimal
    // Linux without `which`, node/npm read as absent and launches were
    // blocked (Codex P2 on #3891). No console window to suppress either.
    let tool = tool.to_string();
    tokio::task::spawn_blocking(move || find_on_path(&tool, std::env::var_os("PATH").as_deref()))
        .await
        .ok()
        .flatten()
}

/// Version of the prereq binary at `path` (already resolved on PATH), via
/// `--version`, bounded so a hung binary can't stall the launch check.
async fn probe_prereq_version(path: &str) -> Option<String> {
    let path = path.to_string();
    let probe = tokio::task::spawn_blocking(move || {
        crate::backend::tool_store::probe_version(&path, &Some("--version".to_string()))
    });
    tokio::time::timeout(std::time::Duration::from_secs(5), probe)
        .await
        .ok()?
        .ok()
        .flatten()
}

/// `tool`'s absolute path in the directories of `path` (a PATH-style list).
fn find_on_path(tool: &str, path: Option<&std::ffi::OsStr>) -> Option<String> {
    let cwd = std::env::current_dir().ok()?;
    which::which_in(tool, path, cwd).ok().map(|p| p.to_string_lossy().into_owned())
}

/// Version of a managed provider install, read from the installed package's
/// own `package.json`. `None` for anything unreadable — a missing directory, a
/// partial install, unparseable JSON — all of which mean "we do not know",
/// which is a distinct answer from "up to date" and must stay distinct.
///
/// `npm_package` becomes a path segment, so the join goes through
/// [`crate::backend::base::safe_join_within_base`] rather than an ad-hoc
/// check. An earlier revision hand-rolled the validation (empty / `..` /
/// leading `/` / leading `\` / NUL) and missed the Windows **drive-letter**
/// form: `PathBuf::push("C:\\Users\\…")` REPLACES the whole path rather than
/// appending, so `npm_package = "C:\\Users\\Victim\\AppData"` escaped the
/// install dir entirely and read an attacker-chosen `package.json`
/// (ReAgent P1 on #3350). That helper already rejects drive-letter and
/// drive-relative prefixes, UNC roots, rooted paths and `..`, and accepts
/// both separators — which is exactly why it exists and why this must not
/// grow a second copy of the same reasoning.
///
/// Scoped names (`@scope/name`) are legitimate and produce one nested
/// directory; the helper splits on `/` and `\` and handles them.
///
/// `dir` is the install whose shim `install.check` resolved: the version is
/// read from that same install, never from another dir's leftover manifest —
/// otherwise a shared dir with a manifest but no shim would lend its version
/// to the legacy binary actually in use and hide drift (Codex P2 on #3927).
fn installed_version_in(dir: &std::path::Path, npm_package: &str) -> Option<String> {
    let base = dir.join("node_modules");
    let pkg_dir = crate::backend::base::safe_join_within_base(&base, npm_package).ok()?;
    manifest_version(&pkg_dir.join("package.json"))
}

fn manifest_version(manifest: &std::path::Path) -> Option<String> {
    let raw = std::fs::read_to_string(manifest).ok()?;
    let parsed: serde_json::Value = serde_json::from_str(&raw).ok()?;
    parsed.get("version")?.as_str().map(|s| s.to_string())
}

pub(super) fn installed_bin_in(dir: &std::path::Path, cli_command: &str) -> Option<std::path::PathBuf> {
    for managed in [
        crate::backend::cli_download::download_bin(dir, cli_command),
        crate::backend::cli_uv::venv_bin(dir, cli_command),
    ] {
        if managed.is_file() {
            return Some(managed);
        }
    }
    let bin_dir = dir.join("node_modules").join(".bin");
    let candidates: &[&str] = if cfg!(windows) {
        &[".cmd", ".exe", ""]
    } else {
        &["", ".cmd"]
    };
    candidates
        .iter()
        .map(|suffix| bin_dir.join(format!("{cli_command}{suffix}")))
        .find(|p| p.is_file())
}

/// The first of `dirs` holding `cli_command`'s shim — the install a launch
/// would use. Pure over `dirs`, for tests.
fn installed_dir_among(
    dirs: Vec<std::path::PathBuf>,
    cli_command: &str,
) -> Option<std::path::PathBuf> {
    dirs.into_iter()
        .find(|dir| installed_bin_in(dir, cli_command).is_some())
}

fn resolve_installed_dir(provider_id: &str, cli_command: &str) -> Option<std::path::PathBuf> {
    installed_dir_among(provider_installed_dirs(provider_id), cli_command)
}

pub fn register_install_handlers(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let registry = state.install_sessions.clone();
    let broker = state.broker.clone();
    engine.register_typed(COMMAND_INSTALL_START, move |req: InstallStartReq, _ctx| {
        let registry = registry.clone();
        let broker = broker.clone();
        async move { install_start(&registry, &broker, req) }
    });

    engine.register_typed(COMMAND_INSTALL_CHECK, move |req: InstallCheckReq, _ctx| async move {
        install_check(req).await
    });

    engine.register_typed(COMMAND_RESOLVE_PREREQS, move |req: ResolvePrereqsReq, _ctx| async move {
        resolve_prereqs(req).await
    });

    let registry = state.install_sessions.clone();
    engine.register_typed(COMMAND_INSTALL_CANCEL, move |req: InstallCancelReq, _ctx| {
        let registry = registry.clone();
        async move { Ok(install_cancel(&registry, req)) }
    });
}

/// `install.start`: claim the provider, record a session, and run the install in the background.
pub(crate) fn install_start(
    registry: &Arc<InstallSessionRegistry>,
    broker: &Arc<Broker>,
    req: InstallStartReq,
) -> Result<InstallStartResult, String> {
    let provider = installable_provider(&req.provider_id).map_err(|e| format!("install.start: {e}"))?;
    if !registry.try_claim_provider(provider.id) {
        return Err(format!("install.start: provider {} is already being installed in another session", provider.id));
    }
    let session_id = format!("install-{}", uuid::Uuid::new_v4());
    tracing::info!(
        session_id = %session_id,
        provider_id = %provider.id,
        npm_package = %provider.npm_package,
        pinned_version = %provider.pinned_version,
        "install.start"
    );

    let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();
    registry.insert(session_id.clone(), cancel_tx);

    super::install_task::spawn_install_task(
        broker.clone(),
        registry.clone(),
        session_id.clone(),
        provider.id.to_string(),
        provider.cli_command.to_string(),
        provider.npm_package.to_string(),
        provider.pinned_version.to_string(),
        cancel_rx,
    );

    Ok(InstallStartResult { session_id })
}

/// `install.check`: whether the provider's CLI is there, and which version.
pub(crate) async fn install_check(req: InstallCheckReq) -> Result<InstallCheckResult, String> {
    if !is_safe_provider_id(&req.provider_id) {
        return Err(format!("install.check: invalid provider id {:?}", req.provider_id));
    }
    if !is_safe_cli_command(&req.cli_command) {
        return Err(format!("install.check: invalid cli command {:?}", req.cli_command));
    }
    // The one lookup a launch uses (`locate_provider_cli`), so the
    // agent picker and the Toolchain pane give the same answer: an
    // npm provider's own install only, and the user's copy only for
    // a provider AgentMux doesn't install.
    let located = crate::server::cli_handlers::locate_provider_cli(&req.provider_id).await;
    let version = match &located {
        // Read from the install's package.json, not by running it
        // (one check per picker card): only meaningful when the
        // CLI is actually there, and read from that same install.
        Some(l) if l.source == crate::server::cli_handlers::CliSource::LocalInstall => {
            resolve_installed_dir(&req.provider_id, &req.cli_command).and_then(|dir| match req.npm_package.as_deref() {
                Some(pkg) => installed_version_in(&dir, pkg),
                // A release download has no package.json: its completion marker holds the pin.
                None => std::fs::read_to_string(dir.join(crate::backend::cli_install::COMPLETE_MARKER))
                    .ok()
                    .map(|v| v.trim().to_string())
                    .filter(|v| !v.is_empty()),
            })
        }
        // The user's own copy has no package.json of ours: ask it.
        // Only the few providers AgentMux doesn't install get here.
        Some(l) => Some(crate::server::cli_handlers::get_cli_version(&l.path).await)
            .filter(|v| !v.is_empty() && v != "unknown"),
        None => None,
    };
    Ok(InstallCheckResult {
        installed: located.is_some(),
        version,
        path: located.as_ref().map(|l| l.path.clone()),
        source: located.map(|l| l.source.as_str().to_string()),
    })
}

/// `resolve.prereqs`: where each named tool is, and its version for those that asked.
pub(crate) async fn resolve_prereqs(req: ResolvePrereqsReq) -> Result<ResolvePrereqsResult, String> {
    let mut results = Vec::with_capacity(req.tools.len());
    for tool in &req.tools {
        if !is_safe_cli_command(tool) {
            return Err(format!("resolve.prereqs: invalid tool name {:?}", tool));
        }
        let path = resolve_tool_path(tool).await;
        let wants_version = req.versions.as_deref().is_some_and(|v| v.iter().any(|t| t == tool));
        let version = match (&path, wants_version) {
            (Some(p), true) => probe_prereq_version(p).await,
            _ => None,
        };
        results.push(PrereqToolResolution { tool: tool.clone(), found: path.is_some(), path, version });
    }
    Ok(ResolvePrereqsResult { results })
}

/// `install.cancel`.
pub(crate) fn install_cancel(registry: &Arc<InstallSessionRegistry>, req: InstallCancelReq) -> InstallCancelResult {
    let ok = registry.cancel(&req.session_id);
    InstallCancelResult {
        success: ok,
        error: if ok { None } else { Some(format!("unknown or already-terminal session: {}", req.session_id)) },
    }
}

#[cfg(test)]
mod tests {
    use super::{find_on_path, is_safe_cli_command, is_safe_provider_id};

    /// The lookup is in-process: a PATH holding only the tool (no `which`
    /// binary anywhere on it) still finds it. Codex P2 on #3891.
    #[test]
    fn find_on_path_needs_no_which_binary() {
        let dir = tempfile::tempdir().unwrap();
        let name = if cfg!(windows) { "node.exe" } else { "node" };
        let exe = dir.path().join(name);
        std::fs::write(&exe, b"").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let found = find_on_path("node", Some(dir.path().as_os_str())).expect("node found");
        assert_eq!(std::path::Path::new(&found), exe.as_path());
        assert!(find_on_path("npm", Some(dir.path().as_os_str())).is_none());
    }

    #[test]
    fn safe_provider_ids_accepted() {
        for id in ["claude", "claude-code", "open_claw", "Codex42"] {
            assert!(is_safe_provider_id(id), "{id} should be accepted");
        }
    }

    #[test]
    fn unsafe_provider_ids_rejected() {
        for id in [
            "",
            "../escape",
            "a/b",
            "a\\b",
            "a b",
            ".",
            "..",
            "a..b",
            "a/../b",
            "\0null",
            &"x".repeat(65),
        ] {
            assert!(!is_safe_provider_id(id), "{id:?} should be rejected");
        }
    }

    #[test]
    fn safe_cli_commands_accepted() {
        for cmd in ["claude", "claude-code", "kimi.cmd", "agentmux-srv", "open_claw"] {
            assert!(is_safe_cli_command(cmd), "{cmd} should be accepted");
        }
    }

    #[test]
    fn unsafe_cli_commands_rejected() {
        for cmd in [
            "",
            "../etc/passwd",
            "../../etc/passwd",
            "a/b",
            "a\\b",
            "a b",
            "..",
            "a..b",
            "a/../b",
            "\0null",
            &"x".repeat(65),
        ] {
            assert!(!is_safe_cli_command(cmd), "{cmd:?} should be rejected");
        }
    }
}

// Request/result shape tests for the four install-related commands.
#[cfg(test)]
mod req_shape_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn install_chunk_events_serialize_as_before() {
        let line = InstallChunkEvent::line("s1", "added 3 packages".into(), InstallStream::Stderr);
        assert_eq!(serde_json::to_value(&line).unwrap(), json!({ "sessionId": "s1", "line": "added 3 packages", "stream": "stderr" }));
        let done = InstallChunkEvent::done("s1", false, json!(None::<String>));
        assert_eq!(serde_json::to_value(&done).unwrap(), json!({ "sessionId": "s1", "op": "done", "ok": false, "error": null }));
    }

    // These structs rely on rename_all = "camelCase" to match the wire. Drop
    // that attribute and everything still compiles and the binding still
    // generates -- it just silently stops parsing real payloads.
    #[test]
    fn the_requests_accept_the_camel_case_payloads_the_stub_sends() {
        let start: InstallStartReq = serde_json::from_value(json!({ "providerId": "claude" })).expect("install.start");
        assert_eq!(start.provider_id, "claude");

        // A client that still sends the package fields is understood; they are not read.
        let older: InstallStartReq = serde_json::from_value(json!({
            "providerId": "claude",
            "cliCommand": "claude",
            "npmPackage": "some-other-package",
            "pinnedVersion": "9.9.9",
        }))
        .expect("install.start from an older client");
        assert_eq!(older.provider_id, "claude");

        serde_json::from_value::<InstallCancelReq>(json!({"sessionId": "s1"}))
            .expect("install.cancel");
        serde_json::from_value::<InstallCheckReq>(
            json!({"providerId": "claude", "cliCommand": "claude"}),
        )
        .expect("install.check");
        serde_json::from_value::<ResolvePrereqsReq>(json!({"tools": ["node", "git"]}))
            .expect("resolve.prereqs");
        let with_versions = serde_json::from_value::<ResolvePrereqsReq>(
            json!({"tools": ["node", "git"], "versions": ["node"]}),
        )
        .expect("resolve.prereqs with versions");
        assert_eq!(with_versions.versions.as_deref(), Some(&["node".to_string()][..]));

        assert!(
            serde_json::from_value::<InstallCancelReq>(json!({"session_id": "s1"})).is_err(),
            "snake_case is not the wire format for these"
        );
    }

    // THE DRIFT THIS SLICE FIXES. The handler writes an explicit null for
    // `error` on success, so the key is ALWAYS present. The hand-written stub
    // said `error?: string`, i.e. the key may be absent -- it never is.
    #[test]
    fn cancel_result_always_carries_the_error_key() {
        let ok = serde_json::to_value(InstallCancelResult { success: true, error: None })
            .expect("serializable");
        assert_eq!(ok, json!({"success": true, "error": null}));
        assert!(
            ok.as_object().unwrap().contains_key("error"),
            "error is `string | null`, not an optional property"
        );
    }

    #[test]
    fn prereq_results_carry_a_nullable_path() {
        let v = serde_json::to_value(ResolvePrereqsResult {
            results: vec![PrereqToolResolution {
                tool: "node".to_string(),
                found: false,
                path: None,
                version: None,
            }],
        })
        .expect("serializable");
        assert_eq!(
            v,
            json!({"results": [{"tool": "node", "found": false, "path": null, "version": null}]})
        );
    }
}

#[cfg(test)]
mod installed_version_tests {
    use super::installed_version_in;

    fn read_installed_version(_provider: &str, npm_package: &str) -> Option<String> {
        installed_version_in(
            &std::env::temp_dir().join("agentmux-no-such-install"),
            npm_package,
        )
    }

    /// The npm package name becomes a path segment, so a hostile or malformed
    /// one must be rejected BEFORE any filesystem access. These cases return
    /// early on the validation branch, so they hold regardless of whether a
    /// data dir exists in the test environment — which is also why they are
    /// the part worth pinning: the happy path needs a real per-version cache,
    /// but the rejection path is pure and is the one with teeth.
    ///
    /// The drive-letter cases are the ones an earlier revision of this
    /// function got wrong (ReAgent P1 on #3350): `PathBuf::push` REPLACES the
    /// path when the pushed component carries a drive prefix, so these escaped
    /// the install dir entirely while a `starts_with('/')`/`starts_with('\\')`
    /// check waved them through. They are listed explicitly so a future
    /// refactor back to a hand-rolled check fails here instead of shipping.
    #[test]
    fn rejects_path_escapes_and_malformed_package_names() {
        for bad in [
            "",
            "..",
            "../../../etc/passwd",
            "@scope/../../escape",
            "/absolute/path",
            "\\windows\\absolute",
            "has\0null",
            "C:\\Users\\Victim\\AppData",
            "C:/some/dir",
            "C:payload.txt",
            "\\\\server\\share\\evil",
        ] {
            assert_eq!(
                read_installed_version("claude", bad),
                None,
                "expected {bad:?} to be rejected"
            );
        }
    }

    /// A legitimate scoped name (`@anthropic-ai/claude-code`) must NOT be
    /// caught by the escape check — it is the common case, and one nested
    /// directory is exactly what npm creates for it. Returns None here only
    /// because no such install exists in the test environment; the assertion
    /// that matters is that it does not panic and is not rejected for the
    /// wrong reason.
    #[test]
    fn scoped_package_names_are_not_treated_as_escapes() {
        let _ = read_installed_version("claude", "@anthropic-ai/claude-code");
    }

    /// Codex P2 on #3927: the version comes from the install whose shim is
    /// used. A completed shared dir that lost its shim (manifest still there)
    /// must not lend its version to the legacy binary actually launched.
    #[test]
    fn the_version_is_read_from_the_install_whose_shim_is_used() {
        use super::installed_dir_among;
        let (pkg, cli) = ("@anthropic-ai/claude-code", "claude");
        let manifest = |dir: &std::path::Path| {
            dir.join("node_modules")
                .join("@anthropic-ai")
                .join("claude-code")
                .join("package.json")
        };
        let install = |dir: &std::path::Path, version: &str, shim: bool| {
            let m = manifest(dir);
            std::fs::create_dir_all(m.parent().unwrap()).unwrap();
            std::fs::write(&m, format!(r#"{{"version":"{version}"}}"#)).unwrap();
            if shim {
                let bin = dir.join("node_modules").join(".bin");
                std::fs::create_dir_all(&bin).unwrap();
                let name = if cfg!(windows) {
                    "claude.cmd"
                } else {
                    "claude"
                };
                std::fs::write(bin.join(name), "").unwrap();
            }
        };
        let tmp = tempfile::tempdir().unwrap();
        let (shared, legacy) = (tmp.path().join("shared"), tmp.path().join("legacy"));
        install(&shared, "2.1.280", false);
        install(&legacy, "2.1.100", true);
        let dir = installed_dir_among(vec![shared.clone(), legacy.clone()], cli)
            .expect("the legacy shim");
        assert_eq!(dir, legacy);
        assert_eq!(installed_version_in(&dir, pkg).as_deref(), Some("2.1.100"));

        // With both shims present the shared install wins, with its own version.
        install(&shared, "2.1.280", true);
        let dir = installed_dir_among(vec![shared.clone(), legacy], cli).unwrap();
        assert_eq!(dir, shared);
        assert_eq!(installed_version_in(&dir, pkg).as_deref(), Some("2.1.280"));

        // The used install's manifest unreadable: "unknown", not another
        // install's version.
        std::fs::write(manifest(&shared), "not json").unwrap();
        assert_eq!(installed_version_in(&shared, pkg), None);
    }

    /// A uv venv install (Aider, Kimi) is an install too, or `install.start`
    /// rebuilds a finished one and `install.check` loses its version (#6618).
    #[test]
    fn a_uv_venv_and_a_release_download_count_as_installed() {
        use super::installed_dir_among;
        let tmp = tempfile::tempdir().unwrap();
        let (venv, download, empty) = (tmp.path().join("venv"), tmp.path().join("dl"), tmp.path().join("none"));
        for (dir, bin) in [
            (&venv, crate::backend::cli_uv::venv_bin(&venv, "aider")),
            (&download, crate::backend::cli_download::download_bin(&download, "goose")),
        ] {
            std::fs::create_dir_all(bin.parent().unwrap()).unwrap();
            std::fs::write(&bin, "").unwrap();
            assert!(dir.exists());
        }
        std::fs::create_dir_all(&empty).unwrap();
        assert_eq!(installed_dir_among(vec![empty.clone(), venv.clone()], "aider"), Some(venv));
        assert_eq!(installed_dir_among(vec![empty.clone(), download.clone()], "goose"), Some(download));
        assert_eq!(installed_dir_among(vec![empty], "aider"), None);
    }
}

#[cfg(test)]
mod registry_only_tests {
    use super::*;

    async fn start(engine: &Arc<WshRpcEngine>, out: &mut tokio::sync::mpsc::UnboundedReceiver<crate::backend::rpc_types::RpcMessage>, data: serde_json::Value) -> crate::backend::rpc_types::RpcMessage {
        engine.handle_message(crate::backend::rpc_types::RpcMessage {
            command: COMMAND_INSTALL_START.to_string(),
            reqid: "r1".to_string(),
            data: Some(data),
            ..Default::default()
        });
        tokio::time::timeout(std::time::Duration::from_secs(2), out.recv()).await.unwrap().unwrap()
    }

    #[test]
    fn every_installable_provider_is_installed_from_its_own_registry_entry() {
        let mut seen = 0;
        for p in crate::backend::providers::all_providers().filter(|p| !p.npm_package.is_empty()) {
            let got = installable_provider(p.id).expect(p.id);
            assert_eq!(got.id, p.id);
            assert_eq!(got.npm_package, p.npm_package);
            seen += 1;
        }
        assert!(seen > 0);
    }

    /// Every provider with no npm package is installed by AgentMux's own download or uv install, so
    /// each one is accepted here (the refusal for one that is not has no provider to try it on).
    #[test]
    fn every_provider_with_no_npm_package_is_installed_by_a_download_or_uv() {
        let mut seen = 0;
        for p in crate::backend::providers::all_providers().filter(|p| p.npm_package.is_empty()) {
            assert!(crate::backend::cli_download::is_managed_install(p.id), "{} has no npm package and no managed install", p.id);
            assert!(installable_provider(p.id).is_ok(), "{}", p.id);
            seen += 1;
        }
        assert!(seen >= 6);
    }

    #[test]
    fn an_id_outside_the_registry_is_refused() {
        for id in ["", "no-such-agent", "left-pad", "../claude", "claude/../../x", "@scope/pkg", "claude ", "C:\\x"] {
            assert!(installable_provider(id).is_err(), "{id:?}");
        }
    }

    #[tokio::test]
    async fn install_start_refuses_an_unknown_provider_even_when_it_names_a_package() {
        let state = crate::server::tests::test_state();
        let (engine, mut out) = WshRpcEngine::new();
        register_install_handlers(&engine, &state);
        let resp = start(
            &engine,
            &mut out,
            serde_json::json!({
                "providerId": "no-such-agent",
                "cliCommand": "no-such-agent",
                "npmPackage": "left-pad",
                "pinnedVersion": "1.0.0",
            }),
        )
        .await;
        assert!(resp.data.is_none(), "{resp:?}");
        assert!(resp.error.contains("not a known agent CLI"), "{resp:?}");
        assert!(state.install_sessions.sessions.lock().is_empty(), "no session was started");
        assert!(state.install_sessions.active_providers.lock().is_empty(), "nothing was claimed");
    }

    #[tokio::test]
    async fn cancelling_a_session_nobody_started_says_so() {
        let state = crate::server::tests::test_state();
        let result = install_cancel(&state.install_sessions, InstallCancelReq { session_id: "install-nope".into() });
        assert!(!result.success);
        assert_eq!(result.error.as_deref(), Some("unknown or already-terminal session: install-nope"));
    }

    #[tokio::test]
    async fn install_check_refuses_an_unsafe_provider_id_or_cli_command() {
        let req = |provider: &str, cli: &str| InstallCheckReq {
            provider_id: provider.into(),
            cli_command: cli.into(),
            npm_package: None,
        };
        let err = install_check(req("../claude", "claude")).await.err().unwrap();
        assert!(err.starts_with("install.check: invalid provider id"), "{err}");
        let err = install_check(req("claude", "claude; rm -rf /")).await.err().unwrap();
        assert!(err.starts_with("install.check: invalid cli command"), "{err}");
    }

    #[tokio::test]
    async fn resolve_prereqs_refuses_an_unsafe_tool_name_and_answers_nothing_for_no_tools() {
        let err = resolve_prereqs(ResolvePrereqsReq { tools: vec!["git".into(), "a b".into()], versions: None }).await.err().unwrap();
        assert!(err.starts_with("resolve.prereqs: invalid tool name"), "{err}");
        let none = resolve_prereqs(ResolvePrereqsReq { tools: vec![], versions: None }).await.unwrap();
        assert!(none.results.is_empty());
    }

    #[tokio::test]
    async fn install_start_refuses_without_claiming_when_called_directly() {
        let state = crate::server::tests::test_state();
        let req = InstallStartReq { provider_id: "no-such-agent".into() };
        let err = install_start(&state.install_sessions, &state.broker, req).err().unwrap();
        assert!(err.starts_with("install.start: ") && err.contains("not a known agent CLI"), "{err}");
        assert!(state.install_sessions.active_providers.lock().is_empty());
    }
}
