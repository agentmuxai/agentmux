// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The spawn environment of a persistent-controller CLI process.

use std::sync::Arc;

/// Builds the full spawn environment for a persistent-controller CLI
/// process: `cmd:env` base → Layer 3 identity/credential gate → the two
/// RESERVED wrapper variables (unconditionally overwritten —
/// `AGENTMUX_AUTH_KEY`/`AGENTMUX_BLOCKID`) → the two agent-identity
/// variables and per-agent git identity (from the row's slug when it has
/// one, `identity_env`) → bundled/user tools PATH → the reserved guards (`gh_guard`,
/// `account_login_guard`: no agent gets a human's `gh` login or the
/// account's cloud login).
///
/// Shared by every real spawn of a persistent-controller CLI — this
/// function's own logic used to live inline in `run_agent_turn` below, and
/// only there. `PersistentSubprocessController::start()`'s eager-resume
/// path (`SPEC_PERSISTENT_CONTROLLER_EAGER_RESUME_ON_RECONNECT_2026_09_20.md`)
/// built its own second, independent copy of a SUBSET of this — codex P1 on
/// PR #3513 found it had already drifted from this one: missing PATH and
/// (then) MuxBus-token injection entirely, and using `entry().or_insert()` instead
/// of an unconditional overwrite for the two reserved wrapper variables
/// (meaning a stale persisted `cmd:env` value for either would silently
/// survive across an eager resume, unlike a live message send). One
/// function, not two copies kept in sync by hand.
///
/// `Err(SpawnGateError)` means the caller must NOT spawn — same contract as
/// `inject_identity_env_async` itself, which this wraps.
pub(crate) async fn build_persistent_spawn_env(
    mstore: Arc<crate::backend::storage::store::Store>,
    id_store: Arc<crate::backend::storage::store::Store>,
    identity_store: Arc<crate::backend::storage::store::Store>,
    broker: Option<Arc<crate::backend::mps::Broker>>,
    block_meta: &crate::backend::obj::MetaMapType,
    block_id: &str,
    auth_key: &str,
    base_env_vars: std::collections::HashMap<String, String>,
) -> Result<std::collections::HashMap<String, String>, crate::identity::resolver::SpawnGateError> {
    // Identity injection: look up the active AgentInstance for this block,
    // resolve its identity_id's bindings, and merge each per-provider env
    // var into the spawn map. Api-key-class failures are logged and
    // skipped — the agent CLI launches with whatever resolved cleanly plus
    // the static cmd:env block. Oauth-class resolution failures are
    // BLOCKING unless the agent opted into ambient login (layer-3 spawn
    // gate, SPEC_ACCOUNT_DELETE_DEAUTH_LAYERS_2_4_2026_07_14.md §2.2).
    // Kept for the identity-M0 slug lookup below; `mstore` itself moves into
    // the identity injector.
    let mstore_for_slug = mstore.clone();
    let mut env_vars =
        super::endpoint::inject_identity(mstore, id_store, identity_store, broker, block_meta, block_id, base_env_vars).await?;

    // No MuxBus account login here: agents reach the cloud through the srv,
    // which uses its own stored login (see `account_login_guard`).

    // Streaming-bash wrapper auth + discovery
    // (SPEC_STREAMING_BASH_RUNNER_2026_05_11.md §7).
    //
    // 1. AGENTMUX_AUTH_KEY — config.rs's `Config::from_env_and_args` removed
    //    it from the process env at startup (security PR #801). Re-inject for this spawn so the
    //    wrapper (running inside Claude's bash subprocess tree) can
    //    authenticate against the auth_middleware-gated
    //    /agentmux/wps/publish endpoint via X-AuthKey.
    // 2. PATH — prepend the bundled tools/bin dir so `agentmux-bashwrap.exe`
    //    resolves when the PreToolUse hook (auto-injected by
    //    agent_config.rs) rewrites the command to invoke it.
    //    AGENTMUX_LOCAL_URL is already in the inherited process env
    //    (set by `bind_listeners_and_network` in bootstrap/network.rs).
    //
    // Unconditional `insert`, NOT `entry().or_insert()` — these two are
    // RESERVED, server-controlled values (the current key, the real block
    // id), not user configuration. A persisted `cmd:env` carrying a stale
    // value for either (e.g. from a prior server restart with a different
    // key) must not survive into this spawn.
    env_vars.insert("AGENTMUX_AUTH_KEY".to_string(), auth_key.to_string());
    // Block id so the wrapper can scope its MPS publishes to `block:<id>`.
    // Without this, chunks publish without a scope and the frontend's
    // per-block subscription doesn't receive them.
    env_vars.insert("AGENTMUX_BLOCKID".to_string(), block_id.to_string());

    // Identity M0 + M1. M0: name and slug as two explicit variables, on this
    // path too — the frontend's `launchAgentDefinition` already sets both;
    // this path never did, so a reader could only get at either by guessing
    // what `AGENTMUX_AGENT_ID` held (`split_agent_identity_env`). M1: the
    // UID and the agent's token, from the row the server itself created
    // (`carry_agent_uid_env`). One store round-trip serves both.
    {
        let display_name = crate::backend::obj::meta_get_string(block_meta, "agentName", "");
        // P0.3: the workspace `.mcp.json` follows the slug too.
        let meta = |key, default| crate::backend::obj::meta_get_string(block_meta, key, default);
        let (cwd, mode) = (meta("cmd:cwd", ""), meta("agentMode", "host"));
        let identity = {
            let mstore = mstore_for_slug.clone();
            let owned_block_id = block_id.to_string();
            match tokio::task::spawn_blocking(move || {
                let identity = super::block_identity::persisted_agent_identity(&mstore, &owned_block_id);
                if let Some(slug) = identity.as_ref().and_then(|i| i.slug.as_deref()) {
                    crate::backend::mcp_json_identity::realign_workspace_mcp_json(&mstore, &cwd, &mode, slug);
                }
                // #3577, re-checked here: this is the last store read before
                // the process starts. The gate refused a deleted agent, but
                // the agent can be deleted after the gate's lookup succeeded
                // (the delete handlers share no lock with the spawn path) —
                // then no row resolves here, and without this the spawn
                // would go ahead with no identity (Codex P1 on #3591). A
                // delete after THIS read is a delete of a running agent.
                let deleted =
                    super::block_identity::deleted_agent_of_rowless_block(&mstore, &owned_block_id, identity.as_ref());
                (identity, deleted)
            })
            .await
            {
                Ok((_, Some(agent_id))) => {
                    return Err(crate::identity::resolver::SpawnGateError::AgentDeleted {
                        agent_id,
                    });
                }
                Ok((identity, None)) => identity,
                Err(e) => {
                    tracing::warn!(
                        block_id = %block_id,
                        error = %e,
                        "spawn env: persisted-identity lookup task failed — no UID/token this spawn; AGENTMUX_AGENT_SLUG falls back to cmd:env"
                    );
                    None
                }
            }
        };
        super::block_identity::split_agent_identity_env(
            &mut env_vars,
            &display_name,
            identity.as_ref().and_then(|i| i.slug.as_deref()),
        );
        super::block_identity::carry_agent_uid_env(&mut env_vars, identity.as_ref());
        // The id, its MuxBus mirror and the git identity: from the row's slug
        // when it has one, over `cmd:env` (see `identity_env`).
        super::identity::carry_agent_identity_env(&mut env_vars, &display_name, identity.as_ref().and_then(|i| i.slug.as_deref()));
    }
    super::block_identity::carry_agent_workdir_env(&mut env_vars, block_meta);
    carry_model_env(&mstore_for_slug, block_id, block_meta, &mut env_vars).await;
    super::workspaces::record_launch_workspace(block_id, &env_vars);
    // PATH gets the bundled tools dir (portable builds, runtime/tools/bin/)
    // and the user tools dir (~/.agentmux/tools/bin/), both ahead of the
    // inherited PATH: bundled is None in `task dev` (tool_store excludes
    // target/debug), so the wrapper is in the user dir and a system copy must
    // not shadow it. See `UserToolsPrecedence`.
    crate::backend::tool_store::prepend_agent_tools_path(&mut env_vars);
    super::bash_prefix::apply_for_spawn(&mut env_vars, block_id, block_meta);
    // Plain `gh` must not act as a human's gh login (see `gh_guard`), and no
    // agent holds the account's cloud login (see `account_login_guard`). Last,
    // so nothing above — a persisted `cmd:env`, an identity binding — can set
    // them back: these are reserved variables, not defaults.
    crate::backend::gh_guard::apply_gh_guard(&mut env_vars);
    crate::backend::account_login_guard::strip_account_login(&mut env_vars);
    // AgentMux agents message each other through jekt: the CLI's separate messaging is off by default.
    crate::backend::cli_messaging::apply_for_spawn(&mut env_vars, block_meta);

    Ok(env_vars)
}

/// The models an agent runs on: an agent on a model route is pointed at the gateway, then the sub-agent model
/// is chosen, which keeps a model the route has already set.
async fn carry_model_env(mstore: &Arc<crate::backend::storage::store::Store>, block_id: &str, block_meta: &crate::backend::obj::MetaMapType, env_vars: &mut std::collections::HashMap<String, String>) {
    let (mstore, block, mut route_env) = (mstore.clone(), block_id.to_string(), env_vars.clone());
    let carried = tokio::task::spawn_blocking(move || {
        crate::backend::model_gateway::launch::carry_for_block(&mstore, &block, &mut route_env);
        route_env
    })
    .await;
    if let Ok(carried) = carried {
        *env_vars = carried;
    }
    crate::backend::subagent_model::carry(env_vars, block_meta);
}
