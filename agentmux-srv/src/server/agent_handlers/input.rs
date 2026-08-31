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

/// Drop `--input-format <value>` from a container agent's argv.
///
/// `--input-format stream-json` tells the CLI that every stdin line is a JSON
/// envelope. That is true for the PERSISTENT controller, which owns a
/// long-lived stdin and writes real envelopes — and false for a container
/// agent, which runs one `docker exec` per turn and whose stdin is written by
/// `container_spawn.rs` as the raw message text (`format!("{}\n", message)`).
///
/// Mixing the two is fatal rather than untidy: the first line the CLI reads is
/// the startup markdown, and it dies with
/// `Error parsing streaming input line: # Session Context: JSON Parse error:
/// Unrecognized token '#'`, EOF'ing the exec in under a second. That is the
/// whole reason container agents have never started (verified live,
/// 2026-08-31).
///
/// The root cause is fixed at the source in `agent-model.ts`, which no longer
/// picks `persistentLaunchArgs` for a container agent. This is the self-heal
/// for blocks ALREADY persisted with the bad argv: `cmd:args` lives in block
/// meta, so without this, every pane created before that fix keeps failing
/// forever with no way back short of recreating the agent. Applied at the
/// point of use so it cannot be bypassed by a block that never re-runs
/// `resync_controller`.
///
/// Removes the flag and its value; tolerates a trailing `--input-format` with
/// no value rather than panicking on a malformed argv.
fn strip_stream_json_input_format(args: Vec<String>) -> Vec<String> {
    let mut out = Vec::with_capacity(args.len());
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        if a == "--input-format" {
            let _ = it.next(); // discard its value
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

    // agentinput → send message to agent (persistent or per-turn subprocess)
    let wstore_ai = state.wstore.clone();
    let id_store_ai = state.id_store.clone();
    let identity_store_ai = state.identity_store.clone();
    // Streaming-bash wrapper auth — clone the per-launch auth_key into the
    // handler's closure so each spawn can inject it into Claude's env.
    // See SPEC_STREAMING_BASH_RUNNER_2026_05_11.md §7.
    let auth_key_ai = state.auth_key.clone();
    // Broker — passed into the identity-injection path so the OAuth
    // expiry probe (PR D, spec §4.4) can publish
    // `identitybundlebindings:changed:<bundle_id>` on status change.
    let broker_ai = state.broker.clone();
    // Container manager — None on hosts without Docker; container agents
    // return an error to the caller rather than crashing the server.
    let container_manager_ai = state.container_manager.clone();
    // Filestore — the spawn-gate error frame must be PERSISTED to the
    // block file, not just live-broadcast (reagent P1, PR #2164 round 2).
    let filestore_ai = state.filestore.clone();
    let local_web_url_ai = state.local_web_url.clone();
    // codex P1, PR #2802: the spawn-gate refusal below never called
    // classify()/persist_last_failure — it only appended a raw
    // error_during_execution frame to the block's output log, so the
    // frontend's structured recovery card (agent:last_failure meta +
    // EVENT_AGENT_FAILURE) never populated for a pre-spawn gate refusal,
    // same host_spawn.rs pattern the POST-spawn exit path already uses.
    let event_bus_ai = state.event_bus.clone();
    engine.register_handler(
        COMMAND_AGENT_INPUT,
        Box::new(move |data, _ctx| {
            let wstore = wstore_ai.clone();
            let id_store = id_store_ai.clone();
            let identity_store = identity_store_ai.clone();
            let auth_key = auth_key_ai.clone();
            let broker = broker_ai.clone();
            let container_manager = container_manager_ai.clone();
            let filestore_gate = filestore_ai.clone();
            let local_web_url = local_web_url_ai.clone();
            let event_bus_gate = event_bus_ai.clone();
            Box::pin(async move {
                let cmd: CommandAgentInputData = serde_json::from_value(data)
                    .map_err(|e| format!("agentinput: {e}"))?;
                tracing::info!(block_id = %cmd.blockid, "AgentInput");

                let ctrl = blockcontroller::get_controller(&cmd.blockid)
                    .ok_or_else(|| format!("no controller for block {}", cmd.blockid))?;

                // Re-read the spawn config from block metadata
                let block: Block = wstore
                    .get(&cmd.blockid)
                    .map_err(|e| format!("agentinput: load block: {e}"))?
                    .ok_or_else(|| format!("block {} not found", cmd.blockid))?;

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
                    cmd.blockid.clone(),
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
                            &cmd.blockid,
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
                            &cmd.blockid,
                            Some(&gate_failure),
                            &Some(wstore.clone()),
                            &Some(event_bus_gate.clone()),
                        );
                        broker.publish(crate::backend::wps::WaveEvent {
                            event: crate::backend::wps::EVENT_AGENT_FAILURE.to_string(),
                            scopes: vec![format!("block:{}", cmd.blockid)],
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
                env_vars.insert("AGENTMUX_BLOCKID".to_string(), cmd.blockid.clone());
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
                        message_id: cmd.message_id.clone(),
                    };
                    persistent_ctrl.send_message(cmd.message, config)?;
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

                        // Ensure container is alive (pull image if needed — P1b).
                        if let Err(e) = cm.ensure_running(&container_name, &container_image, &volumes, &[]).await {
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
                                &cmd.blockid,
                                crate::backend::blockcontroller::subprocess::SUBPROCESS_OUTPUT_SUBJECT,
                                format!("{error_frame}\n").as_bytes(),
                                Some(&filestore_gate),
                                None,
                            );
                            return Err(format!("container ensure_running failed: {e}"));
                        }

                        tracing::info!(
                            block_id = %cmd.blockid,
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
                        let mut base_cmd = vec![container_command];
                        base_cmd.extend(strip_stream_json_input_format(cli_args));

                        let config = blockcontroller::subprocess::SubprocessSpawnConfig {
                            cli_command: String::new(), // unused by spawn_container_turn
                            cli_args: vec![],           // unused by spawn_container_turn
                            working_dir: String::new(), // unused — container has own cwd
                            env_vars,
                            message: cmd.message,
                            resume_flag,
                            resume_strategy,
                            session_id_field,
                            message_id: cmd.message_id,
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
                            message: cmd.message,
                            resume_flag,
                            resume_strategy,
                            session_id_field,
                            message_id: cmd.message_id,
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
                        .register_agent(&agent_name, &cmd.blockid, None);
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
                        if let Some(ctrl) = crate::backend::blockcontroller::get_controller(&cmd.blockid) {
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
                            &cmd.blockid,
                        );
                        crate::backend::reactive::registry::write_shared_from_env(
                            &agent_name,
                            &local_web_url,
                            &cmd.blockid,
                        );
                    }
                }

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
    use super::strip_stream_json_input_format;

    fn v(xs: &[&str]) -> Vec<String> {
        xs.iter().map(|s| s.to_string()).collect()
    }

    /// The exact argv a container agent was getting before the fix — the
    /// persistent controller's args, copied verbatim from a live broken block
    /// (Moras, 2026-08-31). `--input-format stream-json` here is what killed
    /// the exec on its first stdin line.
    #[test]
    fn strips_input_format_and_its_value_from_a_real_broken_argv() {
        let got = strip_stream_json_input_format(v(&[
            "--input-format", "stream-json",
            "--output-format", "stream-json",
            "--verbose", "--include-partial-messages",
            "--permission-prompt-tool", "stdio",
            "--dangerously-skip-permissions",
            "--model", "sonnet", "--effort", "high",
        ]));
        assert!(!got.iter().any(|a| a == "--input-format"), "the flag must be gone");
        // Its VALUE must go too — a bare leftover `stream-json` would be
        // parsed as a positional prompt argument.
        assert_eq!(
            got,
            v(&[
                "--output-format", "stream-json",
                "--verbose", "--include-partial-messages",
                "--permission-prompt-tool", "stdio",
                "--dangerously-skip-permissions",
                "--model", "sonnet", "--effort", "high",
            ]),
        );
    }

    /// `--output-format stream-json` is REQUIRED — it is how the pane parses
    /// the container's output at all. Only the input side is wrong.
    #[test]
    fn leaves_output_format_untouched() {
        let got = strip_stream_json_input_format(v(&["--output-format", "stream-json"]));
        assert_eq!(got, v(&["--output-format", "stream-json"]));
    }

    #[test]
    fn is_a_no_op_on_argv_that_never_had_the_flag() {
        let args = v(&["--output-format", "stream-json", "--verbose"]);
        assert_eq!(strip_stream_json_input_format(args.clone()), args);
        assert_eq!(strip_stream_json_input_format(vec![]), Vec::<String>::new());
    }

    /// A malformed argv (flag with no value) must not panic — this runs on
    /// every container turn, and a stale block could carry anything.
    #[test]
    fn tolerates_a_trailing_input_format_with_no_value() {
        assert_eq!(
            strip_stream_json_input_format(v(&["--verbose", "--input-format"])),
            v(&["--verbose"]),
        );
    }

    /// Defensive: if a block somehow carries the flag twice, remove both
    /// rather than leaving a stray one that reintroduces the failure.
    #[test]
    fn removes_every_occurrence() {
        let got = strip_stream_json_input_format(v(&[
            "--input-format", "stream-json", "--verbose", "--input-format", "stream-json",
        ]));
        assert_eq!(got, v(&["--verbose"]));
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
