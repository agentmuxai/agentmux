// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0


use std::sync::Arc;


use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::rpc_types::{
    COMMAND_SUBPROCESS_SPAWN, COMMAND_AGENT_INPUT, COMMAND_AGENT_STOP,
    CommandSubprocessSpawnData, CommandAgentInputData, CommandAgentStopData,
};
use crate::backend::obj::Block;
use crate::backend::blockcontroller;

use super::super::AppState;

/// Heal a container agent's argv IN PLACE when — and only when — it still
/// carries the persistent controller's flags.
///
/// A container agent runs one `docker exec` per turn: `container_spawn.rs`
/// writes the raw message, closes stdin, and reads until EOF. There is no
/// long-lived stdin and no control channel. The PERSISTENT argv is wrong for
/// that in three separate ways, and it is what stale blocks carry:
///
///   * `--input-format stream-json` — makes the CLI parse every stdin line as
///     a JSON envelope, so it meets the startup markdown and dies with
///     `JSON Parse error: Unrecognized token '#'`. This is the crash that kept
///     container agents from EVER starting (verified live, 2026-08-31).
///   * missing `-p` — the one-shot print flag. Without it the CLI is not in
///     the mode this path drives at all.
///   * `--permission-prompt-tool stdio` (+ a non-bypass `--permission-mode`) —
///     routes tool permissions through the control protocol, which only the
///     persistent controller speaks. A container turn has nothing to answer
///     `can_use_tool`, so it rejects or hangs on the first permission check.
///
/// An earlier cut of this deleted only `--input-format` and left the other two
/// (codex P1 on PR #2867) — that unblocks the immediate crash while leaving the
/// pane in the wrong CLI mode, which is a worse failure because it looks like
/// it works.
///
/// The cut after THAT rebuilt the argv wholesale from `launch_args`, which was
/// worse again (reagent P1 on the same PR): this runs on EVERY container turn,
/// not once per stale block, so a rebuild also threw away everything
/// `agent-model.ts` legitimately appends after the base args —
/// `agent.provider_flags` (user-configurable) and `--fork-session` (gated on
/// `providerId === "claude"`, not on `agentMode`, so container agents reach it).
/// It would have permanently broken both for container agents, on blocks whose
/// argv this fix had already assembled correctly.
///
/// So: subtract, don't rebuild. Remove the flags the PERSISTENT argv carries
/// that the one-shot argv does not, restore any one-shot flag that's missing,
/// and leave every other token — the user's `--model`/`--effort`, their
/// `provider_flags`, `--fork-session` — exactly where it was. Which flags are
/// "persistent-only" is derived from the provider catalog rather than hardcoded,
/// so a future flag moving between the two lists doesn't silently strand this.
///
/// An argv carrying NO persistent-only flag is returned untouched: it was built
/// by the fixed `selectLaunchArgs` path and there is nothing to heal.
///
/// The root cause is fixed at the source in `launch-args.ts`; this is the
/// self-heal for blocks ALREADY persisted with the bad argv, since `cmd:args`
/// lives in block meta and would otherwise stay wrong forever. Applied at the
/// point of use so a block that never re-runs `resync_controller` can't bypass
/// it.
fn container_argv(argv: Vec<String>, provider_id: &str) -> Vec<String> {
    let Some(provider) = crate::backend::providers::get_provider(provider_id) else {
        // Unknown provider — no catalog to diff against, so remove the one flag
        // that is outright fatal rather than guessing at the rest.
        return strip_flag_with_value(argv, "--input-format");
    };
    let Some(persistent) = provider.persistent_launch_args else {
        // Subprocess-shaped provider: its only argv IS the one-shot argv, so a
        // container block could never have been given persistent flags.
        return argv;
    };

    let one_shot = flags_with_arity(provider.launch_args);
    let persistent_only: Vec<(&str, bool)> = flags_with_arity(persistent)
        .into_iter()
        .filter(|(flag, _)| !one_shot.iter().any(|(f, _)| f == flag))
        .collect();

    // Already one-shot-shaped — the frontend assembled this argv, hands off.
    if !argv
        .iter()
        .any(|a| persistent_only.iter().any(|(f, _)| f == a))
    {
        return argv;
    }

    let mut out = argv;
    for (flag, takes_value) in &persistent_only {
        out = if *takes_value {
            strip_flag_with_value(out, flag)
        } else {
            out.into_iter().filter(|a| a != flag).collect()
        };
    }

    // Restore the one-shot flags the persistent argv never had (`-p` above all).
    // Prepended so the provider's own baseline keeps its leading position.
    let mut restored: Vec<String> = Vec::new();
    for (flag, takes_value) in &one_shot {
        if out.iter().any(|a| a == flag) {
            continue;
        }
        restored.push(flag.to_string());
        if *takes_value {
            if let Some(pos) = provider.launch_args.iter().position(|a| a == flag) {
                if let Some(v) = provider.launch_args.get(pos + 1) {
                    restored.push(v.to_string());
                }
            }
        }
    }
    restored.extend(out);
    restored
}

/// Split an args list into `(flag, takes_a_value)` pairs.
///
/// Arity is read off the list itself — a flag whose next token is not another
/// flag takes a value. That's what lets `container_argv` strip
/// `--permission-prompt-tool stdio` (two tokens) and `--verbose` (one)
/// correctly without a hardcoded table of every provider's flag shapes.
fn flags_with_arity(args: &'static [&'static str]) -> Vec<(&'static str, bool)> {
    let mut out = Vec::new();
    for (i, tok) in args.iter().enumerate() {
        if !tok.starts_with('-') {
            continue;
        }
        let takes_value = args
            .get(i + 1)
            .is_some_and(|next| !next.starts_with('-'));
        out.push((*tok, takes_value));
    }
    out
}

/// Remove `flag` and the value token following it, everywhere it appears.
/// Tolerates a trailing `flag` with no value rather than panicking.
fn strip_flag_with_value(args: Vec<String>, flag: &str) -> Vec<String> {
    let mut out = Vec::with_capacity(args.len());
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        if a == flag {
            let _ = it.next();
            continue;
        }
        out.push(a);
    }
    out
}

/// Per-agent git commit identity env vars for a spawned agent process
/// (2026-08-22, docs/retro/retro-shared-git-identity-committer-misattribution-2026-08-22.md).
///
/// Without this, `git commit` falls through to whatever user.name/
/// user.email happens to be sitting in a shared multi-agent host's
/// `~/.gitconfig` -- one agent's real identity, silently baked into every
/// OTHER agent's commits too. `agentmux-cloud`'s review-notification
/// consumer resolves "who committed this" via GitHub's own commit->account
/// auto-linking (`commit.author.login`), not the raw git commit metadata
/// directly -- so whichever agent's *real, GitHub-verified* email is
/// sitting in the shared config silently receives every other agent's
/// committer notifications too (confirmed live: every commit across 9+
/// PRs from 4 different agents on one host resolved to a single agent's
/// account this way).
///
/// `GIT_AUTHOR_*`/`GIT_COMMITTER_*` env vars take precedence over
/// config-file `user.name`/`user.email` at commit time (git's own
/// behavior) -- scoped to just the spawned process tree, same pattern as
/// `AGENTMUX_AGENT_ID`. The email intentionally uses a non-existent
/// `.local` domain: GitHub can't link it to any real account, so an agent
/// with no dedicated PAT registered (the common case -- see this repo's
/// `CLAUDE.md`, "Which GitHub account am I acting as?") simply drops out
/// of `commit.author.login` lookups entirely instead of resolving to
/// someone else's real, verified identity.
fn git_identity_env_vars(agent_id: &str) -> [(&'static str, String); 4] {
    let git_email = format!("{}@agentmux.local", agent_id.to_lowercase());
    [
        ("GIT_AUTHOR_NAME", agent_id.to_string()),
        ("GIT_COMMITTER_NAME", agent_id.to_string()),
        ("GIT_AUTHOR_EMAIL", git_email.clone()),
        ("GIT_COMMITTER_EMAIL", git_email),
    ]
}


/// The slice of [`AppState`] an agent turn needs in order to be started.
///
/// Exists so [`run_agent_turn`] can be driven from somewhere other than the
/// `agentinput` RPC handler it was originally inlined in. The second caller is
/// the reactive handler's message sender (`bootstrap::install_agent_turn_delivery`):
/// a `SubprocessController` starts a turn ONLY via this path, so without a
/// non-RPC entry point every inter-agent message addressed to one was dropped.
/// See `docs/reports/REPORT_JEKT_DELIVERY_DROPS_SUBPROCESS_AGENTS_2026_09_02.md`.
///
/// Field names match the local bindings the extracted body already used, so the
/// body itself is unchanged by the extraction.
#[derive(Clone)]
pub struct AgentTurnDeps {
    pub wstore: Arc<crate::backend::storage::store::Store>,
    pub id_store: Arc<crate::backend::storage::store::Store>,
    pub identity_store: Arc<crate::backend::storage::store::Store>,
    /// Streaming-bash wrapper auth key — see SPEC_STREAMING_BASH_RUNNER_2026_05_11.md §7.
    pub auth_key: String,
    pub broker: Arc<crate::backend::wps::Broker>,
    pub container_manager: Arc<crate::backend::container::ContainerRuntimeHandle>,
    /// Named `filestore_gate` because the spawn-gate error frame MUST be
    /// persisted through it, not merely live-broadcast (reagent P1, PR #2164).
    pub filestore_gate: Arc<crate::backend::storage::filestore::FileStore>,
    pub local_web_url: String,
    pub event_bus_gate: Arc<crate::backend::eventbus::EventBus>,
}

impl AgentTurnDeps {
    pub fn from_state(state: &AppState) -> Self {
        Self {
            wstore: state.wstore.clone(),
            id_store: state.id_store.clone(),
            identity_store: state.identity_store.clone(),
            auth_key: state.auth_key.clone(),
            broker: state.broker.clone(),
            container_manager: state.container_manager.clone(),
            filestore_gate: state.filestore.clone(),
            local_web_url: state.local_web_url.clone(),
            event_bus_gate: state.event_bus.clone(),
        }
    }
}

/// Start one agent turn for `block_id` with `message`.
///
/// This is the whole of what `AgentInputCommand` does, verbatim: re-read the
/// spawn config from block meta, inject identity/muxbus/bashwrap env (failing
/// closed on the oauth spawn gate), then dispatch to the persistent controller,
/// the host subprocess path, or the container `docker exec` path as the block's
/// `agentMode` and controller type require — and finally re-register the agent
/// for reactive delivery.
///
/// Extracted from the `agentinput` handler body with no behavior change so a
/// second, non-RPC caller can reach it; see [`AgentTurnDeps`].
/// Whether starting a turn should also (re-)register the agent for reactive
/// delivery.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnRegistration {
    /// The RPC / UI path. Registers, exactly as `AgentInputCommand` always has.
    Register,
    /// The reactive-delivery path (`bootstrap::install_agent_turn_delivery`).
    /// Skipping is required, for two independent reasons:
    ///
    /// 1. **Redundant.** That caller resolved this block *by looking the agent
    ///    up in the reactive handler's own `agent_to_block` map*, so the agent
    ///    is registered by construction. There is nothing to re-register.
    /// 2. **Deadlock.** `ReactiveHandler::inject_message` holds the global
    ///    `Mutex<Handler>` across the message-sender call, and that sender
    ///    drives this function to completion on the same thread. The
    ///    registration below calls `get_global_handler().register_agent(...)`,
    ///    which locks that same `std::sync::Mutex` — which is NOT reentrant.
    ///    Registering here would block the thread on a lock it already holds,
    ///    wedging the reactive handler process-wide, on essentially every
    ///    successful delivery to a subprocess agent (reagent P0 on PR #2930).
    Skip,
}

pub async fn run_agent_turn(
    deps: &AgentTurnDeps,
    block_id: String,
    message: String,
    message_id: Option<String>,
    registration: TurnRegistration,
) -> Result<(), String> {
    let AgentTurnDeps {
        wstore,
        id_store,
        identity_store,
        auth_key,
        broker,
        container_manager,
        filestore_gate,
        local_web_url,
        event_bus_gate,
    } = deps.clone();

    let ctrl = blockcontroller::get_controller(&block_id)
        .ok_or_else(|| format!("no controller for block {}", block_id))?;

    // Re-read the spawn config from block metadata
    let block: Block = wstore
        .get(&block_id)
        .map_err(|e| format!("agentinput: load block: {e}"))?
        .ok_or_else(|| format!("block {} not found", block_id))?;

    let cli_command = crate::backend::obj::meta_get_string(
        &block.meta, "cmd", "claude",
    );
    let cli_args: Vec<String> = match block.meta.get("cmd:args") {
        Some(serde_json::Value::Array(arr)) => arr
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect(),
        _ => vec![
            "-p".to_string(),
            "--input-format".to_string(),
            "stream-json".to_string(),
            "--output-format".to_string(),
            "stream-json".to_string(),
        ],
    };
    let working_dir = crate::backend::obj::meta_get_string(
        &block.meta, "cmd:cwd", "",
    );
    let mut env_vars: std::collections::HashMap<String, String> = match block.meta.get("cmd:env") {
        Some(serde_json::Value::Object(obj)) => obj
            .iter()
            .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
            .collect(),
        _ => std::collections::HashMap::new(),
    };
    // Identity injection: look up the active AgentInstance for
    // this block, resolve its identity_id's bindings, and merge
    // each per-provider env var into the spawn map. Api-key-class
    // failures are logged and skipped — the agent CLI launches
    // with whatever resolved cleanly plus the static cmd:env
    // block. Oauth-class resolution failures are BLOCKING unless
    // the agent opted into ambient login (layer-3 spawn gate,
    // SPEC_ACCOUNT_DELETE_DEAUTH_LAYERS_2_4_2026_07_14.md §2.2):
    // surface the error in the agent pane (same
    // `error_during_execution` frame the container spawn-failure
    // path uses below) and abort before the CLI is created.
    // See agentmux-srv/src/identity/resolver.rs. Broker
    // hand-in lets the OAuth expiry probe (PR D, spec §4.4)
    // publish `identitybundlebindings:changed:<bundle_id>`
    // when it flips a token's status valid→expired etc.
    env_vars = match crate::identity::resolver::inject_identity_env_async(
        wstore.clone(),
        id_store.clone(),
        identity_store.clone(),
        Some(broker.clone()),
        block_id.clone(),
        env_vars,
    )
    .await
    {
        Ok(env) => env,
        Err(gate) => {
            let error_frame = serde_json::json!({
                "type": "result",
                "is_error": true,
                "subtype": "error_during_execution",
                "error": {"message": format!("[AgentMux] {gate}")}
            })
            .to_string();
            // Some(&filestore_gate): the frame must be PERSISTED
            // to the block file, not just live-broadcast —
            // otherwise the error vanishes on pane
            // reload/reconnect (reagent P1, PR #2164 round 2).
            crate::backend::blockcontroller::shell::handle_append_block_file(
                &broker,
                &block_id,
                crate::backend::blockcontroller::subprocess::SUBPROCESS_OUTPUT_SUBJECT,
                format!("{error_frame}\n").as_bytes(),
                Some(&filestore_gate),
                None,
            );
            // codex P1, PR #2802: the frame above only lands in
            // the block's raw output log, which the recovery
            // banner does NOT read from — it reads the
            // structured `agent:last_failure` block-meta key
            // (persist_last_failure) + the ephemeral
            // EVENT_AGENT_FAILURE push, same as every POST-spawn
            // exit classification (host_spawn.rs). No process
            // ever ran here, so classify() gets no exit
            // code/stderr/result-frame — just the gate's own
            // Display text, exactly like health.rs's in-band-error
            // reclassification call.
            let gate_failure = crate::agents::failure::classify(
                None,
                None,
                &gate.to_string(),
                None,
            );
            crate::backend::blockcontroller::core::persist_last_failure(
                &block_id,
                Some(&gate_failure),
                &Some(wstore.clone()),
                &Some(event_bus_gate.clone()),
            );
            broker.publish(crate::backend::wps::WaveEvent {
                event: crate::backend::wps::EVENT_AGENT_FAILURE.to_string(),
                scopes: vec![format!("block:{}", block_id)],
                sender: String::new(),
                persist: 1,
                data: serde_json::to_value(&gate_failure).ok(),
            });
            return Err(format!("identity spawn gate: {gate}"));
        }
    };
    // MuxBus cloud token — injects MUXBUS_TOKEN + MUXBUS_COGNITO_DOMAIN
    // if the user has authenticated via muxbus.login. No-op if no
    // credentials are stored. Auto-refreshes if token is nearly expired.
    crate::server::muxbus_handlers::inject_muxbus_env(&id_store, &mut env_vars).await;
    // Streaming-bash wrapper auth + discovery
    // (SPEC_STREAMING_BASH_RUNNER_2026_05_11.md §7).
    //
    // 1. AGENTMUX_AUTH_KEY — config.rs:42 removed it from
    //    the process env at startup (security PR #801).
    //    Re-inject for this spawn so the wrapper (running
    //    inside Claude's bash subprocess tree) can
    //    authenticate against the auth_middleware-gated
    //    /agentmux/wps/publish endpoint via X-AuthKey.
    // 2. PATH — prepend the bundled tools/bin dir so
    //    `agentmux-bashwrap.exe` resolves when the
    //    PreToolUse hook (auto-injected by agent_config.rs)
    //    rewrites the command to invoke it. AGENTMUX_LOCAL_URL
    //    is already in the inherited process env (main.rs:498).
    env_vars.insert("AGENTMUX_AUTH_KEY".to_string(), auth_key.clone());
    // Block id so the wrapper can scope its WPS publishes
    // to `block:<id>`. Without this, chunks publish without
    // a scope and the frontend's per-block subscription
    // doesn't receive them.
    env_vars.insert("AGENTMUX_BLOCKID".to_string(), block_id.clone());
    // Agent display name for MuxBus self-identification.
    // AGENTMUX_AGENT_ID is the canonical, app-wide agent-identity
    // variable (used far beyond muxbus -- MCP tool routing, native
    // memory, shell OSC titling, jekt auto-registration, etc; see
    // this repo's CLAUDE.md Naming Conventions table). MUXBUS_AGENT_ID
    // is set below too, mirroring the same value, purely so
    // muxbus-client picks it up under the MUXBUS_* prefix it already
    // checks first (alongside MUXBUS_TOKEN/MUXBUS_COGNITO_DOMAIN
    // injected above) -- this does NOT make MUXBUS_AGENT_ID a second
    // source of truth for agent identity app-wide, it's scoped to
    // this one muxbus hand-off point (ARCH-002, 2026-07-28
    // architecture analyst report).
    // Only set if not already present in cmd:env — user-provided values take precedence.
    if !env_vars.contains_key("AGENTMUX_AGENT_ID") {
        let agent_display_name = crate::backend::obj::meta_get_string(
            &block.meta, "agentName", "",
        );
        if !agent_display_name.is_empty() {
            env_vars.insert("AGENTMUX_AGENT_ID".to_string(), agent_display_name);
        }
    }
    if !env_vars.contains_key("MUXBUS_AGENT_ID") {
        if let Some(agent_id) = env_vars.get("AGENTMUX_AGENT_ID").cloned() {
            env_vars.insert("MUXBUS_AGENT_ID".to_string(), agent_id);
        }
    }
    // Per-agent git commit identity -- see git_identity_env_vars()
    // doc comment. Still overridable per the same "user-provided
    // values take precedence" rule as every other var here.
    if let Some(agent_id) = env_vars.get("AGENTMUX_AGENT_ID").cloned() {
        for (key, value) in git_identity_env_vars(&agent_id) {
            if !env_vars.contains_key(key) {
                env_vars.insert(key.to_string(), value);
            }
        }
    }
    // PATH includes BOTH bundled tools dir (portable
    // builds, runtime/tools/bin/) AND user tools dir
    // (~/.agentmux/tools/bin/). bundled is None in dev
    // mode (target/debug exclusion in tool_store), so
    // without user_tools_dir the wrapper wouldn't be on
    // the agent's PATH during `task dev`.
    {
        let existing = env_vars
            .get("PATH")
            .cloned()
            .or_else(|| std::env::var("PATH").ok())
            .unwrap_or_default();
        let sep = if cfg!(windows) { ";" } else { ":" };
        let mut extras: Vec<String> = Vec::new();
        if let Some(d) = crate::backend::tool_store::bundled_tools_dir() {
            if d.exists() {
                extras.push(d.to_string_lossy().into_owned());
            }
        }
        if let Some(d) = crate::backend::tool_store::user_tools_dir() {
            if d.exists() {
                extras.push(d.to_string_lossy().into_owned());
            }
        }
        if !extras.is_empty() {
            let new_path = format!("{}{}{}", extras.join(sep), sep, existing);
            env_vars.insert("PATH".to_string(), new_path);
        }
    }

    let session_id_field = crate::backend::obj::meta_get_string(
        &block.meta, "agent:session_id_field", "session_id",
    );

    // Try persistent controller first, fall back to subprocess
    if let Some(persistent_ctrl) = ctrl
        .as_any()
        .downcast_ref::<blockcontroller::persistent::PersistentSubprocessController>()
    {
        // Container agents use per-turn docker exec — incompatible with a
        // long-lived persistent subprocess. Fail loudly instead of silently
        // spawning the CLI on the host.
        let agent_mode = crate::backend::obj::meta_get_string(&block.meta, "agentMode", "host");
        if agent_mode == "container" {
            return Err("container agents require a subprocess controller; this provider uses a persistent controller".to_string());
        }
        // Resume support: a /model (or effort/permission) change
        // respawns the persistent CLI with new flags; pass the
        // resume flag + captured session id so the respawn continues
        // the same conversation. Same meta keys the subprocess path
        // reads below. Without this, switching model on a persistent
        // agent would either no-op (old behavior) or lose context.
        let resume_flag = crate::backend::obj::meta_get_string(
            &block.meta, "agent:resume_flag", "--resume",
        );
        let persisted_session_id = crate::backend::obj::meta_get_string(
            &block.meta, "agent:sessionid", "",
        );
        let config = blockcontroller::persistent::PersistentSpawnConfig {
            cli_command,
            cli_args,
            working_dir,
            env_vars,
            session_id_field,
            resume_flag,
            session_id: persisted_session_id,
            message_id: message_id.clone(),
        };
        persistent_ctrl.send_message(message, config)?;
    } else if let Some(subprocess_ctrl) = ctrl
        .as_any()
        .downcast_ref::<blockcontroller::subprocess::SubprocessController>()
    {
        let resume_flag = crate::backend::obj::meta_get_string(
            &block.meta, "agent:resume_flag", "--resume",
        );
        let resume_strategy = crate::backend::obj::meta_get_string(
            &block.meta,
            "agent:resume_strategy",
            if session_id_field == "thread_id" {
                "codex-exec"
            } else if resume_flag.is_empty() {
                "none"
            } else {
                "flag"
            },
        );
        // Picker reattach: the frontend writes the prior
        // block's session id here when launching with
        // `continueOfInstanceId`. spawn_turn hydrates its
        // inner.session_id from this on the first turn so
        // --resume <sid> lands on the very first launch.
        let persisted_session_id = crate::backend::obj::meta_get_string(
            &block.meta, "agent:sessionid", "",
        );

        // Container agent branch: use Docker socket API exec (P1a: no
        // secrets in argv). Host agent branch: regular CLI subprocess.
        let agent_mode = crate::backend::obj::meta_get_string(
            &block.meta, "agentMode", "host",
        );
        // Cross-process session-lease key (registry::LeaseStore) —
        // read once here for both branches below. Only the host
        // branch's spawn_turn actually enforces it in this PR;
        // the container branch's config field is unused for now
        // (struct-completeness — see host_spawn.rs's doc comment).
        let instance_id = crate::backend::obj::meta_get_string(
            &block.meta, "agentId", "",
        );
        if agent_mode == "container" {
            let cm = container_manager.get().await
                .ok_or_else(|| "Docker not available on this host; cannot start container agent".to_string())?;
            let container_image = {
                let img = crate::backend::obj::meta_get_string(&block.meta, "agent:container_image", "");
                if img.is_empty() { "ghcr.io/agentmuxai/agent-claude:latest".to_string() } else { img }
            };
            // Use agentId (UUID) — always valid as a Docker name; display names can have spaces.
            let agent_id = crate::backend::obj::meta_get_string(
                &block.meta, "agentId", "",
            );
            let container_name = crate::backend::container::container_name_for_slug(&agent_id);
            let volumes_json = crate::backend::obj::meta_get_string(
                &block.meta, "agent:container_volumes", "[]",
            );
            let volumes: Vec<String> = serde_json::from_str(&volumes_json).unwrap_or_default();

            // Mount the bound account's credentials and the agent's working
            // directory. `CLAUDE_CONFIG_DIR` was resolved into `env_vars` by the
            // identity injection above, but it is a HOST path and so is stripped
            // by CONTAINER_ENV_DENYLIST before exec — mounting it is the only way
            // the in-container CLI ever sees `.credentials.json`. Without this a
            // container agent authenticates never, no matter how many times the
            // operator logs in via Armory.
            let mount_spec = crate::backend::container::ContainerMountSpec {
                claude_config_host_dir: env_vars
                .get("CLAUDE_CONFIG_DIR")
                .and_then(|d| crate::backend::container::credentials_dir_if_file_backed(d)),
                workspace_host_dir: Some(working_dir.clone()).filter(|d| !d.is_empty()),
            };

            // Ensure container is alive (pull image if needed — P1b).
            if let Err(e) = cm.ensure_running(&container_name, &container_image, &volumes, &[], &mount_spec).await {
                // Surface the error in the agent pane before returning, so the user
                // sees why the container failed (image not found, Docker down, etc.).
                let error_frame = serde_json::json!({
                    "type": "result",
                    "is_error": true,
                    "subtype": "error_during_execution",
                    "error": {"message": format!("[AgentMux] container ensure_running failed: {e}")}
                }).to_string();
                // Some(&filestore_gate): PERSIST, not just live-broadcast —
                // same requirement as the identity spawn-gate frame above
                // (reagent P1, PR #2164 round 2). Previously passed None
                // here despite the comment above claiming parity with that
                // path — codex P1 on PR #2390: muxspect's last_error_frame
                // (which reads only the persisted `output` file) could
                // never see this failure after the live moment passed.
                crate::backend::blockcontroller::shell::handle_append_block_file(
                    &broker,
                    &block_id,
                    crate::backend::blockcontroller::subprocess::SUBPROCESS_OUTPUT_SUBJECT,
                    format!("{error_frame}\n").as_bytes(),
                    Some(&filestore_gate),
                    None,
                );
                return Err(format!("container ensure_running failed: {e}"));
            }

            tracing::info!(
                block_id = %block_id,
                container = %container_name,
                image = %container_image,
                "container agent turn: bollard exec (env via Docker socket, not argv)",
            );

            // Env is passed via CreateExecOptions.env (Docker socket API),
            // NOT as -e KEY=VALUE argv args — this prevents CWE-214 exposure.
            // spawn_container_turn filters config.env_vars (denylist) per
            // turn, so cmd:cwd (host path) and host-path vars never reach
            // the container, and each queued turn uses its own env.

            // Base cmd: [container_command, ...cli_args]. The command
            // is the provider CLI resolved INSIDE the image (on PATH,
            // e.g. `claude`) — NOT `cli_command`/`cmd`, which is the
            // host-resolved absolute npm path and does not exist in the
            // container (docker exec would fail "no such file or
            // directory"). cli_args are format flags (-p, --input-format
            // …) + provider flags — no host paths, safe as-is.
            // spawn_container_turn appends --resume <sid> internally.
            let container_command = crate::backend::obj::meta_get_string(
                &block.meta, "agent:container_command", "claude",
            );
            // Provider id for the one-shot argv rebuild below.
            let agent_provider = crate::backend::obj::meta_get_string(
                &block.meta, "agentProvider", "claude",
            );
            let mut base_cmd = vec![container_command];
            base_cmd.extend(container_argv(cli_args, &agent_provider));

            let config = blockcontroller::subprocess::SubprocessSpawnConfig {
                cli_command: String::new(), // unused by spawn_container_turn
                cli_args: vec![],           // unused by spawn_container_turn
                working_dir: String::new(), // unused — container has own cwd
                env_vars,
                message: message,
                resume_flag,
                resume_strategy,
                session_id_field,
                message_id: message_id,
                session_id: if persisted_session_id.is_empty() {
                    None
                } else {
                    Some(persisted_session_id)
                },
                instance_id: instance_id.clone(),
            };
            subprocess_ctrl.spawn_container_turn(cm.clone(), container_name, base_cmd, config)?;
        } else {
            // Host agent: regular CLI subprocess (env set on child process, not in argv).
            let config = blockcontroller::subprocess::SubprocessSpawnConfig {
                cli_command,
                cli_args,
                working_dir,
                env_vars,
                message: message,
                resume_flag,
                resume_strategy,
                session_id_field,
                message_id: message_id,
                session_id: if persisted_session_id.is_empty() {
                    None
                } else {
                    Some(persisted_session_id)
                },
                instance_id,
            };
            subprocess_ctrl.spawn_turn(config)?;
        }
    } else {
        return Err("controller is not a SubprocessController or PersistentSubprocessController".to_string());
    }

    // Registration is skipped on the reactive-delivery path — see
    // `TurnRegistration::Skip` for why that is both redundant AND a hard
    // deadlock requirement, not an optimisation.
    if matches!(registration, TurnRegistration::Register) {
        // Register with cloud subscriber + reactive handler so cloud-injected
        // messages (e.g. GitHub PR review notifications) reach this agent.
        // Uses agentName (the logical display name, e.g. "smike") as the key —
        // matching the namespace used by reactive.rs:233 (`req.agent_id`) and the
        // delivery path (`agent_to_block` keyed by lowercased logical agent_id).
        // PR bodies embed $AGENTMUX_AGENT_ID (same value) so the cloud injection
        // key and the poll key are always consistent.
        // Both calls are idempotent: add_agent skips the WS send if already
        // subscribed; register_agent replaces any stale mapping from a prior session.
        let agent_name = crate::backend::obj::meta_get_string(
            &block.meta, "agentName", "",
        );
        if !agent_name.is_empty() {
            let registered = crate::backend::reactive::handler::get_global_handler()
                .register_agent(&agent_name, &block_id, None);
            if registered.is_ok() {
                // Refresh the block's OWN captured identity too
                // (reagentx P1, round 2 on #2697): this call runs on
                // EVERY turn, using block.meta["agentName"] as the
                // source of truth — which can diverge from whatever
                // a PersistentSubprocessController captured at its
                // original spawn_process call (rename, reconfigured
                // cmd:env) without the block ever respawning. Without
                // this, inject_message_inner's recipient-identity
                // check (#2695) would compare the current (correct)
                // target_agent against that stale spawn-time value
                // and falsely reject the agent's own, correctly-
                // addressed jekts as an identity mismatch. A no-op
                // for controller types that don't override
                // set_agent_id (e.g. SubprocessController).
                if let Some(ctrl) = crate::backend::blockcontroller::get_controller(&block_id) {
                    ctrl.set_agent_id(Some(agent_name.clone()));
                }
                if let Some(sub) = crate::muxbus::cloud_subscriber::get_global_subscriber() {
                    sub.add_agent(&agent_name);
                }
                // Also mirror into the per-channel + host-global
                // shared file registries — this AgentInput/
                // SubprocessController path (unlike ShellController/
                // PersistentSubprocessController) previously only
                // ever registered in the in-memory Tier-1 map,
                // leaving it permanently unreachable via Tier 2/2b
                // cross-instance/cross-channel delivery (reagent P1,
                // third round on PR #2350).
                let data_dir = crate::backend::base::get_wave_data_dir();
                crate::backend::reactive::registry::write(
                    &data_dir,
                    &agent_name,
                    &local_web_url,
                    &block_id,
                );
                crate::backend::reactive::registry::write_shared_from_env(
                    &agent_name,
                    &local_web_url,
                    &block_id,
                );
            }
        }
    }

    Ok(())
}

pub fn register_agent_input_handlers(engine: &Arc<WshRpcEngine>, state: &AppState) {
    // subprocessspawn → spawn agent CLI as subprocess for a single turn
    let wstore_spawn = state.wstore.clone();
    let broker_spawn = state.broker.clone();
    let event_bus_spawn = state.event_bus.clone();
    let filestore_spawn = state.filestore.clone();
    let boot_id_spawn = state.boot_id.clone();
    engine.register_handler(
        COMMAND_SUBPROCESS_SPAWN,
        Box::new(move |data, _ctx| {
            let wstore = wstore_spawn.clone();
            let broker = broker_spawn.clone();
            let event_bus = event_bus_spawn.clone();
            let filestore = filestore_spawn.clone();
            let boot_id = boot_id_spawn.clone();
            Box::pin(async move {
                let cmd: CommandSubprocessSpawnData = serde_json::from_value(data)
                    .map_err(|e| format!("subprocessspawn: {e}"))?;
                tracing::info!(
                    block_id = %cmd.blockid,
                    cli = %cmd.cli_command,
                    "SubprocessSpawn"
                );

                // Get or create a SubprocessController for this block
                let ctrl = match blockcontroller::get_controller(&cmd.blockid) {
                    Some(c) if c.controller_type() == blockcontroller::BLOCK_CONTROLLER_SUBPROCESS => c,
                    _ => {
                        // Create and register a new SubprocessController
                        let registry = wstore.shared_agent_registry();
                        let ctrl = blockcontroller::subprocess::SubprocessController::new(
                            cmd.tabid.clone(),
                            cmd.blockid.clone(),
                            Some(broker),
                            Some(event_bus),
                            Some(wstore),
                            Some(filestore),
                            registry,
                            boot_id,
                        );
                        let ctrl = std::sync::Arc::new(ctrl);
                        ctrl.set_self_ref();
                        blockcontroller::register_controller(&cmd.blockid, ctrl.clone());
                        ctrl as std::sync::Arc<dyn blockcontroller::Controller>
                    }
                };

                // Downcast to SubprocessController to call spawn_turn
                let subprocess_ctrl = ctrl
                    .as_any()
                    .downcast_ref::<blockcontroller::subprocess::SubprocessController>()
                    .ok_or_else(|| "controller is not a SubprocessController".to_string())?;

                let config = blockcontroller::subprocess::SubprocessSpawnConfig {
                    cli_command: cmd.cli_command,
                    cli_args: cmd.cli_args,
                    working_dir: cmd.working_dir,
                    env_vars: cmd.env_vars,
                    message: cmd.message,
                    resume_flag: "--resume".to_string(),
                    resume_strategy: "flag".to_string(),
                    session_id_field: "session_id".to_string(),
                    message_id: None,
                    // Direct-spawn legacy command — caller doesn't
                    // carry a reattach context. Greenfield session id
                    // is None; spawn_turn captures it from CLI stdout
                    // on the first turn as before.
                    session_id: None,
                    // COMMAND_SUBPROCESS_SPAWN has no block to read
                    // agentId from (dead code — unused by the
                    // frontend, per grep); empty disables leasing.
                    instance_id: String::new(),
                };
                subprocess_ctrl.spawn_turn(config)?;
                Ok(None)
            })
        }),
    );

    // agentinput → send message to agent (persistent or per-turn subprocess).
    // Everything the turn needs from AppState now travels as one
    // `AgentTurnDeps` (see its doc comment for why it is a named struct rather
    // than nine separate closure captures).
    let deps_ai = AgentTurnDeps::from_state(state);
    engine.register_handler(
        COMMAND_AGENT_INPUT,
        Box::new(move |data, _ctx| {
            let deps = deps_ai.clone();
            Box::pin(async move {
                let cmd: CommandAgentInputData = serde_json::from_value(data)
                    .map_err(|e| format!("agentinput: {e}"))?;
                tracing::info!(block_id = %cmd.blockid, "AgentInput");
                run_agent_turn(
                    &deps,
                    cmd.blockid,
                    cmd.message,
                    cmd.message_id,
                    TurnRegistration::Register,
                )
                .await?;
                Ok(None)
            })
        }),
    );

    // agentstop → stop the running agent subprocess
    engine.register_handler(
        COMMAND_AGENT_STOP,
        Box::new(|data, _ctx| {
            Box::pin(async move {
                let cmd: CommandAgentStopData = serde_json::from_value(data)
                    .map_err(|e| format!("agentstop: {e}"))?;
                tracing::info!(block_id = %cmd.blockid, force = cmd.force, "AgentStop");
                match blockcontroller::get_controller(&cmd.blockid) {
                    Some(ctrl) => {
                        ctrl.stop(!cmd.force, blockcontroller::STATUS_DONE)?;
                        // Deregister: unregister_block cleans up both agent_to_block and
                        // block_to_agent maps; remove_agent then removes the cloud poll entry
                        // using the logical agent_id recovered from block_to_agent.
                        let handler = crate::backend::reactive::handler::get_global_handler();
                        let agent_name = handler.agent_id_for_block(&cmd.blockid);
                        handler.unregister_block(&cmd.blockid);
                        if let Some(ref name) = agent_name {
                            // Symmetric teardown for the registry writes added
                            // alongside SubprocessSpawn's register_agent call.
                            let data_dir = crate::backend::base::get_wave_data_dir();
                            crate::backend::reactive::registry::remove(&data_dir, name);
                            crate::backend::reactive::registry::remove_shared_from_env(name);
                        }
                        if let (Some(sub), Some(name)) = (
                            crate::muxbus::cloud_subscriber::get_global_subscriber(),
                            agent_name,
                        ) {
                            sub.remove_agent(&name);
                        }
                        Ok(None)
                    }
                    None => Ok(None),
                }
            })
        }),
    );
}

#[cfg(test)]
mod tests {
    use super::git_identity_env_vars;
    use super::{container_argv, flags_with_arity, strip_flag_with_value};

    fn v(xs: &[&str]) -> Vec<String> {
        xs.iter().map(|s| s.to_string()).collect()
    }

    /// The exact argv a container agent carried before the fix — the persistent
    /// controller's args, copied verbatim from the live broken block (Moras,
    /// 2026-08-31).
    fn stale_persistent_argv() -> Vec<String> {
        v(&[
            "--input-format", "stream-json",
            "--output-format", "stream-json",
            "--verbose", "--include-partial-messages",
            "--permission-prompt-tool", "stdio",
            "--permission-mode", "default",
            "--model", "sonnet", "--effort", "high",
        ])
    }

    /// All three persistent-only defects must go, not just the fatal one
    /// (codex P1 on PR #2867): the parse crash, the missing one-shot `-p`, and
    /// the control-protocol permission flags a container turn cannot answer.
    #[test]
    fn rebuilds_a_stale_persistent_argv_into_the_one_shot_form() {
        let got = container_argv(stale_persistent_argv(), "claude");

        assert!(!got.iter().any(|a| a == "--input-format"), "the fatal parse flag must go");
        assert!(got.iter().any(|a| a == "-p"), "one-shot print mode must be present");
        assert!(
            !got.iter().any(|a| a == "--permission-prompt-tool"),
            "the container turn has no control channel to answer can_use_tool",
        );
        assert!(!got.iter().any(|a| a == "stdio"), "its value token must go with it");
        assert!(
            !got.iter().any(|a| a == "--permission-mode"),
            "non-bypass permission mode needs the control protocol",
        );
        assert!(
            got.iter().any(|a| a == "--output-format"),
            "output-format stream-json is how the pane parses the turn at all",
        );
    }

    /// A user's model/effort selections must survive the heal — silently
    /// resetting someone's model on every stale pane would be its own bug.
    #[test]
    fn preserves_the_users_model_and_effort_choices() {
        let got = container_argv(stale_persistent_argv(), "claude");
        let pos = |f: &str| got.iter().position(|a| a == f);
        assert_eq!(got[pos("--model").expect("--model kept") + 1], "sonnet");
        assert_eq!(got[pos("--effort").expect("--effort kept") + 1], "high");
    }

    /// reagent P1 on PR #2867. This runs on EVERY container turn, not once per
    /// stale block, so it must not touch an argv the frontend already built
    /// correctly — a rebuild-from-baseline silently deleted `provider_flags`
    /// and `--fork-session` on every single turn, forever.
    #[test]
    fn leaves_an_already_correct_one_shot_argv_completely_untouched() {
        let correct = v(&[
            "-p", "--output-format", "stream-json",
            "--verbose", "--include-partial-messages",
            "--dangerously-skip-permissions",
            "--exclude-dynamic-system-prompt-sections",
            "--model", "opus",
            "--my-custom-provider-flag", "42",   // agent.provider_flags
            "--fork-session",                    // resolveForkSessionArgs
        ]);
        assert_eq!(container_argv(correct.clone(), "claude"), correct);
    }

    /// …and when it DOES heal a stale argv, those same user-owned tokens still
    /// have to come through. Healing is subtraction, not reconstruction.
    #[test]
    fn a_heal_preserves_provider_flags_and_fork_session_too() {
        let mut stale = stale_persistent_argv();
        stale.extend(v(&["--my-custom-provider-flag", "42", "--fork-session"]));

        let got = container_argv(stale, "claude");

        assert!(got.iter().any(|a| a == "--my-custom-provider-flag"), "provider_flags survive");
        assert_eq!(
            got[got.iter().position(|a| a == "--my-custom-provider-flag").unwrap() + 1],
            "42",
            "…with its value",
        );
        assert!(got.iter().any(|a| a == "--fork-session"), "fork-session survives");
        assert!(!got.iter().any(|a| a == "--input-format"), "while still being healed");
        assert!(got.iter().any(|a| a == "-p"));
    }

    /// An unknown provider has no baseline to diff against, so fall back to
    /// removing only the outright-fatal flag rather than guessing.
    #[test]
    fn an_unknown_provider_falls_back_to_removing_only_the_fatal_flag() {
        let got = container_argv(v(&["--input-format", "stream-json", "--custom"]), "no-such-provider");
        assert_eq!(got, v(&["--custom"]));
    }

    /// A subprocess-shaped provider has no persistent argv, so nothing it was
    /// ever launched with could need healing.
    #[test]
    fn a_provider_with_no_persistent_variant_is_passed_through() {
        let argv = v(&["--json", "--whatever"]);
        assert_eq!(container_argv(argv.clone(), "codex"), argv);
    }

    /// Arity comes off the catalog, not a hardcoded table — this is what lets
    /// a valued flag drop its value and a bare flag not eat the next one.
    #[test]
    fn flags_with_arity_reads_valued_and_bare_flags_off_the_list() {
        static ARGS: &[&str] = &["-p", "--output-format", "stream-json", "--verbose"];
        assert_eq!(
            flags_with_arity(ARGS),
            vec![("-p", false), ("--output-format", true), ("--verbose", false)],
        );
    }

    #[test]
    fn strip_flag_with_value_removes_every_occurrence_and_tolerates_a_missing_value() {
        assert_eq!(
            strip_flag_with_value(v(&["--x", "1", "--keep", "--x", "2"]), "--x"),
            v(&["--keep"]),
        );
        assert_eq!(strip_flag_with_value(v(&["--keep", "--x"]), "--x"), v(&["--keep"]));
        assert_eq!(strip_flag_with_value(vec![], "--x"), Vec::<String>::new());
    }

    #[test]
    fn maps_agent_id_to_name_and_placeholder_email() {
        let vars = git_identity_env_vars("korp");
        let map: std::collections::HashMap<&str, String> = vars.into_iter().collect();
        assert_eq!(map["GIT_AUTHOR_NAME"], "korp");
        assert_eq!(map["GIT_COMMITTER_NAME"], "korp");
        assert_eq!(map["GIT_AUTHOR_EMAIL"], "korp@agentmux.local");
        assert_eq!(map["GIT_COMMITTER_EMAIL"], "korp@agentmux.local");
    }

    #[test]
    fn lowercases_email_but_preserves_display_name_casing() {
        // AGENTMUX_AGENT_ID is natural display casing (e.g. "Korp", per
        // SPEC_PR_TITLE_AGENT_HOST_PREFIX_2026_08_22.md's distinction
        // between the tag's lowercase machine-key and the title's natural
        // casing) -- the git *name* field should read naturally too, but
        // the email's local-part must stay lowercase so it can never
        // collide with a differently-cased but same agent (git/GitHub
        // treat email local-parts as effectively case-sensitive strings
        // for linking purposes; we want exactly one canonical email per
        // agent regardless of what casing happened to be in the block's
        // agentName metadata at spawn time).
        let vars = git_identity_env_vars("Korp");
        let map: std::collections::HashMap<&str, String> = vars.into_iter().collect();
        assert_eq!(map["GIT_AUTHOR_NAME"], "Korp");
        assert_eq!(map["GIT_AUTHOR_EMAIL"], "korp@agentmux.local");
    }

    #[test]
    fn distinct_agents_get_distinct_non_colliding_identities() {
        let a = git_identity_env_vars("agenty");
        let b = git_identity_env_vars("smike");
        let a_map: std::collections::HashMap<&str, String> = a.into_iter().collect();
        let b_map: std::collections::HashMap<&str, String> = b.into_iter().collect();
        assert_ne!(a_map["GIT_AUTHOR_EMAIL"], b_map["GIT_AUTHOR_EMAIL"]);
        assert_ne!(a_map["GIT_AUTHOR_NAME"], b_map["GIT_AUTHOR_NAME"]);
    }

    #[test]
    fn email_domain_is_not_a_real_github_verifiable_domain() {
        // Load-bearing for the fix's safety property: this must NOT be a
        // domain GitHub could ever link to a real account (see the
        // git_identity_env_vars doc comment) -- asserting the exact
        // domain here so a future edit can't accidentally change it to
        // something real (e.g. "users.noreply.github.com") without this
        // test catching it.
        let vars = git_identity_env_vars("camper");
        let map: std::collections::HashMap<&str, String> = vars.into_iter().collect();
        assert!(map["GIT_AUTHOR_EMAIL"].ends_with("@agentmux.local"));
        assert!(map["GIT_COMMITTER_EMAIL"].ends_with("@agentmux.local"));
    }
}
