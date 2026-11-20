// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

#[cfg(windows)]
use agentmux_common::win32::NoWindow;
use std::sync::Arc;

use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::rpc_types::{
    ToolchainEnvReq, ToolchainEnvResult, ToolchainVersionsReq,
    ToolchainPruneItem, ToolchainPruneReq, ToolchainPruneResult, ToolchainPruneSkip,
    WidgetApiResult, WidgetHealthResult,
    CheckCliAuthResult, CommandCheckCliAuthData, CommandResolveCliData, CommandRunCliLoginData,
    EnsureProviderAuthDirReq, EnsureProviderAuthDirResult, ResolveCliResult, RunCliLoginResult,
    COMMAND_CHECK_CLI_AUTH, COMMAND_ENSURE_PROVIDER_AUTH_DIR, COMMAND_RESOLVE_CLI,
};

use super::AppState;

/// Register CLI-related RPC handlers (resolvecli, checkcliauth, runclilogin).
pub fn register_cli_handlers(engine: &Arc<WshRpcEngine>, state: &AppState) {
    // resolvecli → detect or install a CLI tool for an agent provider.
    // Installs are shared across AgentMux versions, keyed by the provider's
    // pinned CLI version: `<shared_dir>/cli/<provider>/<pinned_version>/`,
    // with the legacy `<home>/instances/v<AGENTMUX_VERSION>/cli/<provider>/`
    // as a read-only fallback. One definition for every resolver
    // (`install.start` / `install.check`, `agent.open`, the picker's
    // read-only lookup): `backend::cli_install`.
    // Never falls back to system PATH for npm-backed providers.
    let broker_resolve = state.broker.clone();
    let filestore_resolve = state.filestore.clone();
    let mstore_resolve = state.mstore.clone();
    engine.register_typed(
        COMMAND_RESOLVE_CLI,
        move |cmd: CommandResolveCliData, _ctx| {
            let notices = CliNotices {
                broker: broker_resolve.clone(),
                filestore: filestore_resolve.clone(),
                mstore: mstore_resolve.clone(),
                block_id: notice_block_id(&cmd),
                agent_id: cmd.agent_id.clone().filter(|a| !a.trim().is_empty()),
                provider: cmd.provider_id.clone(),
            };
            resolve_cli(cmd, broker_resolve.clone(), notices)
        },
    );

    // checkcliauth → check if a CLI tool is authenticated
    // For Claude: reads ~/.claude/.credentials.json directly (instant, no subprocess).
    // For other providers: falls back to running the CLI auth check command.
    engine.register_typed(COMMAND_CHECK_CLI_AUTH, |cmd: CommandCheckCliAuthData, _ctx| check_cli_auth(cmd));

    // provider.ensureauthdir → create the provider's default (shared) auth dir
    // and apply its isolation guarantees, exactly as agent open does; returns
    // the path for the launch env (e.g. CLAUDE_CONFIG_DIR). Replaces the CEF
    // host's `ensure_auth_dir`, which only created the dir.
    engine.register_typed(COMMAND_ENSURE_PROVIDER_AUTH_DIR, |req: EnsureProviderAuthDirReq, _ctx| {
        ensure_provider_auth_dir(req)
    });

    // runclilogin → spawn CLI login flow, extract OAuth URL from output, return immediately
    engine.register_typed("runclilogin", |cmd: CommandRunCliLoginData, _ctx| run_cli_login(cmd));

    // toolchain.env — report the environment the srv resolves tools in: the
    // effective PATH, how it was derived (set by the host/srv PATH enricher,
    // see SPEC_TOOLCHAIN_MANAGER §3), and OS/arch. Powers the Toolchain
    // modal's Environment section so PATH problems are diagnosable.
    //
    // `Option<_>` -- see `ToolchainEnvReq`: it is what makes both encodings
    // of "no argument" deserialize.
    engine.register_typed("toolchain.env", |_req: Option<ToolchainEnvReq>, _ctx| toolchain_env());

    // toolchain.prune — find (dry run, the default) or remove (an explicit
    // `dry_run: false`, from a user action) provider CLI installs that nothing
    // has used for 30 days and nothing is running. Never runs on its own.
    // SPEC_RUNTIME_MENU_REMAINING_GAPS_2026_10_01.md §5.
    let mstore_prune = state.mstore.clone();
    engine.register_typed("toolchain.prune", move |req: Option<ToolchainPruneReq>, _ctx| {
        toolchain_prune(req, mstore_prune.clone())
    });

    // widget.health — HTTP liveness probe for an external widget server running
    // on localhost. The frontend passes { port, health_check_path,
    // health_check_body_contains? } and gets back { healthy, status_code }.
    // Connection-refused or timeout → { healthy: false } (not an RPC error).
    // health_check_body_contains lets callers distinguish services that share
    // a default port (e.g. Flowise and Grafana both default to 3000).
    // Registered with `serde_json::Value` as the REQUEST type rather than a
    // struct. `Value` deserializes from anything, so this is exactly as
    // tolerant as the untyped handler it replaces -- and the handler needs that
    // tolerance: a bad port or path is answered with `healthy: false`, not an
    // RPC error, which a typed request would turn into a deserialize failure.
    //
    // What it buys over `register_handler` is the RESPONSE type, which is the
    // half that was actually drifting.
    engine.register_typed("widget.health", |data: serde_json::Value, _ctx| widget_health(data));

    // widget.api — HTTP proxy to a widget's local server. Bypasses browser CORS
    // restrictions: the frontend sends { port, path, method?, headers?, body? }
    // and gets back { ok, status_code, body, error? }. Agents use this to call
    // ComfyUI /prompt, Grafana /api/query, etc. without needing a CORS header.
    // 30-second timeout accommodates generative tasks (image synthesis, etc.).
    // `Value` request for the same reason as `widget.health` above: an invalid
    // port or path is a `ok: false` ANSWER, not a rejected request.
    engine.register_typed("widget.api", |data: serde_json::Value, _ctx| widget_api(data));

    // toolchain.versions — fetch the latest published version for a list of npm
    // packages from the npm registry. Input: { packages: [{id, package}] }.
    // Output: { id: "x.y.z" | null, ... }. Each lookup is independent — a
    // network error for one package yields null for that entry, not a failure.
    // 5-second per-request timeout; all lookups run concurrently.
    engine.register_typed("toolchain.versions", |req: ToolchainVersionsReq, _ctx| toolchain_versions(req));
}

/// `resolvecli`: find the CLI, else say how to install it, else install it.
async fn resolve_cli(
    cmd: CommandResolveCliData,
    broker: Arc<crate::backend::mps::Broker>,
    notices: CliNotices,
) -> Result<ResolveCliResult, String> {
    const AGENTMUX_VERSION: &str = env!("CARGO_PKG_VERSION");

    tracing::info!(
        provider = %cmd.provider_id,
        cli = %cmd.cli_command,
        block_id = %cmd.block_id,
        agentmux_version = AGENTMUX_VERSION,
        "ResolveCli"
    );

    // Only AgentMux's own registry names a package to install: an id
    // outside it that sends one is refused before anything is looked up.
    if !cmd.npm_package.is_empty() && crate::backend::providers::get_provider(&cmd.provider_id).is_none() {
        return Err(format!("resolvecli: {:?} is not a known agent CLI", cmd.provider_id));
    }

    // Install location: shared across AgentMux versions, keyed
    // by the provider's pinned CLI version, with the legacy
    // per-version dir as a read-only fallback — see
    // `backend::cli_install` (SPEC_AGENT_OPEN_LATENCY_2026_09_27.md
    // F1). The backend registry's pin wins over the caller's.
    let paths = agentmux_common::DataPaths::from_env()
        .ok_or_else(|| "DataPaths::from_env() failed".to_string())?;
    let pinned_version = crate::backend::providers::get_provider(&cmd.provider_id)
        .map(|p| p.pinned_version.to_string())
        .unwrap_or_else(|| cmd.pinned_version.clone());
    let install_dir = crate::backend::cli_install::install_dir(&paths, &cmd.provider_id, &pinned_version);
    let provider_dir = install_dir.to_string_lossy().to_string();
    // npm binary path — the only valid location for installed CLIs.
    let npm_bin = crate::backend::cli_install::npm_bin(&install_dir, &cmd.cli_command)
        .to_string_lossy()
        .to_string();

    let registered = crate::backend::providers::get_provider(&cmd.provider_id);
    if let Some(found) = locate_for_resolve(&cmd, &paths, &pinned_version).await {
        let version = get_cli_version(&found.path).await;
        tracing::info!(path = %found.path, version = %version, source = found.source.as_str(), "CLI found");
        notices.observe_version(&version);
        return Ok(ResolveCliResult {
            cli_path: found.path,
            version,
            source: found.source.as_str().to_string(),
        });
    }
    if let Some(refusal) = not_installed_refusal(&cmd, registered) {
        return Err(refusal);
    }

    // What gets installed is the registry's package for this id,
    // whatever the request names; an id outside the registry is not
    // installed.
    let provider = crate::server::install_handlers::installable_provider(&cmd.provider_id)
        .map_err(|e| format!("resolvecli: {e}"))?;
    if crate::backend::cli_download::is_managed_install(provider.id) {
        return resolve_by_download(&notices, &broker, &paths, &cmd, provider.id, &pinned_version).await;
    }
    tracing::info!(
        provider = %provider.id,
        npm_package = %provider.npm_package,
        pinned_version = %pinned_version,
        target_dir = %provider_dir,
        "CLI not found locally, installing via npm"
    );
    install_via_npm(&cmd, &broker, &notices, &paths, provider, &pinned_version, &npm_bin).await
}

/// Steps 1 and 2 of `resolvecli`: already there? A provider in the registry goes
/// through the one lookup (`locate_provider_cli`): AgentMux's
/// install, else the user's own copy only when the registry says
/// AgentMux doesn't install it. A core tool (git, node, …) isn't
/// in the registry: its install dir, else PATH when the caller
/// sent no package.
async fn locate_for_resolve(
    cmd: &CommandResolveCliData,
    paths: &agentmux_common::DataPaths,
    pinned_version: &str,
) -> Option<LocatedCli> {
    if crate::backend::providers::get_provider(&cmd.provider_id).is_some() {
        return locate_provider_cli(&cmd.provider_id).await;
    }
    match crate::backend::cli_install::find_installed(paths, &cmd.provider_id, pinned_version, &cmd.cli_command) {
        Some(found) => Some(LocatedCli {
            path: found.to_string_lossy().into_owned(),
            source: CliSource::LocalInstall,
        }),
        None if cmd.npm_package.is_empty() => {
            resolve_provider_cli_on_path(&cmd.provider_id, &cmd.cli_command)
                .await
                .map(|path| LocatedCli { path, source: CliSource::SystemPath })
        }
        None => None,
    }
}

/// A CLI that was not found and that `resolvecli` must not install: AgentMux doesn't
/// install it (say how to), or the caller sent no package (a look, not an install request).
fn not_installed_refusal(
    cmd: &CommandResolveCliData,
    registered: Option<&'static crate::backend::providers::ProviderConfig>,
) -> Option<String> {
    let path_only = registered
        .map(|p| p.npm_package.is_empty() && !crate::backend::cli_download::is_managed_install(p.id))
        .unwrap_or_else(|| cmd.npm_package.is_empty());
    if path_only {
        // This branch fires for PATH-only providers
        // (`npm_package` empty) whose CLI isn't on the
        // system PATH. AgentMux can't auto-install these
        // — emit AMX-CLI-004 with a manual install hint
        // instead of AMX-CLI-001 ("Click Install now"),
        // which would point the user at an install
        // affordance that doesn't apply.
        let install_hint = if cfg!(target_os = "windows") {
            cmd.windows_install_command.clone()
        } else {
            cmd.unix_install_command.clone()
        };
        return Some(
            agentmux_common::AgentMuxError::CliMissingOnPath {
                provider: cmd.provider_id.clone(),
                cli: cmd.cli_command.clone(),
                install_hint,
            }
            .to_wire()
            .to_string(),
        );
    }
    // An npm-managed provider that isn't installed, asked about
    // without its package: a look, not an install request.
    if cmd.npm_package.is_empty() {
        return Some(
            agentmux_common::AgentMuxError::CliNotInstalled {
                provider: cmd.provider_id.clone(),
                cli: cmd.cli_command.clone(),
            }
            .to_wire()
            .to_string(),
        );
    }
    None
}

/// Whether `npm` can be found: `where` on Windows, `which` elsewhere.
async fn npm_is_available() -> bool {
    if cfg!(windows) {
        // CREATE_NO_WINDOW (0x08000000) suppresses cmd flash —
        // see broader fix in this file's other spawns.
        let mut probe = tokio::process::Command::new("where");
        probe.arg("npm");
        #[cfg(windows)]
        {
            probe.no_window();
        }
        probe.output().await.map(|o| o.status.success()).unwrap_or(false)
    } else {
        tokio::process::Command::new("which").arg("npm").output().await
            .map(|o| o.status.success()).unwrap_or(false)
    }
}

/// Install the provider's pinned package with npm and report the result.
async fn install_via_npm(
    cmd: &CommandResolveCliData,
    broker: &Arc<crate::backend::mps::Broker>,
    notices: &CliNotices,
    paths: &agentmux_common::DataPaths,
    provider: &'static crate::backend::providers::ProviderConfig,
    pinned_version: &str,
    npm_bin: &str,
) -> Result<ResolveCliResult, String> {
    // Verify npm is available before attempting install.
    if !npm_is_available().await {
        return Err(format!(
            "{} requires Node.js/npm to install. \
            Install Node.js from https://nodejs.org then restart AgentMux.",
            cmd.cli_command
        ));
    }

    let outcome = run_npm_install(cmd, broker, notices, paths, provider, pinned_version).await?;

    use crate::backend::cli_install::{InstallError, InstallOutcome};
    let (bin, source) = match outcome {
        Ok(InstallOutcome::Installed(bin)) => (bin, "installed"),
        Ok(InstallOutcome::AlreadyInstalled(bin)) => (bin, "local_install"),
        Err(InstallError::NpmFailed { code }) => {
            return Err(agentmux_common::AgentMuxError::NpmInstallFailed {
                package: format!("{}@{}", provider.npm_package, pinned_version),
                message: format!(
                    "exit {}; check the output above",
                    code.unwrap_or(-1)
                ),
            }
            .to_wire()
            .to_string());
        }
        Err(InstallError::ShimMissing { .. }) => {
            return Err(agentmux_common::AgentMuxError::CliShimMissing {
                provider: cmd.provider_id.clone(),
                expected_path: npm_bin.to_string(),
            }
            .to_wire()
            .to_string());
        }
        Err(e) => return Err(e.to_string()),
    };
    let bin = bin.to_string_lossy().to_string();
    let version = get_cli_version(&bin).await;
    tracing::info!(path = %bin, version = %version, source, "CLI installed (npm)");
    notices.observe_version(&version);
    Ok(ResolveCliResult {
        cli_path: bin,
        version,
        source: source.to_string(),
    })
}

/// One install routine for this and the startup warm-up
/// (`cli_install::install_pinned_cli`): cross-instance lock
/// held for npm's whole lifetime — inside the blocking task,
/// so a timed-out RPC can't release it early — reuse of an
/// install another instance finished while we waited, and
/// the completion marker only after the shim is verified.
/// The pane says it's installing, then how it went: a pane
/// restored at startup otherwise just starts slower.
async fn run_npm_install(
    cmd: &CommandResolveCliData,
    broker: &Arc<crate::backend::mps::Broker>,
    notices: &CliNotices,
    paths: &agentmux_common::DataPaths,
    provider: &'static crate::backend::providers::ProviderConfig,
    pinned_version: &str,
) -> Result<
    Result<crate::backend::cli_install::InstallOutcome, crate::backend::cli_install::InstallError>,
    String,
> {
    let req = crate::backend::cli_install::NpmInstallRequest {
        provider_id: provider.id.to_string(),
        npm_package: provider.npm_package.to_string(),
        pinned_version: pinned_version.to_string(),
        cli_command: provider.cli_command.to_string(),
        background: false,
    };
    let install_paths = paths.clone();
    let block_id_install = cmd.block_id.clone();
    let install_id = format!("{}-{}-{}", cmd.provider_id, pinned_version, agentmux_common::time::now_ms());
    let install_started = std::time::Instant::now();
    notices.install(&install_id, pinned_version, crate::backend::cli_notice::InstallState::Installing, None, None);
    let broker_npm = broker.clone();
    let outcome = tokio::task::spawn_blocking(move || {
        let on_line = |stream: &'static str, line: &str| {
            tracing::info!(stream, line = %line, "npm output");
            if !block_id_install.is_empty() {
                crate::backend::mps::publish_install_progress(
                    &broker_npm,
                    &block_id_install,
                    line,
                );
            }
        };
        crate::backend::cli_install::install_pinned_cli(
            &install_paths,
            &req,
            &on_line,
        )
    })
    .await
    .map_err(|e| format!("npm install task panicked: {e}"))?;
    match &outcome {
        Ok(_) => notices.install(
            &install_id,
            pinned_version,
            crate::backend::cli_notice::InstallState::Installed,
            Some(install_started.elapsed().as_secs_f64()),
            None,
        ),
        Err(e) => notices.install(
            &install_id,
            pinned_version,
            crate::backend::cli_notice::InstallState::Failed,
            None,
            Some(&e.to_string()),
        ),
    }
    Ok(outcome)
}

/// `checkcliauth`.
///
/// Two-phase auth check for Claude; single-phase for other providers.
///
/// Phase 1 (fast, <1 ms): read the credentials file to determine whether
/// tokens exist at all. If no file / no tokens → return unauthenticated
/// immediately without spawning the CLI. This avoids a 10+ second cold-start
/// stall when the user is definitely not logged in.
///
/// Phase 2 (CLI, 10 s timeout): only when tokens ARE present, run
/// `claude auth status --json` to validate them and obtain the real email.
/// This catches expired/revoked tokens — the false-positive that the old
/// file-only fast path missed.
///
/// Other providers skip Phase 1 and go straight to the CLI (they don't have
/// a predictable credentials file layout).
///
/// See SPEC_AUTH_CHECK_FALSE_POSITIVE_2026_04_15.md.
async fn check_cli_auth(cmd: CommandCheckCliAuthData) -> Result<CheckCliAuthResult, String> {
    tracing::info!(cli = %cmd.cli_path, "CheckCliAuth");

    if cmd.cli_path.to_lowercase().contains("claude") {
        if let Some(signed_out) = claude_credentials_gate(&cmd).await {
            return Ok(signed_out);
        }
    }

    let (authenticated, email, auth_method, raw_output) =
        run_auth_check(&cmd.cli_path, &cmd.auth_check_args, &cmd.auth_env).await?;

    // REMOVED 2026-08-31 (Codex P1 on PR #2878) — the "self-heal a
    // stale isolated Claude credential" step. When validation
    // failed, it copied the user's personal
    // `~/.claude/.credentials.json` into the isolated dir (via
    // `refresh_claude_dir_from_global_if_stale`) whenever the global
    // access token differed, then re-validated ONCE so the agent
    // "recovered" — meaning a channel silently acquired a credential
    // authenticated OUTSIDE it and could then report authenticated
    // with no channel-local login at all.
    //
    // Same source and same end state as the first-run bootstrap
    // removed above, just a different trigger, so removing only that
    // one would have left this per-channel-isolation bypass wide
    // open. See
    // `docs/analysis/ANALYSIS_PER_CHANNEL_AUTH_BYPASSES_2026_08_31.md`
    // (#5).
    //
    // The real failure it was papering over (a global re-login
    // rotating the token and killing the isolated refresh token —
    // the "Pozl 401") is now surfaced honestly: validation fails, the
    // auth card appears, and the user logs in FOR THIS CHANNEL.

    Ok(CheckCliAuthResult {
        authenticated,
        email,
        auth_method,
        raw_output,
        store_problem: None,
    })
}

/// Phase 1 of the Claude auth check: `Some(result)` answers "signed out" without
/// running the CLI; `None` means go on and ask the CLI.
async fn claude_credentials_gate(cmd: &CommandCheckCliAuthData) -> Option<CheckCliAuthResult> {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_default();

    // REMOVED 2026-08-31 — the first-run bootstrap that copied
    // the user's global `~/.claude/.credentials.json` into the
    // isolated CLAUDE_CONFIG_DIR (gated on a
    // `.agentmux-cred-seeded` sentinel).
    //
    // It ran during a routine AUTH CHECK — no login, no user
    // action, no bound account — so an agent in a fresh
    // channel silently acquired the operator's personal
    // credential. That is the most direct of the four
    // per-channel-isolation bypasses catalogued in
    // `docs/analysis/ANALYSIS_PER_CHANNEL_AUTH_BYPASSES_2026_08_31.md`
    // (#4), and the most likely answer to the reported "agents
    // operate with no account" (it also explains why the
    // symptom varied by machine — it depended on whether
    // `~/.claude` happened to hold a valid credential).
    //
    // Its original justification
    // (`docs/retro/retro-provider-auth-isolation-regression-2026-06-05.md`)
    // predates the "agents never use ~/.claude" requirement
    // (`SPEC_BLOCK_AMBIENT_HOME_DIR_IDENTITY_BINDING_2026_08_25.md`)
    // and was never revisited against it.
    //
    // An unauthenticated isolated dir must now simply report
    // `authenticated: false` and let the user log in FOR THIS
    // CHANNEL. Do not reintroduce an import here: a check must
    // report state, never mint credentials as a side effect.

    // §4 INVARIANT (provider-auth-isolation.md): validate the SAME dir
    // the agent runs in — the isolated CLAUDE_CONFIG_DIR if set, else
    // global ~/.claude. NEVER "isolated OR global": that "check global /
    // run isolated" split is the validate-spin regression's root cause
    // (phase 2 below runs `claude auth status --json` against this exact
    // dir, so phase 1 must check the same one).
    let creds_path = cmd
        .auth_env
        .get("CLAUDE_CONFIG_DIR")
        .map(|d| format!("{}/.credentials.json", d))
        .unwrap_or_else(|| format!("{}/.claude/.credentials.json", home));

    let tokens_exist = claude_tokens_exist(&creds_path);

    // Login/logout robustness diagnostic: snapshot the SAME dir
    // we validate (token fingerprint is redacted — see
    // identity::auth_diag). Across login/logout rounds this
    // surfaces the classic bugs — a check reading a different
    // dir than login wrote, a token that didn't change after a
    // re-login, or a credential that outlived a logout. The CLI
    // verdict is logged separately below; compare the two.
    // Cover BOTH the isolated dir (CLAUDE_CONFIG_DIR) and the
    // global-fallback dir (~/.claude) — creds_path/tokens_exist
    // above use exactly this resolution, so the snapshot always
    // reflects the dir actually validated (not only the isolated
    // case). "auth.credstate:" is `muxlog auth` vocabulary.
    let checked_dir = cmd
        .auth_env
        .get("CLAUDE_CONFIG_DIR")
        .cloned()
        .unwrap_or_else(|| format!("{home}/.claude"));
    tracing::info!(
        "auth.credstate: check {}",
        crate::identity::auth_diag::snapshot(&checked_dir)
    );

    if !tokens_exist {
        // On macOS the Claude CLI stores credentials in the
        // Keychain ("Claude Safe Storage"), NOT in
        // .credentials.json — so a missing file does NOT mean
        // logged out. Fall through to `claude auth status`, which
        // reads the Keychain (and does so without a prompt — the
        // CLI owns that Keychain item). On Windows/Linux the file
        // IS the credential store, so the fast "definitely not
        // authenticated" short-circuit stays correct there.
        #[cfg(not(target_os = "macos"))]
        {
            tracing::info!("claude auth check: no credentials in provider dir, skipping CLI");
            return Some(CheckCliAuthResult::signed_out("no credentials found"));
        }
        #[cfg(target_os = "macos")]
        {
            // No login keychain: say so instead of asking the CLI, whose read of it
            // makes macOS open its "Keychain Not Found" dialog on every poll.
            if let Some(result) = super::keychain_probe::no_keychain_result().await {
                return Some(result);
            }
            tracing::info!("claude auth check: no credentials file — checking Keychain via CLI (macOS)");
        }
    } else {
        // Tokens exist in the provider dir — validate with the CLI (10 s timeout).
        tracing::info!("claude auth check: credentials found, validating via CLI");
    }
    None
}

/// Whether the Claude credentials file holds an access or a refresh token.
fn claude_tokens_exist(creds_path: &str) -> bool {
    match std::fs::read_to_string(creds_path) {
        Ok(content) => serde_json::from_str::<serde_json::Value>(&content)
            .ok()
            .map(|json| {
                let oauth = json.get("claudeAiOauth");
                let has_token = oauth
                    .and_then(|o| o.get("accessToken"))
                    .and_then(|v| v.as_str())
                    .map(|s| !s.is_empty())
                    .unwrap_or(false);
                let has_refresh = oauth
                    .and_then(|o| o.get("refreshToken"))
                    .and_then(|v| v.as_str())
                    .map(|s| !s.is_empty())
                    .unwrap_or(false);
                has_token || has_refresh
            })
            .unwrap_or(false),
        Err(_) => false,
    }
}

async fn ensure_provider_auth_dir(req: EnsureProviderAuthDirReq) -> Result<EnsureProviderAuthDirResult, String> {
    let provider = crate::backend::providers::get_provider(&req.provider_id)
        .ok_or_else(|| format!("provider.ensureauthdir: unknown provider {:?}", req.provider_id))?;
    let path = crate::backend::providers::default_auth_dir(provider)?;
    crate::backend::providers::prepare_provider_auth_dir(provider, &path)
        .map_err(|e| format!("provider.ensureauthdir: failed to prepare {path}: {e}"))?;
    Ok(EnsureProviderAuthDirResult { path })
}

async fn run_cli_login(cmd: CommandRunCliLoginData) -> Result<RunCliLoginResult, String> {
    tracing::info!(cli = %cmd.cli_path, args = ?cmd.login_args, "RunCliLogin");

    // Dead path: the active login flow is the CEF host IPC
    // `run_cli_login`, which owns the child's lifecycle (supersede-kill
    // + timeout reaper). This srv-side variant previously spawned a
    // DETACHED, unkillable `auth login` child here — a process leak if
    // ever invoked. It has no live caller; do NOT spawn.
    Ok(RunCliLoginResult { auth_url: None, raw_output: String::new() })
}

async fn toolchain_env() -> Result<ToolchainEnvResult, String> {
    Ok(ToolchainEnvResult {
        path: std::env::var("PATH").unwrap_or_default(),
        path_source: std::env::var("AGENTMUX_PATH_SOURCE")
            .unwrap_or_else(|_| "inherited".to_string()),
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
    })
}

async fn toolchain_prune(
    req: Option<ToolchainPruneReq>,
    mstore: Arc<crate::backend::storage::store::Store>,
) -> Result<ToolchainPruneResult, String> {
    let dry_run = req.as_ref().and_then(|r| r.dry_run).unwrap_or(true);
    let only = req.and_then(|r| r.only);
    let paths = agentmux_common::DataPaths::from_env()
        .ok_or_else(|| "DataPaths::from_env() failed".to_string())?;
    let live = super::uninstall_handlers::live_block_commands(&mstore).map_err(|e| format!("toolchain.prune: {e}"))?;
    let max_idle = crate::backend::cli_prune::DEFAULT_MAX_IDLE;
    let now = std::time::SystemTime::now();
    let run = tokio::task::spawn_blocking(move || {
        crate::backend::cli_prune::run(&paths, &live, dry_run, only.as_deref(), max_idle, now)
    })
    .await
    .map_err(|e| format!("toolchain.prune: {e}"))?;
    Ok(prune_result(run, dry_run, max_idle))
}

async fn widget_health(data: serde_json::Value) -> Result<WidgetHealthResult, String> {
    let port_raw = data.get("port").and_then(|v| v.as_u64()).unwrap_or(0);
    if port_raw == 0 || port_raw > 65535 {
        return Ok(WidgetHealthResult { healthy: false, status_code: None });
    }
    let port = port_raw as u16;
    let path = data
        .get("health_check_path")
        .and_then(|v| v.as_str())
        .unwrap_or("/")
        .to_string();
    let body_contains = data
        .get("health_check_body_contains")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let url = format!("http://127.0.0.1:{}{}", port, path);
    let client = crate::http_client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()
        .map_err(|e| e.to_string())?;
    match client.get(&url).send().await {
        Ok(resp) => {
            let status = resp.status().as_u16();
            let ok_status = resp.status().is_success();
            if !ok_status {
                return Ok(WidgetHealthResult {
                    healthy: false,
                    status_code: Some(status),
                });
            }
            // Optionally verify response body for service identity.
            let healthy = if let Some(needle) = body_contains {
                let body = resp.text().await.unwrap_or_default();
                body.contains(&needle)
            } else {
                true
            };
            Ok(WidgetHealthResult { healthy, status_code: Some(status) })
        }
        Err(_) => Ok(WidgetHealthResult { healthy: false, status_code: None }),
    }
}

fn widget_api_rejection(why: &str) -> WidgetApiResult {
    WidgetApiResult {
        ok: false,
        status_code: None,
        body: None,
        error: Some(why.to_string()),
    }
}

async fn widget_api(data: serde_json::Value) -> Result<WidgetApiResult, String> {
    let reject = widget_api_rejection;
    let port_raw = data.get("port").and_then(|v| v.as_u64()).unwrap_or(0);
    if port_raw == 0 || port_raw > 65535 {
        return Ok(reject("invalid port"));
    }
    let port = port_raw as u16;
    let path = data.get("path").and_then(|v| v.as_str()).unwrap_or("/").to_string();
    // Reject paths that could escape localhost: must start with '/',
    // no '@' (user-info injection: 127.0.0.1@evil.com), no backslash,
    // no protocol-relative '//' prefix.
    if !path.starts_with('/') || path.contains('@') || path.contains('\\') || path.starts_with("//") {
        return Ok(reject("invalid path"));
    }
    let method = data
        .get("method")
        .and_then(|v| v.as_str())
        .unwrap_or("GET")
        .to_uppercase();
    let body_str = data.get("body").and_then(|v| v.as_str()).map(|s| s.to_string());
    let headers_obj = data.get("headers").and_then(|v| v.as_object()).cloned();

    let url = format!("http://127.0.0.1:{}{}", port, path);
    let client = crate::http_client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| e.to_string())?;

    let mut req = match method.as_str() {
        "POST" => client.post(&url),
        "PUT" => client.put(&url),
        "DELETE" => client.delete(&url),
        "PATCH" => client.patch(&url),
        _ => client.get(&url),
    };

    if let Some(headers) = headers_obj {
        for (k, v) in &headers {
            if let Some(vs) = v.as_str() {
                if let (Ok(hn), Ok(hv)) = (
                    reqwest::header::HeaderName::from_bytes(k.as_bytes()),
                    reqwest::header::HeaderValue::from_str(vs),
                ) {
                    req = req.header(hn, hv);
                }
            }
        }
    }

    if let Some(body) = body_str {
        req = req
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body);
    }

    match req.send().await {
        Ok(resp) => {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            Ok(WidgetApiResult {
                ok: true,
                status_code: Some(status),
                body: Some(body),
                error: None,
            })
        }
        Err(e) => Ok(reject(&e.to_string())),
    }
}

async fn toolchain_versions(
    req: ToolchainVersionsReq,
) -> Result<std::collections::HashMap<String, Option<String>>, String> {
    let packages = req.packages;

    let client = crate::http_client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .user_agent("agentmux/toolchain-versions")
        .build()
        .map_err(|e| e.to_string())?;

    let futs: Vec<_> = packages.into_iter().map(|pkg| {
        let c = client.clone();
        async move {
            let url = format!("https://registry.npmjs.org/{}/latest", pkg.package);
            let version = match c.get(&url).send().await {
                Ok(resp) if resp.status().is_success() => {
                    resp.json::<serde_json::Value>().await.ok()
                        .and_then(|v| v.get("version").and_then(|s| s.as_str()).map(|s| s.to_string()))
                }
                _ => None,
            };
            (pkg.id, version)
        }
    }).collect();

    let results = futures_util::future::join_all(futs).await;
    // A map keyed by the caller's own `id`s, so the response type
    // is the map itself rather than a named struct -- there is no
    // fixed set of fields to name.
    let mut out = std::collections::HashMap::new();
    for (id, version) in results {
        out.insert(id, version);
    }
    Ok(out)
}

pub(crate) use crate::backend::pane_env::make_cli_cmd;

/// `pi --list-models` exits 0 either way; it is signed in when it lists
/// models rather than "No models available".
fn pi_lists_models(success: bool, stdout: &str, stderr: &str) -> bool {
    success && !stdout.contains("No models available") && !stderr.contains("No models available")
}

/// A probe (`--version`, the auth check) must not start the CLI's own update: agy 1.5 replaces itself on any
/// run, which held a probe for the length of a 200 MB download and, from a windowless srv, could open a
/// console of its own. Updating stays with the agent runs and the user's own terminal.
fn quiet_probe(cmd: &mut tokio::process::Command, cli_path: &str) {
    if is_agy(cli_path) {
        cmd.env("AGY_CLI_DISABLE_AUTO_UPDATE", "1");
    }
}

fn is_agy(cli_path: &str) -> bool {
    let stem = std::path::Path::new(cli_path).file_stem().and_then(|s| s.to_str());
    stem.is_some_and(|s| s.eq_ignore_ascii_case("agy"))
}

/// Run the provider auth-check CLI against `auth_env` and parse the verdict.
/// Returns `(authenticated, email, auth_method, raw_output)`. Extracted so the
/// stale-credential self-heal can re-run the exact same check after refreshing.
async fn run_auth_check(
    cli_path: &str,
    auth_check_args: &[String],
    auth_env: &std::collections::HashMap<String, String>,
) -> Result<(bool, Option<String>, Option<String>, String), String> {
    // No check command must never read as signed in: a bare CLI with null
    // stdin often just exits 0 (pi-acp does).
    if auth_check_args.is_empty() {
        // A CLI with no status command (Grok Build, Muse Code) is asked through its login file.
        return match crate::identity::resolver::cli_file_signed_in(cli_path, auth_env) {
            Some(signed_in) => Ok((signed_in, None, Some("credential file".to_string()), String::new())),
            None => Err("this CLI has no sign-in check".to_string()),
        };
    }
    // pi-acp has no status command of its own; the pi beside it does:
    // `pi --list-models` prints "No models available" until a provider is
    // set up (SPEC_PI_HARNESS_VIA_PI_ACP_2026_10_06.md).
    let pi = crate::backend::providers::pi_beside_pi_acp(cli_path);
    // pi-acp itself exits 0 on any flag, so without its pi there is nothing
    // that can answer.
    if pi.is_none() && crate::backend::providers::is_pi_acp(cli_path) {
        return Err("pi is not installed beside pi-acp".to_string());
    }
    let program = pi.as_deref().unwrap_or(cli_path);
    let output = tokio::time::timeout(std::time::Duration::from_secs(10), {
        let mut check_cmd = make_cli_cmd(program);
        // An auth check is the provider CLI acting as a plain third-party
        // binary, not as one of our agents — no endpoint, no credential.
        // (ReAgent P0, round 5, on #3326.)
        crate::backend::pane_env::sanitize_external_command(&mut check_cmd);
        check_cmd.args(auth_check_args);
        for (k, v) in auth_env {
            check_cmd.env(k, v);
        }
        quiet_probe(&mut check_cmd, program);
        // Null stdin: prevents the CLI from blocking on interactive first-run
        // prompts (onboarding, theme selection) that only appear on a TTY.
        check_cmd.stdin(std::process::Stdio::null());
        check_cmd.output()
    })
    .await
    .map_err(|_| "auth check timed out (10s)".to_string())?
    .map_err(|e| format!("failed to run auth check: {e}"))?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    let mut email = None;
    let mut auth_method = None;
    let authenticated = if let Ok(json) = serde_json::from_str::<serde_json::Value>(&stdout) {
        // Claude outputs `emailAddress`; other CLIs use `email`. Check both.
        email = json
            .get("emailAddress")
            .or_else(|| json.get("email"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        auth_method = json
            .get("authMethod")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        json.get("loggedIn").and_then(|v| v.as_bool()).unwrap_or(false)
    } else if pi.is_some() {
        pi_lists_models(output.status.success(), &stdout, &stderr)
    } else {
        output.status.success()
    };
    let raw_output = if !stdout.is_empty() { stdout } else { stderr };
    Ok((authenticated, email, auth_method, raw_output))
}

// REMOVED 2026-08-31 (Codex P1 on PR #2878): `claude_access_token`,
// `refresh_claude_dir_from_global_if_stale`, and `refresh_dir_from_global` —
// the machinery behind the auth check's "self-heal from the user's global
// ~/.claude" step, together with its `selfheal_tests` module.
//
// Deleted rather than left unused on purpose: a helper whose entire job is
// copying the operator's personal credential into an isolated dir is exactly
// the thing that gets quietly rewired back in later. There is now nothing to
// call. See `docs/analysis/ANALYSIS_PER_CHANNEL_AUTH_BYPASSES_2026_08_31.md`
// (#5) for the full reasoning.
//
// The real condition it recovered from (the "Pozl 401" — a global re-login
// rotating the access token and invalidating the isolated copy's refresh
// token) now surfaces honestly as a failed validation plus an actionable auth
// card, and is fixed by logging in for THIS channel.

/// Where a CLI found by [`locate_provider_cli`] came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CliSource {
    /// AgentMux's own install, under its shared or legacy CLI dir.
    LocalInstall,
    /// The user's own install: on PATH or in its installer's folder.
    SystemPath,
}

impl CliSource {
    /// The wire value, as `resolvecli` and `install.check` report it.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            CliSource::LocalInstall => "local_install",
            CliSource::SystemPath => "system_path",
        }
    }
}

/// A provider CLI that [`locate_provider_cli`] found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LocatedCli {
    pub path: String,
    pub source: CliSource,
}

/// `resolvecli` for a provider that is a release download (goose), not an npm
/// package: the pinned file, its hash checked, unpacked under the install lock
/// and marked complete (`backend::cli_download`).
async fn resolve_by_download(
    notices: &CliNotices,
    broker: &Arc<crate::backend::mps::Broker>,
    paths: &agentmux_common::DataPaths,
    cmd: &CommandResolveCliData,
    provider_id: &str,
    pinned_version: &str,
) -> Result<ResolveCliResult, String> {
    use crate::backend::cli_install::InstallOutcome;
    use crate::backend::cli_notice::InstallState;
    let install_id = format!("{provider_id}-{pinned_version}-{}", agentmux_common::time::now_ms());
    let started = std::time::Instant::now();
    notices.install(&install_id, pinned_version, InstallState::Installing, None, None);
    let (block_id, broker) = (cmd.block_id.clone(), broker.clone());
    let on_line = move |_: &'static str, line: &str| {
        tracing::info!(line = %line, "download output");
        if !block_id.is_empty() {
            crate::backend::mps::publish_install_progress(&broker, &block_id, line);
        }
    };
    let stop = std::sync::atomic::AtomicBool::new(false);
    let outcome = crate::backend::cli_download::install_managed_cli(paths, provider_id, &on_line, &stop).await;
    match &outcome {
        Ok(_) => notices.install(&install_id, pinned_version, InstallState::Installed, Some(started.elapsed().as_secs_f64()), None),
        Err(e) => notices.install(&install_id, pinned_version, InstallState::Failed, None, Some(&e.to_string())),
    }
    let (bin, source) = match outcome.map_err(|e| e.to_string())? {
        InstallOutcome::Installed(bin) => (bin, "installed"),
        InstallOutcome::AlreadyInstalled(bin) => (bin, "local_install"),
    };
    let bin = bin.to_string_lossy().to_string();
    let version = get_cli_version(&bin).await;
    tracing::info!(path = %bin, version = %version, source, "CLI installed (download)");
    notices.observe_version(&version);
    Ok(ResolveCliResult { cli_path: bin, version, source: source.to_string() })
}

/// The CLI a launch of `provider_id` would run, without installing anything:
/// AgentMux's own install, for every provider. The person's own copy on PATH
/// or in its installer's folder only when they listed the provider in
/// `cli:usepath` ("Use the CLI on my PATH"), and then it comes first. Never
/// otherwise, so a copy on PATH can't make a missing install read as "found".
/// The one lookup behind `install.check`, `resolvecli`, the launch
/// (`agent.open`) and the ambient calls, so the agent picker and the Toolchain
/// pane can't disagree (SPEC_AGENT_PICKER_INLINE_CREATE_2026_10_18.md §1).
/// `None` for an unknown provider.
pub(crate) async fn locate_provider_cli(provider_id: &str) -> Option<LocatedCli> {
    locate_provider_cli_with(provider_id, crate::backend::providers::uses_path_cli(provider_id)).await
}

/// [`locate_provider_cli`]'s path, for the ambient calls: registered once at startup (`ambient::cli_locator`).
pub(crate) fn locate_for_ambient(provider_id: &str) -> std::pin::Pin<Box<dyn std::future::Future<Output = Option<String>> + Send + '_>> {
    Box::pin(async move { locate_provider_cli(provider_id).await.map(|l| l.path) })
}

async fn locate_provider_cli_with(provider_id: &str, use_path: bool) -> Option<LocatedCli> {
    let provider = crate::backend::providers::get_provider(provider_id)?;
    if use_path {
        if let Some(path) = resolve_provider_cli_on_path(provider.id, provider.cli_command).await {
            return Some(LocatedCli { path, source: CliSource::SystemPath });
        }
    }
    let paths = agentmux_common::DataPaths::from_env()?;
    crate::backend::cli_install::find_installed_for_provider(&paths, provider.id)
        .map(|bin| LocatedCli { path: bin.to_string_lossy().into_owned(), source: CliSource::LocalInstall })
}

/// The person's own copy of a CLI: on PATH, else in the folder its own installer
/// uses ([`crate::backend::providers::known_install_paths`]), which may not
/// be on this process's PATH until AgentMux restarts.
///
/// A provider that takes its prompt on the command line
/// (`ProviderConfig::prompt_arg_flag`) must run as the real executable: a
/// `.cmd`/`.bat` wrapper is re-parsed by `cmd.exe`, where `&`, `|` or `%` in
/// a prompt could turn into a command. For those the installer's own
/// executable comes first, and a script wrapper on PATH is never used.
pub(crate) async fn resolve_provider_cli_on_path(provider_id: &str, cli_command: &str) -> Option<String> {
    let known = || {
        crate::backend::providers::known_install_paths(provider_id)
            .into_iter()
            .find(|p| p.is_file())
            .map(|p| p.to_string_lossy().into_owned())
    };
    // A prompt-in-argv CLI, a terminal-tier one (it runs as the pane's own
    // process) and Muse's MSP host (its launcher updates itself and wraps the
    // stdio) can't be a `.cmd` wrapper that isn't an npm shim.
    let provider = crate::backend::providers::get_provider(provider_id);
    let prompt_in_argv = provider.and_then(|p| p.prompt_arg_flag()).is_some();
    let terminal = provider.is_some_and(|p| {
        matches!(p.controller_type, crate::backend::providers::ControllerType::Terminal | crate::backend::providers::ControllerType::Msp)
    });
    if prompt_in_argv || terminal {
        if let Some(path) = known() {
            return Some(path);
        }
        // Every match, not just the first: a `.cmd` shim ahead of the real
        // executable on PATH must not hide it.
        return resolve_cli_candidates_on_path(cli_command)
            .await
            .into_iter()
            .find(|path| !is_script_wrapper(path));
    }
    if let Some(path) = resolve_cli_on_path(cli_command).await {
        return Some(path);
    }
    known()
}

/// Whether `path` is a Windows batch script (`.cmd`/`.bat`), which runs
/// through `cmd.exe` and re-parses its arguments.
fn is_script_wrapper(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".cmd") || lower.ends_with(".bat")
}

/// Resolve a CLI command on the system PATH.
///
/// Uses `where` on Windows and `which` on Unix. Returns the absolute path
/// if the command is found and exists, otherwise `None`.
///
/// On Windows we pass the bare command name (no `.cmd` suffix) so that
/// `where` resolves the correct extension via PATHEXT. This correctly finds
/// `docker.exe`, `git.exe`, `node.exe` AND `npm.cmd` — previously the
/// hard-coded `.cmd` suffix caused all `.exe`-based tools (docker, git,
/// node) to report as not installed even when present on PATH.
///
/// `where` can return multiple lines (e.g. Node.js ships both an extensionless
/// `npm` Unix shell script and `npm.cmd` on Windows). We filter to the first
/// line whose extension `make_cli_cmd` can actually spawn (.exe / .cmd / .bat).
/// Taking the raw first line would yield the extensionless entry, which
/// `Command::new` cannot run on Windows without a shell.
async fn resolve_cli_on_path(cli_command: &str) -> Option<String> {
    resolve_cli_candidates_on_path(cli_command).await.into_iter().next()
}

/// Every spawnable match for `cli_command` on PATH that exists, in PATH order.
async fn resolve_cli_candidates_on_path(cli_command: &str) -> Vec<String> {
    let which_result = if cfg!(windows) {
        let mut probe = tokio::process::Command::new("where");
        probe.arg(cli_command);
        #[cfg(windows)]
        {
            probe.no_window();
        }
        probe.output().await
    } else {
        tokio::process::Command::new("which").arg(cli_command).output().await
    };
    match which_result {
        Ok(out) if out.status.success() => spawnable_candidates(&String::from_utf8_lossy(&out.stdout), cfg!(windows))
            .into_iter()
            .filter(|p| std::path::Path::new(p).exists())
            .collect(),
        _ => Vec::new(),
    }
}

/// The lines of `where`/`which` output a spawn can use. On Windows that is
/// the ones `make_cli_cmd` can run (.exe / .cmd / .bat), skipping e.g. the
/// extensionless `npm` shell script; elsewhere `which` prints one path.
fn spawnable_candidates(stdout: &str, windows: bool) -> Vec<String> {
    let lines = stdout.lines().map(str::trim).filter(|l| !l.is_empty());
    if windows {
        lines
            .filter(|l| {
                let lo = l.to_lowercase();
                lo.ends_with(".exe") || lo.ends_with(".cmd") || lo.ends_with(".bat")
            })
            .map(str::to_string)
            .collect()
    } else {
        lines.take(1).map(str::to_string).collect()
    }
}

/// The CLI notices one `ResolveCli` call can put in its pane
/// (`backend::cli_notice`, SPEC_LAUNCH_CONTEXT_WORKSPACE_RULE_AND_STARTUP_FILES_2026_09_30.md
/// §6.3). A probe with no `block_id` (the picker, the Toolchain pane) puts
/// nothing anywhere.
struct CliNotices {
    broker: std::sync::Arc<crate::backend::mps::Broker>,
    filestore: std::sync::Arc<crate::backend::storage::filestore::FileStore>,
    mstore: std::sync::Arc<crate::backend::storage::store::Store>,
    /// The pane being launched (`notice_block_id`, else `block_id`).
    block_id: String,
    /// The agent being launched, when the caller says; else the pane's
    /// `agentId` meta.
    agent_id: Option<String>,
    provider: String,
}

/// The pane a resolve's notices go to: the launch target when the caller
/// names one (a quick fork resolves from its source pane), else `block_id`.
fn notice_block_id(cmd: &CommandResolveCliData) -> String {
    cmd.notice_block_id
        .as_deref()
        .map(str::trim)
        .filter(|b| !b.is_empty())
        .unwrap_or(&cmd.block_id)
        .to_string()
}

impl CliNotices {
    fn install(
        &self,
        install_id: &str,
        version: &str,
        state: crate::backend::cli_notice::InstallState,
        seconds: Option<f64>,
        error: Option<&str>,
    ) {
        if self.block_id.is_empty() {
            return;
        }
        let frame = crate::backend::cli_notice::install_frame(install_id, &self.provider, version, state, seconds, error);
        crate::backend::cli_notice::append_to_pane(&self.broker, &self.filestore, &self.mstore, &self.block_id, &frame);
    }

    /// Record the version this pane's agent is about to run. Claude reports
    /// the version it actually runs in its own `system/init` frame, which
    /// covers every spawn path (`persistent/stdout_reader.rs`), so it is observed
    /// there and not here.
    fn observe_version(&self, raw_version: &str) {
        if self.block_id.is_empty() || self.provider == "claude" {
            return;
        }
        let Some(version) = crate::backend::cli_notice::version_token(raw_version) else { return };
        let (broker, filestore, mstore) = (self.broker.clone(), self.filestore.clone(), self.mstore.clone());
        let (block_id, provider, agent_id) = (self.block_id.clone(), self.provider.clone(), self.agent_id.clone());
        tokio::task::spawn_blocking(move || {
            crate::backend::cli_notice::observe_and_notify(
                &broker,
                &filestore,
                &mstore,
                &block_id,
                agent_id.as_deref(),
                &provider,
                &version,
            );
        });
    }
}

/// The version `cli_path` reports, asked of the CLI once per file (`cli_version.rs`): it runs on
/// every agent open, and each probe is a node process.
pub(crate) async fn get_cli_version(cli_path: &str) -> String {
    super::cli_version::cached_version(cli_path, || probe_cli_version(cli_path)).await
}

/// Run `<cli> --version`; `None` if it timed out or failed.
async fn probe_cli_version(cli_path: &str) -> Option<String> {
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        {
            let mut c = make_cli_cmd(cli_path);
            // Version probe — same reasoning as the auth check above.
            crate::backend::pane_env::sanitize_external_command(&mut c);
            c.arg("--version").stdin(std::process::Stdio::null());
            quiet_probe(&mut c, cli_path);
            c.output()
        },
    ).await;
    match result {
        Ok(Ok(output)) if output.status.success() => {
            Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
        }
        Ok(_) => None,
        Err(_) => {
            tracing::warn!(cli_path = %cli_path, "get_cli_version timed out after 5s");
            None
        }
    }
}

/// The wire form of a prune [`crate::backend::cli_prune::Run`].
fn prune_result(
    run: crate::backend::cli_prune::Run,
    dry_run: bool,
    max_idle: std::time::Duration,
) -> ToolchainPruneResult {
    use crate::backend::cli_prune::Candidate;
    let item = |c: &Candidate| ToolchainPruneItem {
        dir: c.dir.to_string_lossy().into_owned(),
        provider: c.provider.clone(),
        version: c.version.clone(),
        legacy: c.legacy,
        bytes: c.bytes,
        idle_days: c.idle.as_secs() / 86_400,
    };
    let removed_dirs: std::collections::HashSet<&std::path::PathBuf> = run
        .report
        .iter()
        .flat_map(|r| r.removed.iter().map(|(d, _)| d))
        .collect();
    ToolchainPruneResult {
        dry_run,
        scan_ok: run.scan_ok,
        candidates: run.plan.prunable.iter().map(item).collect(),
        reclaimable_bytes: run.plan.reclaimable_bytes(),
        removed: run
            .plan
            .prunable
            .iter()
            .filter(|c| removed_dirs.contains(&c.dir))
            .map(item)
            .collect(),
        skipped: run
            .report
            .iter()
            .flat_map(|r| r.skipped.iter())
            .map(|(d, why)| ToolchainPruneSkip { dir: d.to_string_lossy().into_owned(), reason: why.clone() })
            .collect(),
        kept: run.plan.kept.len() as u32,
        max_idle_days: (max_idle.as_secs() / 86_400) as u32,
    }
}

#[cfg(test)]
mod tests;
