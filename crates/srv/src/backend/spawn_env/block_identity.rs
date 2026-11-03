// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What the spawn environment carries about the agent on a block: the display name and slug, the uid, the workdir.


/// Identity M0 — `SPEC_AGENT_IDENTITY_CARRIED_NOT_DERIVED_2026_09_23.md`
/// §9.1: make the display name and the slug two explicit variables, present
/// on every launch path.
///
/// **Why.** `AGENTMUX_AGENT_ID` means two different things depending on who
/// spawned the agent (spec §0.2): the frontend's `launchAgentDefinition`
/// writes the definition *slug* into it (`agent-model.ts`), while this
/// server path's fallback wrote the *display name*
/// (`block.meta["agentName"]`) until `carry_agent_id_env` (2026-10-12), which
/// writes the slug too when the row has one. Nothing downstream could tell
/// which it got. The frontend path already sets
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
/// here: `identity_env` sets it, from the same slug when there is one; the
/// git identity, `MUXBUS_AGENT_ID`, the PR-body tag and the OSC prompt hook
/// all still derive from it.
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

/// `AGENTMUX_AGENT_WORKDIR`: the agent's own workspace, which the host-only
/// "Your workspace" Operator Config entry tells it to clone and work in
/// (SPEC_LAUNCH_CONTEXT_WORKSPACE_RULE_AND_STARTUP_FILES_2026_09_30.md §3.4).
/// Server-set, like `AGENTMUX_AUTH_KEY`: it is the launch cwd (`cmd:cwd`),
/// not user configuration, so a `cmd:env` copy never wins. Host agents only —
/// a container's cwd is a host path that means nothing inside the image (the
/// key is host-only in `agent_env_scope`).
///
/// Exported as the absolute path the process actually starts in: `~` is
/// expanded and a relative `cmd:cwd` is resolved against srv's own cwd, as
/// `core::apply_working_dir` does, so `cd "$AGENTMUX_AGENT_WORKDIR"` works
/// from anywhere (Codex on #4113).
pub(crate) fn carry_agent_workdir_env(
    env_vars: &mut std::collections::HashMap<String, String>,
    block_meta: &crate::backend::obj::MetaMapType,
) {
    const KEY: &str = "AGENTMUX_AGENT_WORKDIR";
    let agent_mode = crate::backend::obj::meta_get_string(block_meta, "agentMode", "host");
    let work_dir = crate::backend::obj::meta_get_string(block_meta, "cmd:cwd", "");
    if crate::backend::operator_config_seed::agent_kind(&agent_mode) == "host" && !work_dir.trim().is_empty() {
        let expanded = crate::backend::blockcontroller::core::expand_home_dir(&work_dir);
        let absolute = std::path::absolute(&expanded).map_or(expanded, |p| p.to_string_lossy().into_owned());
        env_vars.insert(KEY.to_string(), absolute);
    } else {
        env_vars.remove(KEY);
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
    /// The names it had before its renames (`former_names_env`).
    pub former: super::former_names::FormerNames,
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
pub(crate) fn persisted_agent_identity(
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
    // Identity M4d-2: the UID's own signing keys, copied from its name's on
    // ownership evidence or minted. Only token-carrying spawns get them (the
    // ones that will sign v2, M4d-6); nothing reads them yet. Like the token,
    // a store error is logged, never a spawn failure.
    if token.is_some() {
        if let Err(e) = mstore.agent_uid_keys_ensure(&instance.id) {
            tracing::warn!(block_id, uid = %instance.id, error = %e, "spawn env: UID key ensure failed");
        }
    }
    let former = super::former_names::read(mstore, block_id, &instance.id);
    Some(PersistedAgentIdentity { uid: instance.id, slug, token, former })
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
    super::former_names::carry(env_vars, identity.map(|i| &i.former));
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

/// #3577: the id of the deleted agent `block_id` names, when the block
/// resolved to no row (`identity` is `None`) and its agent was deleted —
/// the builder's last-read re-check of the gate's refusal (Codex P1 on
/// #3591). `None` whenever a row resolved, or the block names nothing
/// deleted.
pub(crate) fn deleted_agent_of_rowless_block(
    mstore: &crate::backend::storage::store::Store,
    block_id: &str,
    identity: Option<&PersistedAgentIdentity>,
) -> Option<String> {
    if identity.is_some() {
        return None;
    }
    crate::identity::resolver::block_agent_id(mstore, block_id)
        .filter(|agent_id| mstore.agent_was_deleted(agent_id))
}

/// Identity M4b-2 (spec §6.5.8): carry `block_id`'s row UID and token, and
/// its slug's identity, into the env of a controller that builds its own (App Server, ACP) instead of
/// going through `build_persistent_spawn_env`. Same resolution and the
/// same reserved-value rules as that builder (stale values are overwritten,
/// or removed when the block has no row). A synchronous store read and an
/// idempotent token mint, so on a multi-thread runtime worker it runs under
/// `block_in_place`, as eager resume does (incident #1782).
pub(crate) fn carry_block_identity_env(
    mstore: &crate::backend::storage::store::Store,
    block_id: &str,
    env_vars: &mut std::collections::HashMap<String, String>,
) {
    let lookup = || persisted_agent_identity(mstore, block_id);
    let identity = match tokio::runtime::Handle::try_current() {
        Ok(handle) if handle.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread => {
            tokio::task::block_in_place(lookup)
        }
        _ => lookup(),
    };
    carry_agent_uid_env(env_vars, identity.as_ref());
    // And the row slug's identity over `cmd:env`'s, as on that builder.
    if let Some(slug) = identity.as_ref().and_then(|i| i.slug.as_deref()) {
        super::identity::carry_slug_identity_env(env_vars, slug);
    }
}
