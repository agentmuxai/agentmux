// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use std::sync::Arc;

use crate::backend::blockcontroller;
use crate::backend::obj::Block;
use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::rpc_types::{
    CommandAgentInputData, CommandAgentStopData, CommandSubprocessSpawnData, COMMAND_AGENT_INPUT,
    COMMAND_AGENT_STOP, COMMAND_SUBPROCESS_SPAWN,
};

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
        let takes_value = args.get(i + 1).is_some_and(|next| !next.starts_with('-'));
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

/// Identity M0 — `SPEC_AGENT_IDENTITY_CARRIED_NOT_DERIVED_2026_09_23.md`
/// §9.1: make the display name and the slug two explicit variables, present
/// on every launch path.
///
/// **Why.** `AGENTMUX_AGENT_ID` means two different things depending on who
/// spawned the agent (spec §0.2): the frontend's `launchAgentDefinition`
/// writes the definition *slug* into it (`agent-model.ts`), while this
/// server path's fallback writes the *display name*
/// (`block.meta["agentName"]`, a few lines above the call site). Nothing
/// downstream can tell which it got. The frontend path already sets
/// `AGENTMUX_AGENT_DISPLAY` and `AGENTMUX_AGENT_SLUG` alongside it; this
/// path set neither, so a reader that wanted the unambiguous form had
/// nothing to read. After this, both are always present when the server
/// knows them, and later phases can migrate readers off `AGENTMUX_AGENT_ID`
/// one at a time.
///
/// **Precedence.** The server's own knowledge wins, `cmd:env` is the
/// fallback — the reverse of the "user-provided values take precedence"
/// rule the surrounding identity variables follow, deliberately:
///
/// - `display_name` is `block.meta["agentName"]`, the value the primary
///   reactive registration is keyed on (see the Register-tail of
///   `run_agent_turn`) and the name the pane header shows. A `cmd:env` copy
///   is a launch-time snapshot that a rename (`setViewName`) leaves stale.
/// - `persisted_slug` is `db_agents.slug` for this block's agent — the
///   collision-resolved value `agent_def_insert` minted. A `cmd:env` copy is
///   whatever the frontend computed before that row existed.
///
/// When the two disagree it is logged, not silently resolved: a disagreement
/// is exactly the kind of signal spec §9.2 says a phase must be able to
/// observe.
///
/// **What this does not do.** `AGENTMUX_AGENT_ID` is never read or written
/// here. Spec §9.4 keeps its present value on both paths through M4; the
/// git identity, `MUXBUS_AGENT_ID`, the PR-body tag and the OSC prompt hook
/// all still derive from it, unchanged.
pub(crate) fn split_agent_identity_env(
    env_vars: &mut std::collections::HashMap<String, String>,
    display_name: &str,
    persisted_slug: Option<&str>,
) {
    let set_authoritative = |env_vars: &mut std::collections::HashMap<String, String>,
                             key: &str,
                             value: &str| {
        let value = value.trim();
        if value.is_empty() {
            return;
        }
        if let Some(existing) = env_vars
            .get(key)
            .map(|v| v.trim())
            .filter(|v| !v.is_empty())
        {
            if existing != value {
                tracing::info!(
                    key,
                    cmd_env = %existing,
                    authoritative = %value,
                    "spawn env: cmd:env disagrees with the server's value — server wins (identity M0)"
                );
            }
        }
        env_vars.insert(key.to_string(), value.to_string());
    };
    set_authoritative(env_vars, "AGENTMUX_AGENT_DISPLAY", display_name);
    if let Some(slug) = persisted_slug {
        set_authoritative(env_vars, "AGENTMUX_AGENT_SLUG", slug);
    }
}

/// What the server knows about the agent on a block, read from its
/// `db_agents` row: the identity that will be carried into the process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PersistedAgentIdentity {
    /// `db_agents.id` — the UID (spec §3). A UUID for every non-template row.
    pub uid: String,
    /// `db_agents.slug`, the collision-resolved value `agent_def_insert`
    /// minted. `None` if the row's slug is empty.
    pub slug: Option<String>,
    /// The agent's local identity token (`storage/agent_tokens.rs`), minted
    /// on first spawn. `None` only if minting failed — logged, and the
    /// spawn proceeds without it, because M1 has no reader for it yet.
    pub token: Option<String>,
}

/// The `db_agents` row for the agent shown on `block_id`, if it has one,
/// plus that agent's token.
///
/// Resolves the block → row the same way the identity injector does
/// (`identity/resolver/inject.rs`): `block.meta.agentId` first, then the
/// latest active launch on this block. A quick-launch pane (`launchAgent`
/// in `agent-model.ts`, whose `agentId` is a provider key like `"claude"`)
/// has no row and gets `None` — it has no slug or UID in the identity
/// sense, and deriving either from its display name would recreate the
/// lossy path spec §1.1 measured as the root defect.
///
/// The token is minted here, at spawn, rather than at row creation: every
/// row that exists today predates the table, so "mint at creation" would
/// leave them all without one. `agent_token_ensure` is idempotent, so a
/// row created in the future gets the same token on every spawn.
///
/// A store error is logged and treated as "unknown", never as a spawn
/// failure: this is disambiguation, not a gate, and spec §10 constraint 1
/// forbids a store read from making a healthy agent unreachable.
fn persisted_agent_identity(
    mstore: &crate::backend::storage::store::Store,
    block_id: &str,
) -> Option<PersistedAgentIdentity> {
    let instance = match mstore.instance_get_active_for_block(block_id) {
        Ok(Some(instance)) if !instance.id.trim().is_empty() => instance,
        Ok(_) => return None,
        Err(e) => {
            tracing::warn!(block_id, error = %e, "spawn env: instance lookup failed — no persisted identity from store");
            return None;
        }
    };
    let slug = match mstore.agent_def_get(&instance.definition_id) {
        Ok(Some(def)) if !def.slug.trim().is_empty() => Some(def.slug),
        Ok(_) => None,
        Err(e) => {
            tracing::warn!(block_id, error = %e, "spawn env: definition lookup failed — no AGENTMUX_AGENT_SLUG from store");
            None
        }
    };
    let token = match mstore.agent_token_ensure(&instance.id) {
        Ok(token) => Some(token),
        Err(e) => {
            tracing::warn!(block_id, uid = %instance.id, error = %e, "spawn env: token mint failed — spawning without AGENTMUX_AGENT_TOKEN");
            None
        }
    };
    Some(PersistedAgentIdentity {
        uid: instance.id,
        slug,
        token,
    })
}

/// Identity M1 — spec §9.1: carry the UID and the token into the process.
///
/// `AGENTMUX_AGENT_UID` and `AGENTMUX_AGENT_TOKEN` are RESERVED,
/// server-controlled values in exactly the sense `AGENTMUX_AUTH_KEY` and
/// `AGENTMUX_BLOCKID` are at the top of `build_persistent_spawn_env`:
/// unconditional `insert`, never `entry().or_insert()`. A persisted
/// `cmd:env` carrying a stale value for either — a pane reused for a
/// different agent, a copied block — must not survive into this spawn, and
/// for the token specifically a stale value would be *another agent's*
/// credential. So when the server knows the identity it overwrites, and
/// when it does not (no row: quick-launch pane, or a lookup failure) it
/// **removes** both rather than leaving whatever was there.
///
/// Spec §4.1: identity is asserted by whoever minted it. The frontend is
/// not consulted (its `agentInstanceId` is documented as going stale on
/// pane reuse), and the agent is not asked. The row is the source.
///
/// Descendant inheritance is real and not claimed otherwise (spec §6.4):
/// the agent's own subprocesses — its shell tool, its MCP servers, build
/// scripts — inherit the token. That is a smaller blast radius than the
/// instance-wide `AGENTMUX_AUTH_KEY` they already inherit, not zero. The
/// `.mcp.json` `agent_config.rs` writes into the working directory carries
/// neither variable (it copies only `AGENTMUX_AGENT_ID`/`_BUS_ID`), so the
/// token never lands in a file a human may commit.
pub(crate) fn carry_agent_uid_env(
    env_vars: &mut std::collections::HashMap<String, String>,
    identity: Option<&PersistedAgentIdentity>,
) {
    const UID: &str = "AGENTMUX_AGENT_UID";
    const TOKEN: &str = "AGENTMUX_AGENT_TOKEN";
    match identity {
        Some(id) => {
            env_vars.insert(UID.to_string(), id.uid.clone());
            match &id.token {
                Some(token) => {
                    env_vars.insert(TOKEN.to_string(), token.clone());
                }
                None => {
                    env_vars.remove(TOKEN);
                }
            }
        }
        None => {
            env_vars.remove(UID);
            env_vars.remove(TOKEN);
        }
    }
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
    pub mstore: Arc<crate::backend::storage::store::Store>,
    pub id_store: Arc<crate::backend::storage::store::Store>,
    pub identity_store: Arc<crate::backend::storage::store::Store>,
    /// Streaming-bash wrapper auth key — see SPEC_STREAMING_BASH_RUNNER_2026_05_11.md §7.
    pub auth_key: String,
    pub broker: Arc<crate::backend::mps::Broker>,
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
            mstore: state.mstore.clone(),
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

/// Builds the full spawn environment for a persistent-controller CLI
/// process: `cmd:env` base → Layer 3 identity/credential gate → MuxBus
/// cloud token → the two RESERVED wrapper variables (unconditionally
/// overwritten — `AGENTMUX_AUTH_KEY`/`AGENTMUX_BLOCKID`) → the two agent-
/// identity variables and per-agent git identity (all user-overridable via
/// `cmd:env`) → bundled/user tools PATH.
///
/// Shared by every real spawn of a persistent-controller CLI — this
/// function's own logic used to live inline in `run_agent_turn` below, and
/// only there. `PersistentSubprocessController::start()`'s eager-resume
/// path (`SPEC_PERSISTENT_CONTROLLER_EAGER_RESUME_ON_RECONNECT_2026_09_20.md`)
/// built its own second, independent copy of a SUBSET of this — codex P1 on
/// PR #3513 found it had already drifted from this one: missing PATH and
/// MuxBus-token injection entirely, and using `entry().or_insert()` instead
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
    let mut env_vars = crate::identity::resolver::inject_identity_env_async(
        mstore,
        id_store.clone(),
        identity_store,
        broker,
        block_id.to_string(),
        base_env_vars,
    )
    .await?;

    // MuxBus cloud token — injects MUXBUS_TOKEN + MUXBUS_COGNITO_DOMAIN if
    // the user has authenticated via muxbus.login. No-op if no credentials
    // are stored. Auto-refreshes if token is nearly expired.
    crate::server::muxbus_handlers::inject_muxbus_env(&id_store, &mut env_vars).await;

    // Streaming-bash wrapper auth + discovery
    // (SPEC_STREAMING_BASH_RUNNER_2026_05_11.md §7).
    //
    // 1. AGENTMUX_AUTH_KEY — config.rs:42 removed it from the process env
    //    at startup (security PR #801). Re-inject for this spawn so the
    //    wrapper (running inside Claude's bash subprocess tree) can
    //    authenticate against the auth_middleware-gated
    //    /agentmux/wps/publish endpoint via X-AuthKey.
    // 2. PATH — prepend the bundled tools/bin dir so `agentmux-bashwrap.exe`
    //    resolves when the PreToolUse hook (auto-injected by
    //    agent_config.rs) rewrites the command to invoke it.
    //    AGENTMUX_LOCAL_URL is already in the inherited process env
    //    (main.rs:498).
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

    // Agent display name for MuxBus self-identification. AGENTMUX_AGENT_ID
    // is the canonical, app-wide agent-identity variable (used far beyond
    // muxbus -- MCP tool routing, native memory, shell OSC titling, jekt
    // auto-registration, etc; see this repo's CLAUDE.md Naming Conventions
    // table). MUXBUS_AGENT_ID mirrors the same value so muxbus-client picks
    // it up under the MUXBUS_* prefix it already checks first (alongside
    // MUXBUS_TOKEN/MUXBUS_COGNITO_DOMAIN injected above) -- this does NOT
    // make MUXBUS_AGENT_ID a second source of truth for agent identity
    // app-wide, it's scoped to this one muxbus hand-off point (ARCH-002,
    // 2026-07-28 architecture analyst report). Only set if not already
    // present in cmd:env — user-provided values take precedence, unlike the
    // two reserved variables above.
    if !env_vars.contains_key("AGENTMUX_AGENT_ID") {
        let agent_display_name = crate::backend::obj::meta_get_string(block_meta, "agentName", "");
        if !agent_display_name.is_empty() {
            env_vars.insert("AGENTMUX_AGENT_ID".to_string(), agent_display_name);
        }
    }
    if !env_vars.contains_key("MUXBUS_AGENT_ID") {
        if let Some(agent_id) = env_vars.get("AGENTMUX_AGENT_ID").cloned() {
            env_vars.insert("MUXBUS_AGENT_ID".to_string(), agent_id);
        }
    }
    // Identity M0 + M1. M0: name and slug as two explicit variables, on this
    // path too — the frontend's `launchAgentDefinition` already sets both;
    // this path never did, so a reader could only get at either by guessing
    // what `AGENTMUX_AGENT_ID` held (`split_agent_identity_env`). M1: the
    // UID and the agent's token, from the row the server itself created
    // (`carry_agent_uid_env`). One store round-trip serves both.
    {
        let display_name = crate::backend::obj::meta_get_string(block_meta, "agentName", "");
        let identity = {
            let mstore = mstore_for_slug.clone();
            let owned_block_id = block_id.to_string();
            match tokio::task::spawn_blocking(move || {
                persisted_agent_identity(&mstore, &owned_block_id)
            })
            .await
            {
                Ok(identity) => identity,
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
        split_agent_identity_env(
            &mut env_vars,
            &display_name,
            identity.as_ref().and_then(|i| i.slug.as_deref()),
        );
        carry_agent_uid_env(&mut env_vars, identity.as_ref());
    }
    // Per-agent git commit identity -- see git_identity_env_vars() doc
    // comment. Still overridable per the same "user-provided values take
    // precedence" rule as every other var here.
    if let Some(agent_id) = env_vars.get("AGENTMUX_AGENT_ID").cloned() {
        for (key, value) in git_identity_env_vars(&agent_id) {
            if !env_vars.contains_key(key) {
                env_vars.insert(key.to_string(), value);
            }
        }
    }
    // PATH includes BOTH bundled tools dir (portable builds,
    // runtime/tools/bin/) AND user tools dir (~/.agentmux/tools/bin/).
    // bundled is None in dev mode (target/debug exclusion in tool_store), so
    // without user_tools_dir the wrapper wouldn't be on the agent's PATH
    // during `task dev`.
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

    Ok(env_vars)
}

pub async fn run_agent_turn(
    deps: &AgentTurnDeps,
    block_id: String,
    message: String,
    message_id: Option<String>,
    registration: TurnRegistration,
) -> Result<(), String> {
    let AgentTurnDeps {
        mstore,
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
    let block: Block = mstore
        .get(&block_id)
        .map_err(|e| format!("agentinput: load block: {e}"))?
        .ok_or_else(|| format!("block {} not found", block_id))?;

    let cli_command = crate::backend::obj::meta_get_string(&block.meta, "cmd", "claude");
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
    let working_dir = crate::backend::obj::meta_get_string(&block.meta, "cmd:cwd", "");
    let env_vars: std::collections::HashMap<String, String> = match block.meta.get("cmd:env") {
        Some(serde_json::Value::Object(obj)) => obj
            .iter()
            .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
            .collect(),
        _ => std::collections::HashMap::new(),
    };
    // Identity gate, MuxBus token, reserved wrapper vars, agent identity,
    // git identity, tools PATH — see `build_persistent_spawn_env`'s own doc
    // comment. Broker hand-in lets the OAuth expiry probe (PR D, spec §4.4)
    // publish `identitybundlebindings:changed:<bundle_id>` when it flips a
    // token's status valid→expired etc.
    let mut env_vars = match build_persistent_spawn_env(
        mstore.clone(),
        id_store.clone(),
        identity_store.clone(),
        Some(broker.clone()),
        &block.meta,
        &block_id,
        &auth_key,
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
            let gate_failure =
                crate::agents::failure::classify(None, None, &gate.to_string(), None);
            crate::backend::blockcontroller::core::persist_last_failure(
                &block_id,
                Some(&gate_failure),
                &Some(mstore.clone()),
                &Some(event_bus_gate.clone()),
            );
            broker.publish(crate::backend::mps::MuxEvent {
                event: crate::backend::mps::EVENT_AGENT_FAILURE.to_string(),
                scopes: vec![format!("block:{}", block_id)],
                sender: String::new(),
                persist: 1,
                data: serde_json::to_value(&gate_failure).ok(),
            });
            return Err(format!("identity spawn gate: {gate}"));
        }
    };

    let session_id_field =
        crate::backend::obj::meta_get_string(&block.meta, "agent:session_id_field", "session_id");

    // App Server owns a persistent thread/turn process and has its own typed
    // protocol path. Keep it ahead of the legacy stream-json branches.
    //
    // KNOWN GAP (ReAgent P1, PR #3215, tracked for a follow-up PR, not fixed
    // here): env_vars above (including this turn's freshly-resolved identity
    // bindings from inject_identity_env_async) is computed but never reaches
    // this branch — send_message carries no env, and the App Server child was
    // already spawned earlier using only command_from_meta's cmd:env snapshot.
    // Unlike the subprocess/persistent controllers, this isn't a missing
    // plumbing call: env vars are fixed at process exec() time, so an
    // already-running App Server process cannot pick up a per-turn identity
    // change at all without being torn down and respawned. Deciding when to
    // force that respawn (e.g. on every identity-binding change vs. only
    // between turns) is a real design question, not a mechanical fix, so it's
    // deliberately left for a follow-up PR rather than rushed in here.
    if let Some(app_server_ctrl) =
        ctrl.as_any()
            .downcast_ref::<blockcontroller::app_server_controller::AppServerController>()
    {
        app_server_ctrl.send_message(message)?;
    }
    // Try persistent controller first, fall back to subprocess
    else if let Some(persistent_ctrl) =
        ctrl.as_any()
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
        let resume_flag =
            crate::backend::obj::meta_get_string(&block.meta, "agent:resume_flag", "--resume");
        let persisted_session_id =
            crate::backend::obj::meta_get_string(&block.meta, "agent:sessionid", "");
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
    } else if let Some(subprocess_ctrl) =
        ctrl.as_any()
            .downcast_ref::<blockcontroller::subprocess::SubprocessController>()
    {
        let resume_flag =
            crate::backend::obj::meta_get_string(&block.meta, "agent:resume_flag", "--resume");
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
        let persisted_session_id =
            crate::backend::obj::meta_get_string(&block.meta, "agent:sessionid", "");

        // Container agent branch: use Docker socket API exec (P1a: no
        // secrets in argv). Host agent branch: regular CLI subprocess.
        let agent_mode = crate::backend::obj::meta_get_string(&block.meta, "agentMode", "host");
        // Cross-process session-lease key (registry::LeaseStore) —
        // read once here for both branches below. Only the host
        // branch's spawn_turn actually enforces it in this PR;
        // the container branch's config field is unused for now
        // (struct-completeness — see host_spawn.rs's doc comment).
        let instance_id = crate::backend::obj::meta_get_string(&block.meta, "agentId", "");
        if agent_mode == "container" {
            let cm = container_manager.get().await.ok_or_else(|| {
                "Docker not available on this host; cannot start container agent".to_string()
            })?;
            let container_image = {
                let img =
                    crate::backend::obj::meta_get_string(&block.meta, "agent:container_image", "");
                if img.is_empty() {
                    "ghcr.io/agentmuxai/agent-claude:latest".to_string()
                } else {
                    img
                }
            };
            // Use agentId (UUID) — always valid as a Docker name; display names can have spaces.
            let agent_id = crate::backend::obj::meta_get_string(&block.meta, "agentId", "");
            let container_name = crate::backend::container::container_name_for_slug(&agent_id);
            let volumes_json =
                crate::backend::obj::meta_get_string(&block.meta, "agent:container_volumes", "[]");
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

            // AGENTMUX_LOCAL_URL is never explicitly set in env_vars for a host
            // agent either (see the PATH-block comment above) — a host
            // subprocess just inherits it from this process's own env
            // (bootstrap.rs). `docker exec` over the Docker socket never
            // inherits host process env, so a container agent gets no sidecar
            // URL at all unless it's injected here. Rewritten to
            // host.docker.internal (routable from inside the container via
            // the extra_hosts entry `create_and_start` sets — see
            // container.rs) since the loopback address this process sees is
            // not reachable from the container's own network namespace.
            // #2939 workstream 1 (host integration) — this alone doesn't
            // finish that workstream: agentmux-mcp/agentmux-bashwrap also
            // need to actually exist in the container image, which they do
            // not yet.
            if let Ok(local_url) = std::env::var("AGENTMUX_LOCAL_URL") {
                env_vars.insert(
                    "AGENTMUX_LOCAL_URL".to_string(),
                    crate::backend::container::rewrite_local_url_for_container(&local_url),
                );
            }

            // Ensure container is alive (pull image if needed — P1b).
            if let Err(e) = cm
                .ensure_running(
                    &container_name,
                    &container_image,
                    &volumes,
                    &[],
                    &mount_spec,
                )
                .await
            {
                // Surface the error in the agent pane before returning, so the user
                // sees why the container failed (image not found, Docker down, etc.).
                let error_frame = serde_json::json!({
                    "type": "result",
                    "is_error": true,
                    "subtype": "error_during_execution",
                    "error": {"message": format!("[AgentMux] container ensure_running failed: {e}")}
                })
                .to_string();
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
                &block.meta,
                "agent:container_command",
                "claude",
            );
            // Provider id for the one-shot argv rebuild below.
            let agent_provider =
                crate::backend::obj::meta_get_string(&block.meta, "agentProvider", "claude");
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
        return Err(
            "controller is not a SubprocessController or PersistentSubprocessController"
                .to_string(),
        );
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
        let agent_name = crate::backend::obj::meta_get_string(&block.meta, "agentName", "");
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
                let data_dir = crate::backend::base::get_mux_data_dir();
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
    let mstore_spawn = state.mstore.clone();
    let broker_spawn = state.broker.clone();
    let event_bus_spawn = state.event_bus.clone();
    let filestore_spawn = state.filestore.clone();
    let boot_id_spawn = state.boot_id.clone();
    engine.register_typed(
        COMMAND_SUBPROCESS_SPAWN,
        move |cmd: CommandSubprocessSpawnData, _ctx| {
            let mstore = mstore_spawn.clone();
            let broker = broker_spawn.clone();
            let event_bus = event_bus_spawn.clone();
            let filestore = filestore_spawn.clone();
            let boot_id = boot_id_spawn.clone();
            async move {
                tracing::info!(
                    block_id = %cmd.blockid,
                    cli = %cmd.cli_command,
                    "SubprocessSpawn"
                );

                // Get or create a SubprocessController for this block
                let ctrl = match blockcontroller::get_controller(&cmd.blockid) {
                    Some(c)
                        if c.controller_type() == blockcontroller::BLOCK_CONTROLLER_SUBPROCESS =>
                    {
                        c
                    }
                    _ => {
                        // Create and register a new SubprocessController
                        let registry = mstore.shared_agent_registry();
                        let ctrl = blockcontroller::subprocess::SubprocessController::new(
                            cmd.tabid.clone(),
                            cmd.blockid.clone(),
                            Some(broker),
                            Some(event_bus),
                            Some(mstore),
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
                Ok(())
            }
        },
    );

    // agentinput → send message to agent (persistent or per-turn subprocess).
    // Everything the turn needs from AppState now travels as one
    // `AgentTurnDeps` (see its doc comment for why it is a named struct rather
    // than nine separate closure captures).
    let deps_ai = AgentTurnDeps::from_state(state);
    engine.register_typed(
        COMMAND_AGENT_INPUT,
        move |cmd: CommandAgentInputData, _ctx| {
            let deps = deps_ai.clone();
            async move {
                tracing::info!(block_id = %cmd.blockid, "AgentInput");
                // Authoritative hidden-window gate for ambient digest reads
                // (next_prompt_suggestion / activity_summary / activity_watcher)
                // — set here, not reconstructed from transcript bytes. See
                // session::set_hidden_reinjection_active's doc comment.
                crate::server::app_api::session::set_hidden_reinjection_active(
                    &cmd.blockid,
                    cmd.hidden.unwrap_or(false),
                );
                run_agent_turn(
                    &deps,
                    cmd.blockid,
                    cmd.message,
                    cmd.message_id,
                    TurnRegistration::Register,
                )
                .await?;
                Ok(())
            }
        },
    );

    // agentstop → stop the running agent subprocess
    engine.register_typed(
        COMMAND_AGENT_STOP,
        |cmd: CommandAgentStopData, _ctx| {
            async move {
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
                            let data_dir = crate::backend::base::get_mux_data_dir();
                            crate::backend::reactive::registry::remove(&data_dir, name);
                            crate::backend::reactive::registry::remove_shared_from_env(name);
                        }
                        if let (Some(sub), Some(name)) = (
                            crate::muxbus::cloud_subscriber::get_global_subscriber(),
                            agent_name,
                        ) {
                            sub.remove_agent(&name);
                        }
                        Ok(())
                    }
                    // Stopping a block with no controller is a no-op, not an
                    // error: the UI cannot tell "already stopped" from "never
                    // started", and either way the caller's intent is
                    // satisfied.
                    None => Ok(()),
                }
            }
        },
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
            "--input-format",
            "stream-json",
            "--output-format",
            "stream-json",
            "--verbose",
            "--include-partial-messages",
            "--permission-prompt-tool",
            "stdio",
            "--permission-mode",
            "default",
            "--model",
            "sonnet",
            "--effort",
            "high",
        ])
    }

    /// All three persistent-only defects must go, not just the fatal one
    /// (codex P1 on PR #2867): the parse crash, the missing one-shot `-p`, and
    /// the control-protocol permission flags a container turn cannot answer.
    #[test]
    fn rebuilds_a_stale_persistent_argv_into_the_one_shot_form() {
        let got = container_argv(stale_persistent_argv(), "claude");

        assert!(
            !got.iter().any(|a| a == "--input-format"),
            "the fatal parse flag must go"
        );
        assert!(
            got.iter().any(|a| a == "-p"),
            "one-shot print mode must be present"
        );
        assert!(
            !got.iter().any(|a| a == "--permission-prompt-tool"),
            "the container turn has no control channel to answer can_use_tool",
        );
        assert!(
            !got.iter().any(|a| a == "stdio"),
            "its value token must go with it"
        );
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
            "-p",
            "--output-format",
            "stream-json",
            "--verbose",
            "--include-partial-messages",
            "--dangerously-skip-permissions",
            "--exclude-dynamic-system-prompt-sections",
            "--model",
            "opus",
            "--my-custom-provider-flag",
            "42",             // agent.provider_flags
            "--fork-session", // resolveForkSessionArgs
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

        assert!(
            got.iter().any(|a| a == "--my-custom-provider-flag"),
            "provider_flags survive"
        );
        assert_eq!(
            got[got
                .iter()
                .position(|a| a == "--my-custom-provider-flag")
                .unwrap()
                + 1],
            "42",
            "…with its value",
        );
        assert!(
            got.iter().any(|a| a == "--fork-session"),
            "fork-session survives"
        );
        assert!(
            !got.iter().any(|a| a == "--input-format"),
            "while still being healed"
        );
        assert!(got.iter().any(|a| a == "-p"));
    }

    /// An unknown provider has no baseline to diff against, so fall back to
    /// removing only the outright-fatal flag rather than guessing.
    #[test]
    fn an_unknown_provider_falls_back_to_removing_only_the_fatal_flag() {
        let got = container_argv(
            v(&["--input-format", "stream-json", "--custom"]),
            "no-such-provider",
        );
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
            vec![
                ("-p", false),
                ("--output-format", true),
                ("--verbose", false)
            ],
        );
    }

    #[test]
    fn strip_flag_with_value_removes_every_occurrence_and_tolerates_a_missing_value() {
        assert_eq!(
            strip_flag_with_value(v(&["--x", "1", "--keep", "--x", "2"]), "--x"),
            v(&["--keep"]),
        );
        assert_eq!(
            strip_flag_with_value(v(&["--keep", "--x"]), "--x"),
            v(&["--keep"])
        );
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

    // ---- identity M0: split_agent_identity_env ---------------------------
    //
    // Fixture per SPEC_AGENT_IDENTITY_CARRIED_NOT_DERIVED_2026_09_23.md §9.3:
    // two agents whose names collide under `derive_slug` ("AgentY" and
    // "AGENTY" both lowercase to "agenty"), which `agent_def_insert` gave
    // the suffixed slugs `agenty-2` / `agenty-3`. Every assertion below is
    // made against that pair so a change that only works for one agent
    // cannot pass.

    use super::split_agent_identity_env;
    use std::collections::HashMap;

    fn env(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    /// The server launch path (`input.rs` fallback) — `cmd:env` carries
    /// nothing identity-shaped, so both must be filled from what the server
    /// knows. Positive case first (retro §5): prove they are SET before
    /// proving anything about what they are not.
    #[test]
    fn server_path_sets_both_display_and_slug_from_server_knowledge() {
        let mut e = env(&[("AGENTMUX_AGENT_ID", "AgentY")]);
        split_agent_identity_env(&mut e, "AgentY", Some("agenty-3"));
        assert_eq!(e["AGENTMUX_AGENT_DISPLAY"], "AgentY");
        assert_eq!(e["AGENTMUX_AGENT_SLUG"], "agenty-3");
    }

    /// The frontend launch path (`launchAgentDefinition`) already set both
    /// to the same values the server holds — nothing changes, nothing is
    /// duplicated under a different key.
    #[test]
    fn frontend_path_with_agreeing_values_is_left_as_is() {
        let before = env(&[
            ("AGENTMUX_AGENT_ID", "agenty-2"),
            ("AGENTMUX_AGENT_SLUG", "agenty-2"),
            ("AGENTMUX_AGENT_DISPLAY", "AGENTY"),
        ]);
        let mut e = before.clone();
        split_agent_identity_env(&mut e, "AGENTY", Some("agenty-2"));
        assert_eq!(e, before);
    }

    /// §9.4: this phase must not change what `AGENTMUX_AGENT_ID` holds on
    /// either path. Slug-valued on the frontend path, name-valued on the
    /// server path — both survive byte-for-byte.
    #[test]
    fn agentmux_agent_id_is_never_touched_on_either_path() {
        let mut frontend = env(&[("AGENTMUX_AGENT_ID", "agenty-2")]);
        split_agent_identity_env(&mut frontend, "AGENTY", Some("agenty-2"));
        assert_eq!(frontend["AGENTMUX_AGENT_ID"], "agenty-2");

        let mut server = env(&[("AGENTMUX_AGENT_ID", "AgentY")]);
        split_agent_identity_env(&mut server, "AgentY", Some("agenty-3"));
        assert_eq!(server["AGENTMUX_AGENT_ID"], "AgentY");

        let mut none = env(&[]);
        split_agent_identity_env(&mut none, "AgentY", Some("agenty-3"));
        assert!(
            !none.contains_key("AGENTMUX_AGENT_ID"),
            "must not invent one either"
        );
    }

    /// The server's knowledge wins over a `cmd:env` snapshot: a rename after
    /// launch moves `block.meta["agentName"]` but not the frozen env copy,
    /// and the persisted slug is the collision-resolved one, not whatever
    /// the frontend computed before the row existed.
    #[test]
    fn server_values_override_stale_cmd_env_snapshots() {
        let mut e = env(&[
            ("AGENTMUX_AGENT_DISPLAY", "AgentY"),
            ("AGENTMUX_AGENT_SLUG", "agenty"),
        ]);
        split_agent_identity_env(&mut e, "AgentY (renamed)", Some("agenty-3"));
        assert_eq!(e["AGENTMUX_AGENT_DISPLAY"], "AgentY (renamed)");
        assert_eq!(e["AGENTMUX_AGENT_SLUG"], "agenty-3");
    }

    /// A quick-launch pane (`launchAgent`, provider key as `agentId`) has no
    /// `db_agents` row and therefore no slug. The display name is still set;
    /// the slug is NOT derived from it — that derivation is the lossy step
    /// spec §1.1 identifies as the defect. A `cmd:env` slug survives as the
    /// fallback.
    #[test]
    fn no_persisted_row_sets_display_only_and_keeps_any_cmd_env_slug() {
        let mut bare = env(&[]);
        split_agent_identity_env(&mut bare, "AgentY", None);
        assert_eq!(bare["AGENTMUX_AGENT_DISPLAY"], "AgentY");
        assert!(!bare.contains_key("AGENTMUX_AGENT_SLUG"));

        let mut with_fallback = env(&[("AGENTMUX_AGENT_SLUG", "agenty-3")]);
        split_agent_identity_env(&mut with_fallback, "AgentY", None);
        assert_eq!(with_fallback["AGENTMUX_AGENT_SLUG"], "agenty-3");
    }

    /// An empty or whitespace display name (a pane with no `agentName` meta
    /// yet) sets nothing and does not clobber a `cmd:env` value with "".
    #[test]
    fn empty_display_name_sets_nothing_and_preserves_cmd_env() {
        let mut bare = env(&[]);
        split_agent_identity_env(&mut bare, "   ", None);
        assert!(!bare.contains_key("AGENTMUX_AGENT_DISPLAY"));

        let mut with_fallback = env(&[("AGENTMUX_AGENT_DISPLAY", "AgentY")]);
        split_agent_identity_env(&mut with_fallback, "", Some("  "));
        assert_eq!(with_fallback["AGENTMUX_AGENT_DISPLAY"], "AgentY");
        assert!(!with_fallback.contains_key("AGENTMUX_AGENT_SLUG"));
    }

    /// The whole point: after the split, the two colliding agents are
    /// distinguishable by BOTH variables, whereas anything derived from the
    /// display name alone is not.
    #[test]
    fn colliding_names_stay_separable_after_the_split() {
        let mut a = env(&[("AGENTMUX_AGENT_ID", "AgentY")]);
        let mut b = env(&[("AGENTMUX_AGENT_ID", "AGENTY")]);
        split_agent_identity_env(&mut a, "AgentY", Some("agenty-3"));
        split_agent_identity_env(&mut b, "AGENTY", Some("agenty-2"));

        assert_ne!(a["AGENTMUX_AGENT_SLUG"], b["AGENTMUX_AGENT_SLUG"]);
        assert_ne!(a["AGENTMUX_AGENT_DISPLAY"], b["AGENTMUX_AGENT_DISPLAY"]);
        // …and the reason the slug has to come from the store rather than
        // from the name: lowercased, the names are the same string.
        assert_eq!(
            a["AGENTMUX_AGENT_DISPLAY"].to_lowercase(),
            b["AGENTMUX_AGENT_DISPLAY"].to_lowercase(),
        );
    }

    // ---- identity M1: carry_agent_uid_env --------------------------------

    use super::{carry_agent_uid_env, PersistedAgentIdentity};

    fn identity(uid: &str, token: Option<&str>) -> PersistedAgentIdentity {
        PersistedAgentIdentity {
            uid: uid.to_string(),
            slug: Some("agenty-3".to_string()),
            token: token.map(str::to_string),
        }
    }

    /// Positive case first: a known row carries both values into the env.
    #[test]
    fn carry_sets_uid_and_token_from_the_row() {
        let mut e = env(&[("AGENTMUX_AGENT_ID", "AgentY")]);
        carry_agent_uid_env(&mut e, Some(&identity("4f3c-a91", Some("tok-a"))));
        assert_eq!(e["AGENTMUX_AGENT_UID"], "4f3c-a91");
        assert_eq!(e["AGENTMUX_AGENT_TOKEN"], "tok-a");
        assert_eq!(e["AGENTMUX_AGENT_ID"], "AgentY", "§9.4: untouched");
    }

    /// Reserved-variable rule: a persisted `cmd:env` carrying ANOTHER
    /// agent's UID and token (pane reuse, copied block) is overwritten, not
    /// respected. For the token this is the difference between an agent
    /// holding its own credential and holding someone else's.
    #[test]
    fn carry_overwrites_stale_cmd_env_values_unconditionally() {
        let mut e = env(&[
            ("AGENTMUX_AGENT_UID", "9b2e-7d4"),
            ("AGENTMUX_AGENT_TOKEN", "tok-of-the-other-agent"),
        ]);
        carry_agent_uid_env(&mut e, Some(&identity("4f3c-a91", Some("tok-a"))));
        assert_eq!(e["AGENTMUX_AGENT_UID"], "4f3c-a91");
        assert_eq!(e["AGENTMUX_AGENT_TOKEN"], "tok-a");
    }

    /// No row (quick-launch pane, lookup failure): both are REMOVED. Leaving
    /// a stale token in place would be worse than having none, and the
    /// server has nothing truthful to put there.
    #[test]
    fn carry_removes_both_when_the_server_knows_no_identity() {
        let mut e = env(&[
            ("AGENTMUX_AGENT_ID", "AgentY"),
            ("AGENTMUX_AGENT_UID", "stale"),
            ("AGENTMUX_AGENT_TOKEN", "stale"),
        ]);
        carry_agent_uid_env(&mut e, None);
        assert!(!e.contains_key("AGENTMUX_AGENT_UID"));
        assert!(!e.contains_key("AGENTMUX_AGENT_TOKEN"));
        assert_eq!(e["AGENTMUX_AGENT_ID"], "AgentY");
    }

    /// A row whose token could not be minted still carries its UID, and a
    /// stale token is removed rather than kept — never a half-truth.
    #[test]
    fn carry_with_uid_but_no_token_sets_uid_and_removes_token() {
        let mut e = env(&[("AGENTMUX_AGENT_TOKEN", "stale")]);
        carry_agent_uid_env(&mut e, Some(&identity("4f3c-a91", None)));
        assert_eq!(e["AGENTMUX_AGENT_UID"], "4f3c-a91");
        assert!(!e.contains_key("AGENTMUX_AGENT_TOKEN"));
    }

    // ---- identity M0/M1: persisted_agent_identity against a real store ---

    use super::persisted_agent_identity;
    use crate::backend::storage::agents::AgentDefinition;
    use crate::backend::storage::store::Store;

    fn agent_def(id: &str, name: &str) -> AgentDefinition {
        AgentDefinition {
            conversation_visibility:
                crate::backend::storage::agents::default_conversation_visibility(),
            id: id.to_string(),
            // Empty on purpose: `agent_def_insert` derives it from `name`
            // and collision-resolves, exactly as the launch flow does.
            slug: String::new(),
            name: name.to_string(),
            icon: "✦".to_string(),
            provider: "claude".to_string(),
            description: String::new(),
            working_directory: String::new(),
            shell: String::new(),
            provider_flags: String::new(),
            auto_start: 0,
            restart_on_crash: 0,
            idle_timeout_minutes: 0,
            created_at: 0,
            agent_type: "host".to_string(),
            environment: String::new(),
            agent_bus_id: String::new(),
            is_seeded: 0,
            accounts: String::new(),
            parent_id: String::new(),
            branch_label: String::new(),
            updated_at: 0,
            user_hidden: 0,
            container_image: String::new(),
            container_volumes: "[]".to_string(),
            container_name: String::new(),
            use_ambient_login: 0,
            auto_continue_enabled: 0,
            model_vendor_base_url: String::new(),
            memory_id: String::new(),
        }
    }

    fn block_with_agent_id(store: &Store, block_id: &str, agent_id: &str) {
        let mut block = crate::backend::obj::Block {
            oid: block_id.to_string(),
            parentoref: String::new(),
            version: 0,
            runtimeopts: None,
            stickers: None,
            meta: {
                let mut m = crate::backend::obj::MetaMapType::new();
                m.insert("view".to_string(), serde_json::json!("agent"));
                m.insert("agentId".to_string(), serde_json::json!(agent_id));
                m
            },
            subblockids: None,
        };
        store.insert(&mut block).unwrap();
    }

    /// The §1.1 scenario end to end: two agents whose names collide under
    /// `derive_slug` get `agenty` and `agenty-2` from `agent_def_insert`, and
    /// the lookup returns each block's OWN row — its UID, its suffixed slug
    /// (which no derivation from its name could produce), and a token that
    /// is that row's and nobody else's.
    #[test]
    fn persisted_identity_returns_the_rows_uid_slug_and_own_token() {
        let store = Store::open_in_memory().unwrap();
        let mut first = agent_def("uid-first", "AgentY");
        store.agent_def_insert(&mut first).unwrap();
        let mut second = agent_def("uid-second", "AGENTY");
        store.agent_def_insert(&mut second).unwrap();
        assert_eq!(
            first.slug, "agenty",
            "fixture: first insert keeps the bare slug"
        );
        assert_eq!(
            second.slug, "agenty-2",
            "fixture: second insert is suffixed"
        );

        block_with_agent_id(&store, "block-first", "uid-first");
        block_with_agent_id(&store, "block-second", "uid-second");

        let a = persisted_agent_identity(&store, "block-first").expect("first resolves");
        let b = persisted_agent_identity(&store, "block-second").expect("second resolves");
        assert_eq!(a.uid, "uid-first");
        assert_eq!(b.uid, "uid-second");
        assert_eq!(a.slug.as_deref(), Some("agenty"));
        assert_eq!(b.slug.as_deref(), Some("agenty-2"));

        // The token is the persisted one for that UID, minted on this first
        // spawn, and distinct between the two colliding agents.
        let tok_a = a.token.clone().expect("token minted");
        let tok_b = b.token.clone().expect("token minted");
        assert_ne!(tok_a, tok_b);
        assert_eq!(
            store.agent_token_load("uid-first").unwrap().as_deref(),
            Some(tok_a.as_str())
        );
        assert_eq!(
            store.agent_token_load("uid-second").unwrap().as_deref(),
            Some(tok_b.as_str())
        );

        // A second spawn of the same agent carries the SAME token — long-lived
        // for the agent's lifetime (spec §6.3), not re-minted per spawn.
        let a_again = persisted_agent_identity(&store, "block-first").unwrap();
        assert_eq!(a_again.token.as_deref(), Some(tok_a.as_str()));
    }

    /// A quick-launch pane carries a provider key as `agentId` and has no
    /// `db_agents` row; a block that does not exist has nothing either.
    /// Both are "no identity", never an error and never a derived value —
    /// and, load-bearing for the token: no row means no token is minted.
    #[test]
    fn persisted_identity_is_none_for_quick_launch_panes_and_missing_blocks() {
        let store = Store::open_in_memory().unwrap();
        block_with_agent_id(&store, "block-quick", "claude");
        assert_eq!(persisted_agent_identity(&store, "block-quick"), None);
        assert_eq!(persisted_agent_identity(&store, "block-missing"), None);
        assert_eq!(
            store.agent_token_load("claude").unwrap(),
            None,
            "no token minted for a provider key"
        );
    }

    /// Spec §6.3: revoked on deletion. Deleting the agent through either
    /// deletion entry point takes its token with it, so a later spawn on a
    /// reused block cannot resurrect the old credential.
    #[test]
    fn deleting_the_agent_revokes_its_token() {
        let store = Store::open_in_memory().unwrap();
        let mut def = agent_def("uid-del", "Deleted");
        store.agent_def_insert(&mut def).unwrap();
        block_with_agent_id(&store, "block-del", "uid-del");
        let minted = persisted_agent_identity(&store, "block-del").unwrap().token;
        assert!(minted.is_some());

        assert!(store.agent_def_delete("uid-del").unwrap());
        assert_eq!(store.agent_token_load("uid-del").unwrap(), None);
        assert_eq!(persisted_agent_identity(&store, "block-del"), None);

        // …and via the launch-side entry point too.
        let mut def2 = agent_def("uid-del2", "Deleted2");
        store.agent_def_insert(&mut def2).unwrap();
        block_with_agent_id(&store, "block-del2", "uid-del2");
        assert!(persisted_agent_identity(&store, "block-del2")
            .unwrap()
            .token
            .is_some());
        assert!(store.instance_delete("uid-del2").unwrap());
        assert_eq!(store.agent_token_load("uid-del2").unwrap(), None);
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
