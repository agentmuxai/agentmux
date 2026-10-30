// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Whether the actor a request names is the agent that sent it — identity
//! M4a-2 (`docs/specs/SPEC_AGENT_IDENTITY_CARRIED_NOT_DERIVED_2026_09_23.md`
//! §6.5.3, §6.5.6).
//!
//! An actor site is a handler whose request says who is acting: a memory
//! owner, a global-memory writer, a work claimer or creator, an inject or bus
//! sender, a cron creator, a UI-automation signer (§6.5.2 item 4). When the
//! request is [`Caller::Agent`], the named actor is checked against that
//! UID's own row, and one of these is counted per site:
//!
//! - `m4.actor_mismatch.<site>` — the name is none of the row's {slug,
//!   display name, `instance_name`}, matched as the M3 resolver matches
//!   (`Store::agents_matching_name`): slug exactly, the others ASCII
//!   case-insensitively. A name that would not select the caller's row.
//! - `m4.actor_ambiguous.<site>` — the name selects the caller's row, but not
//!   by its slug, and also selects another row — any row, a template
//!   included: the colliding-name case, in which a slug-keyed consumer
//!   (memory) resolves to the *other* agent, and the template-stub case of
//!   #3573.
//! - `m4.actor_absent.<site>` — an attributed request that names no actor.
//! - `m4.actor_unchecked.<site>` — the caller's row could not be read.
//!
//! **Never refused, never awaited, and nothing is written:** the check runs
//! detached on the blocking pool; the handler does not await it, though it
//! contends for the store's connection lock like any store read (§7); M4c
//! dual-writes the UID. This only measures how often the name and the token
//! disagree before anything relies on the token. Unattributed requests are not
//! checked — there is no row to compare with, and most are the UI (§6.5.1).

use crate::backend::rpc_types::RpcContext;
use crate::backend::storage::agents::AgentNameMatch;
use crate::backend::storage::error::StoreError;
use crate::backend::storage::store::Store;

use super::caller::Caller;
use super::AppState;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Outcome {
    Mismatch,
    Ambiguous,
    Absent,
    Unchecked,
}

macro_rules! actor_sites {
    ($($site:ident => $name:literal,)*) => {
        /// Where the actor name came from — the `<site>` in the counters.
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub(crate) enum ActorSite {
            $($site,)*
        }

        impl ActorSite {
            pub(crate) fn counter(self, outcome: Outcome) -> &'static str {
                match (self, outcome) {
                    $(
                        (ActorSite::$site, Outcome::Mismatch) => concat!("m4.actor_mismatch.", $name),
                        (ActorSite::$site, Outcome::Ambiguous) => concat!("m4.actor_ambiguous.", $name),
                        (ActorSite::$site, Outcome::Absent) => concat!("m4.actor_absent.", $name),
                        (ActorSite::$site, Outcome::Unchecked) => concat!("m4.actor_unchecked.", $name),
                    )*
                }
            }
        }
    };
}

actor_sites! {
    MemoryList => "memory_list",
    MemoryRead => "memory_read",
    MemoryWrite => "memory_write",
    MemoryHistory => "memory_history",
    MemoryDiff => "memory_diff",
    MemoryRevert => "memory_revert",
    GlobalMemoryWrite => "globalmemory_write",
    GlobalMemoryRevert => "globalmemory_revert",
    GlobalMemoryRemove => "globalmemory_remove",
    IdentityAccounts => "identity_accounts",
    PresetGet => "preset_get",
    IdentityValidate => "identity_validate",
    IdentitySwitch => "identity_switch",
    SkillManage => "skill_manage",
    HistorySearch => "history_search",
    WorkEnqueue => "work_enqueue",
    WorkClaim => "work_claim",
    WorkHeartbeat => "work_heartbeat",
    WorkComplete => "work_complete",
    WorkRelease => "work_release",
    CronCreate => "cron_create",
    Inject => "inject",
    BusRegister => "bus_register",
    BusRead => "bus_read",
    BusSend => "bus_send",
    BusInject => "bus_inject",
    BusBroadcast => "bus_broadcast",
    UiAuth => "ui_auth",
}

/// Whether `actor` would select `row` by name (see the module doc).
pub(crate) fn names_the_row(row: &AgentNameMatch, actor: &str) -> bool {
    let actor = actor.trim();
    !actor.is_empty()
        && (actor == row.slug
            || actor.eq_ignore_ascii_case(&row.name)
            || actor.eq_ignore_ascii_case(&row.instance_name))
}

/// What to count for `actor` against the caller's `row`; `None` when the
/// name is the caller's own. `rows_named` lists every row the name selects,
/// templates included (`Store::rows_answering_to`); it is asked only when
/// the name selects the caller's row other than by its slug.
pub(crate) fn classify(
    row: &AgentNameMatch,
    actor: &str,
    rows_named: impl FnOnce(&str) -> Result<Vec<AgentNameMatch>, StoreError>,
) -> Option<Outcome> {
    let actor = actor.trim();
    if actor == row.slug {
        return None;
    }
    if !names_the_row(row, actor) {
        return Some(Outcome::Mismatch);
    }
    match rows_named(actor) {
        Ok(rows) if rows.iter().any(|r| r.id != row.id) => Some(Outcome::Ambiguous),
        Ok(_) => None,
        Err(_) => Some(Outcome::Unchecked),
    }
}

/// Identity M4d-4: whether `uid`'s own row has `name` as its slug, exactly
/// (spec §6.5.10 rule ii: never its display or instance name), so a request
/// carrying `uid`'s token under `name` is that agent by its token.
pub(crate) fn uid_has_slug(store: &Store, uid: &str, name: &str) -> bool {
    matches!(store.agent_names_by_id(uid), Ok(Some(row)) if !row.slug.is_empty() && row.slug == name)
}

/// A caller its token names that sent no source (`muxsh agent send` in an
/// agent's shell) is that agent: the source becomes its slug, so tier 4 can
/// relay it and srv can sign it (SPEC_WAN_JEKT_SIGNED_BY_SRV_2026_10_14.md
/// §3.1 rule 2). A source the caller did send is kept, and is signed only if
/// it is the token's own (`wan_signer`).
pub(crate) fn source_from_token(store: &Store, caller_uid: &str, req: &mut crate::backend::reactive::InjectionRequest) {
    if caller_uid.is_empty() || req.source_agent.as_deref().is_some_and(|s| !s.trim().is_empty()) {
        return;
    }
    if let Ok(Some(row)) = store.agent_names_by_id(caller_uid) {
        if !row.slug.is_empty() {
            req.source_agent = Some(row.slug);
        }
    }
}

/// The agent srv may sign a WAN jekt as: its `source_agent`, when the
/// request's own caller (`caller_uid`, from its token or a cron job's
/// creator; never anything a hop carries) owns that name (`uid_owns_name`,
/// §3.1 rules 1 and 2). The name is compared ignoring case, as the keys are
/// stored: an agent sends as `QTest`, its slug is `qtest`.
pub(crate) fn wan_signer<'a>(
    store: &Store,
    caller_uid: &str,
    req: &'a crate::backend::reactive::InjectionRequest,
) -> Option<&'a str> {
    let source = req.source_agent.as_deref()?;
    (!caller_uid.is_empty() && uid_owns_name(store, caller_uid, source)).then_some(source)
}

/// Whether `uid`'s row is the one agent this instance knows by `name`: its
/// slug is `name` ignoring case, and no other row's slug folds to `name`.
/// Every per-name key (host HMAC, LAN, WAN) is stored under the folded name,
/// so a second row differing only in case (`Victim` beside `victim`) would
/// hold the first one's keys; for such a name nobody is its owner.
pub(crate) fn uid_owns_name(store: &Store, uid: &str, name: &str) -> bool {
    let folded = name.trim().to_lowercase();
    if folded.is_empty() {
        return false;
    }
    let owns = matches!(store.agent_names_by_id(uid), Ok(Some(row)) if row.slug.trim().to_lowercase() == folded);
    owns && matches!(store.agent_ids_for_slug_folded(&folded), Ok(ids) if ids.len() == 1 && ids[0] == uid)
}

/// Identity M4d-4: whether a host-tier jekt is from the agent it claims by
/// its token (`audit_source_uid` is the request's Caller). Far-side receivers
/// have no Caller and keep checking the signature, which is still sent.
pub(crate) fn jekt_sender_by_token(store: &Store, req: &crate::backend::reactive::InjectionRequest, claimed: &str) -> bool {
    // Rule ii keeps the slug exact here; `uid_owns_name` adds that no other
    // row folds to it, so a case-only alias can't claim the shared key.
    req.delivery_tier.as_deref() == Some("host")
        && !req.audit_source_uid.is_empty()
        && uid_has_slug(store, &req.audit_source_uid, claimed)
        && uid_owns_name(store, &req.audit_source_uid, claimed)
}

/// Identity M4d-4: the pane of a caller whose token names it under its own
/// slug, from its UID's registration, with no key to check. `None` (today's
/// name path) for anyone else, or before the UID registers.
pub(crate) fn own_pane_by_token(store: &Store, caller: Option<&Caller>, name: &str) -> Option<String> {
    let uid = caller.and_then(Caller::uid)?;
    if !uid_has_slug(store, uid, name) {
        return None;
    }
    crate::backend::reactive::handler::get_global_handler().block_for_uid(uid)
}

/// Count `actor` against the caller's row at `site`. A no-op for an
/// Unattributed caller. Never fails and is never awaited: the store reads
/// run detached on the blocking pool (inline under test, so tests can read
/// the counters right after the request).
pub(crate) fn check_actor(
    state: &AppState,
    caller: Option<&Caller>,
    site: ActorSite,
    actor: Option<&str>,
) {
    let Some(uid) = caller.and_then(Caller::uid) else {
        return;
    };
    let Some(actor) = actor.map(str::trim).filter(|a| !a.is_empty()) else {
        crate::backend::agent_resolve::record_uid_fallback(site.counter(Outcome::Absent));
        return;
    };
    let (mstore, uid, actor) = (state.mstore.clone(), uid.to_string(), actor.to_string());
    let run = move || count_against_row(&mstore, &uid, site, &actor);
    #[cfg(not(test))]
    drop(tokio::task::spawn_blocking(run));
    #[cfg(test)]
    run();
}

/// [`check_actor`] that refuses: a request whose token names an agent may
/// claim only that agent as its sender. `Ok` for an Unattributed caller (no
/// token to compare with: the UI, `muxsh`, a peer srv), for a request that
/// names no actor, and when the caller's row can't be read. The error is the
/// handler's answer.
pub(crate) async fn require_actor(
    state: &AppState,
    caller: Option<&Caller>,
    site: ActorSite,
    actor: Option<&str>,
) -> Result<(), String> {
    check_actor(state, caller, site, actor);
    let (Some(uid), Some(actor)) = (caller.and_then(Caller::uid), actor.map(str::trim).filter(|a| !a.is_empty())) else {
        return Ok(());
    };
    let (mstore, uid, name) = (state.mstore.clone(), uid.to_string(), actor.to_string());
    let own = tokio::task::spawn_blocking(move || names_the_caller(&mstore, &uid, &name)).await.unwrap_or(true);
    if own {
        Ok(())
    } else {
        Err("FORBIDDEN: the sender must be the agent this request's token names".to_string())
    }
}

/// Like [`require_actor`], but for a route that acts for an agent and has no
/// meaning without one: the request must carry the token of the agent it
/// names. A request with no token (or one this srv didn't mint) is refused
/// instead of being taken at its word, and so is one whose token names another
/// agent.
pub(crate) async fn require_token_naming(
    state: &AppState,
    caller: Option<&Caller>,
    site: ActorSite,
    actor: &str,
) -> Result<(), String> {
    check_actor(state, caller, site, Some(actor));
    let Some(uid) = caller.and_then(Caller::uid) else {
        return Err("FORBIDDEN: this acts for an agent, so the request must carry that agent's token (AGENTMUX_AGENT_TOKEN)".to_string());
    };
    let (mstore, uid, name) = (state.mstore.clone(), uid.to_string(), actor.trim().to_string());
    match tokio::task::spawn_blocking(move || proven_slug(&mstore, &uid, &name)).await {
        Ok(Some(_)) => Ok(()),
        _ => Err("FORBIDDEN: the request's token must name the agent this acts for, by a name only that agent answers to".to_string()),
    }
}

/// A WebSocket write to one agent's transcript or session state. A connection
/// that registered as an agent (with its token) may write only its own agent's;
/// any other connection must be the AgentMux window, proven by the host.
/// Holding the instance auth key proves nothing, because every agent holds it.
/// `target` is the agent's definition id or its slug; `what` completes
/// "FORBIDDEN: …" when a connection that isn't the window is refused.
pub(crate) fn require_own_transcript(ctx: &RpcContext, store: &Store, target: &str, what: &str) -> Result<(), String> {
    if ctx.agent_id.is_empty() {
        return ctx.operator_only(&format!("{what} is written by its own agent or by the AgentMux window"));
    }
    let target = target.trim();
    let slug = store.agent_def_get(target).ok().flatten().map_or_else(|| target.to_string(), |def| def.slug);
    if slug.eq_ignore_ascii_case(&ctx.agent_id) {
        Ok(())
    } else {
        Err(format!("FORBIDDEN: {what} is written by its own agent or by the AgentMux window, not by another agent"))
    }
}

/// [`require_own_transcript`] for a call that names a pane's block instead of an agent: the
/// archive or restore of that block's transcript. An agent's connection may act only on the block
/// its own agent is running on (a block with no running agent is the window's alone); any other
/// connection must be the AgentMux window, proven by the host.
pub(crate) fn require_own_block_transcript(ctx: &RpcContext, store: &Store, block_id: &str, what: &str) -> Result<(), String> {
    if ctx.agent_id.is_empty() {
        return ctx.operator_only(&format!("{what} is written by its own agent or by the AgentMux window"));
    }
    // Who runs on a pane comes from the launch record (`last_block_id`), never from the block's `agentId`
    // meta, which any connection can write: `instance_get_by_block_id` only lets that meta choose between
    // rows already recorded for this block.
    let owner = store
        .instance_get_by_block_id(block_id)
        .ok()
        .flatten()
        .and_then(|inst| store.agent_def_get(&inst.id).ok().flatten())
        .map(|def| def.slug);
    match owner {
        Some(slug) if slug.eq_ignore_ascii_case(&ctx.agent_id) => Ok(()),
        _ => Err(format!("FORBIDDEN: {what} is written by its own agent or by the AgentMux window, not by another agent")),
    }
}

/// For a route that records who acted: the agent the request's token names, as
/// `(slug, uid)`. `claimed` is the name the request gives for itself, and an
/// empty one leaves the token to decide alone. Refused without a token, and
/// when `claimed` is not a name only that agent answers to, so what is recorded
/// is what the server proved and never a body field.
pub(crate) async fn require_token_actor(
    state: &AppState,
    caller: Option<&Caller>,
    site: ActorSite,
    claimed: &str,
) -> Result<(String, String), String> {
    check_actor(state, caller, site, Some(claimed));
    let Some(uid) = caller.and_then(Caller::uid) else {
        return Err("FORBIDDEN: this acts for an agent, so the request must carry that agent's token (AGENTMUX_AGENT_TOKEN)".to_string());
    };
    let (mstore, uid, claimed) = (state.mstore.clone(), uid.to_string(), claimed.trim().to_string());
    let who = uid.clone();
    let slug = tokio::task::spawn_blocking(move || {
        if claimed.is_empty() {
            mstore.agent_names_by_id(&who).ok().flatten().map(|row| row.slug)
        } else {
            proven_slug(&mstore, &who, &claimed)
        }
    })
    .await
    .ok()
    .flatten();
    match slug {
        Some(slug) => Ok((slug, uid)),
        None => Err("FORBIDDEN: the request's token must name the agent this acts for, by a name only that agent answers to".to_string()),
    }
}

/// The slug of the agent behind `uid` when `actor` is a name only that agent
/// answers to: its slug, a display or instance name no other agent shares, or
/// a name it was renamed from. A name another row also answers to, or a row
/// that can't be read, proves nothing (unlike [`names_the_caller`], which
/// gives them the benefit of the doubt).
pub(crate) fn proven_slug(store: &Store, uid: &str, actor: &str) -> Option<String> {
    let Ok(Some(row)) = store.agent_names_by_id(uid) else {
        return None;
    };
    match classify(&row, actor, |name| store.rows_answering_to(name)) {
        None => Some(row.slug),
        Some(Outcome::Mismatch) => store.former_name_owner(actor).ok().flatten().filter(|owner| owner.uid == uid).map(|_| row.slug),
        Some(_) => None,
    }
}

/// For a route that acts for the agent its token names (its memory, accounts,
/// presets): the request must carry a token this srv minted. Without one the
/// route has no one to act for, and a body's `agent_id` is only a claim, so it
/// is refused instead of being taken at its word.
pub(crate) fn require_token(state: &AppState, caller: Option<&Caller>, site: ActorSite, actor: &str) -> Result<(), String> {
    check_actor(state, caller, site, Some(actor));
    match caller.and_then(Caller::uid) {
        Some(_) => Ok(()),
        None => Err("FORBIDDEN: this acts for an agent, so the request must carry that agent's token (AGENTMUX_AGENT_TOKEN)".to_string()),
    }
}

/// An agent acts on its own panes only: a request that carries an agent's token
/// is refused when the pane (or agent) it acts on belongs to another. A request
/// with no token (the window, a peer srv) is not compared, nor is a target that
/// has no agent.
pub(crate) fn own_agent_only(caller: Option<&Caller>, target_uid: Option<&str>) -> Result<(), String> {
    match (caller.and_then(Caller::uid), target_uid) {
        (Some(me), Some(target)) if me != target => Err("FORBIDDEN: an agent acts on its own panes only".to_string()),
        _ => Ok(()),
    }
}

/// Whether `actor` is a name the agent behind `uid` answers to: its slug,
/// display or instance name, or one it was renamed from (a running agent keeps
/// sending under the name its environment held at launch). A name that also
/// selects another row, or a row that can't be read, is not held against it.
pub(crate) fn names_the_caller(store: &Store, uid: &str, actor: &str) -> bool {
    let Ok(Some(row)) = store.agent_names_by_id(uid) else {
        return true;
    };
    match classify(&row, actor, |name| store.rows_answering_to(name)) {
        Some(Outcome::Mismatch) => store.former_name_owner(actor).ok().flatten().is_some_and(|owner| owner.uid == uid),
        _ => true,
    }
}

fn count_against_row(store: &Store, uid: &str, site: ActorSite, actor: &str) {
    let outcome = match store.agent_names_by_id(uid) {
        Ok(Some(row)) => classify(&row, actor, |name| store.rows_answering_to(name)),
        Ok(None) | Err(_) => Some(Outcome::Unchecked),
    };
    let Some(outcome) = outcome else {
        return;
    };
    let counter = site.counter(outcome);
    crate::backend::agent_resolve::record_uid_fallback(counter);
    tracing::debug!(
        counter,
        caller_uid = uid,
        actor = %actor.chars().take(128).collect::<String>(),
        "identity M4a-2: the request's actor name is not plainly the calling agent's"
    );
}

/// A work claim also carries the claimer's UID (`agent_uid`, identity M1a).
/// A carried UID that is not the token's is counted on its own, since it is
/// the UID — not the name — that M3 writes to `claimed_by_uid`.
pub(crate) fn check_carried_uid(caller: Option<&Caller>, carried_uid: &str) {
    let Some(uid) = caller.and_then(Caller::uid) else {
        return;
    };
    let carried = carried_uid.trim();
    if !carried.is_empty() && carried != uid {
        crate::backend::agent_resolve::record_uid_fallback("m4.actor_uid_mismatch.work_claim");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SPEC_WAN_JEKT_SIGNED_BY_SRV_2026_10_14.md §3.1: who srv signs as.
    mod wan_signing {
        use super::super::*;
        use crate::backend::reactive::InjectionRequest;

        fn store_with_camper() -> Store {
            let store = Store::open_in_memory().unwrap();
            let mut def = crate::backend::storage::agents::test_agent_def("uid-camper", "Camper", "claude", "agent", 1, "");
            def.slug = "camper".into();
            store.agent_def_insert(&mut def).unwrap();
            store
        }

        fn req(source: Option<&str>, caller_uid: &str) -> InjectionRequest {
            InjectionRequest {
                target_agent: "agent2".into(),
                message: "hi".into(),
                source_agent: source.map(str::to_string),
                audit_source_uid: caller_uid.into(),
                ..Default::default()
            }
        }

        #[test]
        fn a_token_with_no_source_sends_as_its_own_slug() {
            let store = store_with_camper();
            let mut r = req(None, "uid-camper");
            source_from_token(&store, "uid-camper", &mut r);
            assert_eq!(r.source_agent.as_deref(), Some("camper"));
            let mut blank = req(Some("  "), "uid-camper");
            source_from_token(&store, "uid-camper", &mut blank);
            assert_eq!(blank.source_agent.as_deref(), Some("camper"));
            // A source the caller sent is kept, even another agent's.
            let mut other = req(Some("agent5"), "uid-camper");
            source_from_token(&store, "uid-camper", &mut other);
            assert_eq!(other.source_agent.as_deref(), Some("agent5"));
            // No token, nothing filled: a terminal's muxsh stays sourceless.
            let mut anon = req(None, "");
            source_from_token(&store, "", &mut anon);
            assert_eq!(anon.source_agent, None);
            let mut unknown = req(None, "uid-nobody");
            source_from_token(&store, "uid-nobody", &mut unknown);
            assert_eq!(unknown.source_agent, None);
        }

        #[test]
        fn srv_signs_only_as_the_agent_the_token_names() {
            let store = store_with_camper();
            let sign = |source: Option<&str>, caller: &str| wan_signer(&store, caller, &req(source, caller)).map(str::to_string);
            assert_eq!(sign(Some("camper"), "uid-camper").as_deref(), Some("camper"));
            // SendMessage sends AGENTMUX_AGENT_ID, which keeps the name's capitals.
            assert_eq!(sign(Some("Camper"), "uid-camper").as_deref(), Some("Camper"), "the slug, ignoring case");
            assert_eq!(sign(Some("agent5"), "uid-camper"), None, "another agent's name");
            assert_eq!(sign(Some("camper"), ""), None, "no caller: a forwarded hop, a script");
            assert_eq!(sign(Some("camper"), "uid-nobody"), None);
            assert_eq!(sign(None, "uid-camper"), None);
            // A hop that set audit_source_uid (a verified channel v2 hop) is
            // still not this request's caller.
            let mut hop = req(Some("camper"), "uid-camper");
            hop.audit_source_uid = "uid-camper".into();
            assert_eq!(wan_signer(&store, "", &hop), None);
        }

        #[test]
        fn a_name_two_rows_fold_to_has_no_owner() {
            // Rows created before slugs collided case-insensitively: both
            // would hold the same folded keys, so neither is signed for.
            let store = store_with_camper();
            {
                let conn = store.conn().lock().unwrap();
                conn.execute_batch(
                    "INSERT INTO db_agents (id, name, slug, provider, created_at) VALUES ('uid-alias', 'Camper', 'Camper', 'claude', 0);",
                )
                .unwrap();
            }
            assert!(!uid_owns_name(&store, "uid-camper", "camper"));
            assert!(!uid_owns_name(&store, "uid-alias", "Camper"));
            assert_eq!(wan_signer(&store, "uid-alias", &req(Some("Camper"), "uid-alias")), None);
            assert_eq!(wan_signer(&store, "uid-camper", &req(Some("camper"), "uid-camper")), None);
        }

        /// Two rows whose names differ only in case share the folded keys, so
        /// neither gets them in its `.mcp.json`.
        #[test]
        fn inject_jekt_signing_keys_writes_none_when_two_rows_fold_to_the_name() {
            let store = crate::backend::storage::store::Store::open_in_memory().unwrap();
            {
                let conn = store.conn().lock().unwrap();
                conn.execute_batch(
                    "INSERT INTO db_agents (id, name, slug, provider, created_at) VALUES ('a', 'victim', 'victim', 'claude', 0);
                     INSERT INTO db_agents (id, name, slug, provider, created_at) VALUES ('b', 'Victim', 'Victim', 'claude', 0);",
                )
                .unwrap();
            }
            let content = serde_json::to_string(&serde_json::json!({
                "mcpServers": { "agentmux": { "env": { "AGENTMUX_AGENT_ID": "Victim", "AGENTMUX_LAN_KEY": "old" } } }
            }))
            .unwrap();
            let rewritten = crate::backend::agent_config::inject_jekt_signing_keys_into_mcp_json(&content, &store, "Victim").expect("the old key removed");
            let env = serde_json::from_str::<serde_json::Value>(&rewritten).unwrap()["mcpServers"]["agentmux"]["env"].clone();
            assert!(env.get("AGENTMUX_JEKT_KEY").is_none() && env.get("AGENTMUX_LAN_KEY").is_none(), "{env}");
            assert!(store.agent_jekt_key_load("victim").unwrap().is_none(), "nothing minted for the name");
        }

        #[test]
        fn a_new_slug_differing_only_in_case_gets_a_suffix() {
            let store = store_with_camper();
            let mut def = crate::backend::storage::agents::test_agent_def("uid-v", "Camper", "claude", "agent", 1, "");
            def.slug = "Camper".into();
            store.agent_def_insert(&mut def).unwrap();
            assert_eq!(def.slug, "Camper-2", "not a second owner of camper's keys");
            assert!(uid_owns_name(&store, "uid-camper", "Camper"));
        }
    }

    fn row(id: &str, name: &str, slug: &str, instance_name: &str) -> AgentNameMatch {
        AgentNameMatch {
            id: id.into(),
            name: name.into(),
            slug: slug.into(),
            instance_name: instance_name.into(),
        }
    }

    fn only(
        rows: Vec<AgentNameMatch>,
    ) -> impl FnOnce(&str) -> Result<Vec<AgentNameMatch>, StoreError> {
        move |_| Ok(rows)
    }

    #[test]
    fn a_name_the_row_answers_to_matches() {
        let r = row("uid-y", "AgentY", "agenty", "Yardbird");
        assert!(names_the_row(&r, "agenty"));
        assert!(names_the_row(&r, " agenty "));
        assert!(
            names_the_row(&r, "AGENTY"),
            "display name is case-insensitive"
        );
        assert!(names_the_row(&r, "yardbird"));
        assert!(!names_the_row(&r, "agentz"));
        assert!(!names_the_row(&r, ""));
        // Slug is exact, as in the M3 resolver.
        let r = row("uid-y", "Why", "agenty", "");
        assert!(!names_the_row(&r, "AGENTY"));
        assert!(
            !names_the_row(&r, ""),
            "an empty instance name matches nothing"
        );
    }

    #[test]
    fn the_callers_own_slug_is_never_counted_and_asks_nothing() {
        let r = row("uid-y2", "AGENTY", "agenty-2", "");
        assert_eq!(classify(&r, "agenty-2", |_| panic!("not asked")), None);
    }

    #[test]
    fn a_name_that_is_not_the_callers_is_a_mismatch() {
        let r = row("uid-y2", "AGENTY", "agenty-2", "");
        assert_eq!(
            classify(&r, "agentz", |_| panic!("not asked")),
            Some(Outcome::Mismatch)
        );
    }

    /// The colliding-names fixture: the second agent ("AGENTY", agenty-2)
    /// sends the first one's slug `agenty`. It matches the second agent's
    /// display name, so it is no mismatch — but it selects the first agent
    /// too, and a slug-keyed consumer resolves it there (M4a-2 review P1).
    #[test]
    fn a_colliding_name_that_also_selects_another_row_is_ambiguous() {
        let first = row("uid-y", "AgentY", "agenty", "");
        let second = row("uid-y2", "AGENTY", "agenty-2", "");
        assert_eq!(
            classify(&second, "agenty", only(vec![first.clone(), second.clone()])),
            Some(Outcome::Ambiguous)
        );
        // "AGENTY" selects both rows by display name, so it is ambiguous too.
        assert_eq!(
            classify(&second, "AGENTY", only(vec![first.clone(), second.clone()])),
            Some(Outcome::Ambiguous)
        );
        // A display name no other row holds is the caller's own.
        let zed = row("uid-z", "Zed", "zed-2", "");
        assert_eq!(classify(&zed, "zed", only(vec![zed.clone()])), None);
        assert_eq!(
            classify(&second, "agenty", |_| Err(StoreError::NotFound)),
            Some(Outcome::Unchecked)
        );
    }
}
