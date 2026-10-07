// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! App API — high-level commands for programmatic control of AgentMux.
//!
//! These commands orchestrate multiple low-level operations (CreateBlock, SetMeta,
//! ControllerResync) behind stable, intent-based interfaces. Callers express what
//! they want ("open an agent pane with AgentX"), not how to do it.

use std::sync::Arc;

use base64::Engine;
use serde_json::json;

use crate::backend::blockcontroller;
use crate::backend::obj::{self, Block, Tab, Workspace, MetaMapType};
use crate::backend::providers;
use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::rpc_types::*;
use crate::backend::session_archive;
use crate::backend::storage::store::{Store, AgentContent, AgentDefinition, AgentInstance};
use crate::backend::storage::identities::IdentityAccount;
use crate::backend::storage::bundles::Bundle;

use super::AppState;
use crate::server::cli_handlers::resolve_provider_cli_on_path;

mod agent_open;
/// Re-exported for `server/mod.rs`'s `POST /api/v1/agent/open` handler (the
/// `OpenAgent` MCP tool's backing route) — the HTTP surface must share the
/// RPC path's exact implementation, including its AGENT_OPEN_LOCKS TOCTOU
/// serialization. See REPORT_AGENT_OPEN_API_GAP_2026_09_06.md.
pub(crate) use agent_open::open_agent_impl;
// pub(crate): `server/mod.rs`'s `/agentmux/agent/stop` handler (the
// cross-channel bulk-stop forward target,
// SPEC_FLEET_BULK_STOP_CROSS_CHANNEL_2026_08_22.md) calls
// `stop_one_agent_block` directly — same function `fleet_bulk_stop_impl`
// already uses for a LOCAL target, reused rather than duplicated for a
// forwarded one.
pub(crate) mod agent_io;
mod agent_define;
/// Re-exported so the human-facing creation/edit RPC handlers
/// (`server::agent_handlers::template`/`core`) can reuse the same
/// vendor-base-url validation `agent.define` already uses, instead of
/// duplicating it.
pub(crate) use agent_define::{check_provider_unchanged, validate_vendor_base_url};
// pub(crate): `server/mod.rs`'s `POST /api/v1/agent/pane/close` handler (the
// `ClosePane` MCP tool's backing route) calls `pane::handle_close_pane`
// directly, same as `fleet`'s exposure just below.
pub(crate) mod pane;
// pub(crate): `server/mod.rs`'s `POST /api/v1/agent/dev_server/register`
// handler (the `RegisterDevServer` MCP tool's backing route) calls
// `dev_server::handle_register_dev_server` directly, same pattern as
// `pane`'s `handle_close_pane` just above.
pub(crate) mod dev_server;
// pub(crate): the viewer feed reads transcripts through `read_range`.
pub(crate) mod blockfile;
pub(crate) mod session;
mod identity;
pub(crate) mod bundle;
mod memory;
mod global_memory;
// The App API bodies live with the submodule that registers them; these
// re-exports keep every `app_api::…` path unchanged.
pub use pane::{open_pane};
pub(crate) use pane::{move_tab};
pub use agent_define::{allocate_agent_workdir};
pub(crate) use agent_define::{agent_define_core};
pub(crate) use identity::{identity_self_accounts_impl, identity_account_validate_stored_impl};
pub(crate) use bundle::{bundle_list_impl, bundle_get_impl, bundle_validate_impl, bundle_self_get_impl};
pub(crate) use bundle::SelfOwner;
pub(crate) use global_memory::GlobalMemoryWriteProvenance;
pub(crate) use memory::MemoryWriteProvenance;
pub(crate) use memory::{memory_list_impl, memory_read_impl, memory_write_impl, memory_history_impl, memory_diff_impl, memory_revert_impl};
pub(crate) use global_memory::{global_memory_write_impl, global_memory_list_impl, global_memory_read_impl, global_memory_remove_impl, global_memory_history_impl, global_memory_diff_impl, bundle_version_diff, global_memory_revert_impl};
mod skill;
mod mcp;
mod bookmarks;
mod layout;
mod browser_start_page;
mod voice;
pub(crate) mod fleet;
mod attachments;
pub(crate) mod connections;
pub(crate) mod viewer;
mod presence;

/// Register all App API handlers on the RPC engine.
pub fn register_app_api_handlers(engine: &Arc<WshRpcEngine>, state: &AppState) {
    agent_open::register(engine, state);
    agent_io::register(engine, state);
    agent_define::register(engine, state);
    pane::register(engine, state);
    blockfile::register(engine, state);
    session::register(engine, state);
    identity::register(engine, state);
    bundle::register(engine, state);
    memory::register(engine, state);
    skill::register(engine, state);
    mcp::register(engine, state);
    bookmarks::register(engine, state);
    layout::register(engine, state);
    browser_start_page::register(engine, state);
    voice::register(engine, state);
    fleet::register(engine, state);
    attachments::register(engine, state);
    connections::register(engine, state);
    viewer::register(engine, state);
    presence::register(engine, state);
}

#[cfg(test)]
#[path = "tests/global_memory_impl_tests.rs"]
mod global_memory_impl_tests;
#[cfg(test)]
#[path = "tests/global_memory_version_impl_tests.rs"]
mod global_memory_version_impl_tests;
#[cfg(test)]
#[path = "tests/memory_version_impl_tests.rs"]
mod memory_version_impl_tests;

// ---------------------------------------------------------------------------
// Shared helpers used by submodules (via `use super::*`)
// ---------------------------------------------------------------------------
/// If the agent's GLOBAL transcript zone (`agent:<defId>:current`) has
/// `output`, return `(global_store, agent_zone)` — preferred even when this
/// channel's local `output` is non-empty. Returns `None` when the block isn't
/// agent-anchored or is archived, there's no global store, or the global zone
/// is empty — callers then read the per-channel store keyed by `block_id`.
///
/// Only the agent `output` stream is globalized; every other file stays local.
pub(super) fn global_output_source(
    _per_channel: &Arc<crate::backend::storage::filestore::FileStore>,
    global: &Option<Arc<crate::backend::storage::filestore::FileStore>>,
    mstore: &Arc<crate::backend::storage::store::Store>,
    block_id: &str,
    filename: &str,
) -> Option<(Arc<crate::backend::storage::filestore::FileStore>, String)> {
    if filename != crate::backend::agent_session::OUTPUT_FILE {
        return None;
    }
    let gfs = global.as_ref()?;
    let block = mstore.get::<Block>(block_id).ok().flatten()?;
    let archived = block
        .meta
        .get(crate::backend::session_archive::META_SESSION_ARCHIVED_AT)
        .and_then(|v| v.as_i64())
        .map(|v| v > 0)
        .unwrap_or(false);
    if archived {
        return None;
    }
    let zone = crate::backend::agent_session::agent_zone_for_block_meta(&block.meta)?;
    match gfs.stat(&zone, filename) {
        Ok(Some(ref wf)) if wf.size > 0 => Some((gfs.clone(), zone)),
        _ => None,
    }
}

/// Exact non-blank line count of the `output` file in `zone`, computed via the
/// same streaming index builder `read_range` uses (so the two endpoints always
/// agree). Returns `Some(0)` for an empty file, `None` on read failure.
///
/// Reuses `output.idx` when its `covered_size` header already matches the
/// current `output` size — same freshness check
/// `register_blockfile_read_range` (`blockfile.rs`) already does — instead of
/// unconditionally rescanning the entire history on every call. Without this,
/// every pane open for an agent with any global-zone history pays a full
/// O(history size) rescan even when nothing has changed since the index was
/// last built.
pub(super) fn global_zone_line_count(
    gfs: &Arc<crate::backend::storage::filestore::FileStore>,
    zone: &str,
) -> Option<u64> {
    use crate::backend::blockcontroller::shell::{extend_output_idx, output_index};

    // `output` and its index from one database snapshot: sizes, header and
    // generation label of the same moment, whatever another srv instance is
    // writing to this shared zone (Codex on #3634).
    let view = output_index(gfs, zone)?;
    if view.output_size == 0 {
        return Some(0);
    }
    if let Some(lines) = view.fresh_lines {
        return Some(lines);
    }

    // Stale or missing. `extend_output_idx` scans only the appended bytes when
    // it can anchor on the existing index, and falls back to a full rebuild
    // when it can't (missing, unreadable, no entries, another generation's, or
    // `output` shrank).
    //
    // The count must stay EXACT. Returning a stale one instead was tried and
    // is wrong: this feeds `useHistoryPagination`'s tail window
    // (`offset = total - PAGE_SIZE`), so an undercount silently drops the most
    // recent history on reopen — codex P1 on PR #2838. Only the COST of
    // keeping it exact was ever negotiable, which is what the incremental scan
    // buys: `output` grows continuously for a live agent, so "stale" is the
    // steady state, and a 30-second line-count poll was driving a full
    // O(file-size) rescan every time — 37 rebuilds in 19 minutes on a 759 MB
    // transcript, mean 3168 ms, ~10% permanent duty cycle. See
    // docs/reports/REPORT_AGENT_PANE_LOAD_RENDER_ARCHITECTURE_2026_08_27.md §5.
    extend_output_idx(gfs, zone)
}

/// Resolve a tab ID: use the provided one, or fall back to the first workspace's active tab.
pub(super) fn resolve_tab_id(mstore: &Store, explicit: Option<&str>) -> Result<String, String> {
    if let Some(tid) = explicit {
        return Ok(tid.to_string());
    }

    // Fall back to first workspace's active tab
    let workspaces: Vec<Workspace> = mstore.get_all::<Workspace>()
        .map_err(|e| format!("agent.open: list workspaces: {e}"))?;

    for ws in &workspaces {
        if !ws.activetabid.is_empty() {
            return Ok(ws.activetabid.clone());
        }
        if let Some(first_tab) = ws.tabids.first() {
            return Ok(first_tab.clone());
        }
    }

    Err("no tabs found in any workspace".to_string())
}

/// Find an existing agent block in a tab by agent ID.
pub(super) fn find_agent_block(mstore: &Store, tab_id: &str, agent_id: &str) -> Result<Option<Block>, String> {
    let tab: Tab = mstore.must_get(tab_id)
        .map_err(|e| format!("TAB_NOT_FOUND: {e}"))?;

    for block_id in &tab.blockids {
        if let Ok(Some(block)) = mstore.get::<Block>(block_id) {
            let block_agent_id = obj::meta_get_string(&block.meta, "agentId", "");
            if block_agent_id == agent_id {
                return Ok(Some(block));
            }
        }
    }
    Ok(None)
}

/// Find which tab a given block currently belongs to, by scanning every
/// tab's `blockids`. Needed because `resolve_tab_id` only resolves an
/// explicit tab_id or "the active tab" — neither answers "which tab is MY
/// OWN block in," which a caller passing its own block id (e.g. an MCP
/// tool's `split_reference_block_id`) actually needs.
pub(super) fn resolve_tab_id_for_block(mstore: &Store, block_id: &str) -> Result<String, String> {
    let tabs: Vec<Tab> = mstore.get_all::<Tab>()
        .map_err(|e| format!("resolve_tab_id_for_block: list tabs: {e}"))?;
    for tab in tabs {
        if tab.blockids.iter().any(|b| b == block_id) {
            return Ok(tab.oid);
        }
    }
    Err(format!("resolve_tab_id_for_block: block {block_id} not found in any tab"))
}

/// Find an existing Editor-view block in a tab on `connection` (its meta
/// `connection`; absent and `local` are this computer), if any. Direct
/// sibling of `find_agent_block` above, checking `meta.view == "editor"`
/// instead of `meta.agentId`. An editor on another connection is never it:
/// its reads and saves go there (remote terminals spec §6.3).
pub(super) fn find_editor_block(mstore: &Store, tab_id: &str, connection: &str) -> Result<Option<Block>, String> {
    let tab: Tab = mstore.must_get(tab_id)
        .map_err(|e| format!("TAB_NOT_FOUND: {e}"))?;

    for block_id in &tab.blockids {
        if let Ok(Some(block)) = mstore.get::<Block>(block_id) {
            if obj::meta_get_string(&block.meta, "view", "") == "editor"
                && same_pane_connection(&obj::meta_get_string(&block.meta, "connection", ""), connection)
            {
                return Ok(Some(block));
            }
        }
    }
    Ok(None)
}

/// Whether two panes' connections are one: absent and `local` are this
/// computer; otherwise the same host however it is spelled.
pub(super) fn same_pane_connection(a: &str, b: &str) -> bool {
    let norm = |c: &str| {
        let c = c.trim();
        if c.is_empty() || c == "local" { String::new() } else { c.to_string() }
    };
    let (a, b) = (norm(a), norm(b));
    if a.is_empty() || b.is_empty() {
        return a == b;
    }
    crate::backend::remote::conn::same_connection(&a, &b)
}

// ---------------------------------------------------------------------------
// S1 enforcement helper
// ---------------------------------------------------------------------------

/// S1: the caller may only act on itself.
///
/// `ctx.agent_id` is stamped at `bus:register` and is the agent's slug;
/// `req_agent_id` is whatever the request named. Both are compared **as
/// canonical `db_agents.id`s** when they are not already byte-identical, so a
/// caller that authenticates with its slug and names itself by id (or by a
/// differently-cased slug) is recognised as itself.
///
/// **Both sides resolve, or neither does.** Phase 4 of
/// `SPEC_CANONICAL_AGENT_ID_MIGRATION_2026_09_21.md` §6 proposed stamping the
/// resolved id into `RpcContext` at registration instead, and flags the hazard
/// itself: resolving one side of this equality while the other stays a slug
/// silently denies **every** caller, turning an authz boundary into a
/// permanent outage that no test of the allow path would catch. Resolving both
/// here, at the single point of comparison, has no such window — there is no
/// deploy ordering in which one side has migrated and the other has not.
///
/// The byte-equality fast path is deliberate and load-bearing: it is the
/// overwhelmingly common case, it costs no store lookup, and it makes this
/// function's behaviour for existing callers *identical* to the pre-migration
/// version. Resolution only ever runs where the old code would already have
/// returned `FORBIDDEN`, so this can admit calls that used to be rejected but
/// can never reject one that used to be admitted.
///
/// Admitting more is safe here precisely because resolution is per-input and
/// fails closed: two different inputs resolve to one id only when they name
/// one agent, and an ambiguous slug (two agents, one slug) resolves to
/// `Err` on both sides — so a collision denies rather than granting.
///
/// A failed resolution is reported as `agent_id mismatch`, never "no such
/// agent": whether some other agent exists is not something an unauthorised
/// caller should be able to probe through this boundary.
/// Takes the store rather than `&AppState` because the handlers do not agree
/// on what they capture — some clone the whole state into the closure, others
/// clone `mstore` alone — and a borrowed `&AppState` cannot escape into a
/// `'static` future.
pub(super) fn check_s1(
    store: &crate::backend::storage::store::Store,
    ctx: &RpcContext,
    req_agent_id: &str,
) -> Result<(), String> {
    check_s1_resolved(store, &ctx.agent_id, req_agent_id)
}

/// `check_s1`'s decision over plain strings, so it can be tested without
/// constructing an `RpcContext`.
fn check_s1_resolved(
    store: &crate::backend::storage::store::Store,
    ctx_agent_id: &str,
    req_agent_id: &str,
) -> Result<(), String> {
    if ctx_agent_id.is_empty() {
        return Err("FORBIDDEN: unauthenticated agent connection".to_string());
    }
    if ctx_agent_id == req_agent_id {
        return Ok(());
    }
    let mismatch = || "FORBIDDEN: agent_id mismatch".to_string();
    let caller = crate::backend::agent_resolve::resolve_agent_id(store, ctx_agent_id)
        .map_err(|_| mismatch())?;
    let target =
        crate::backend::agent_resolve::resolve_agent_id(store, req_agent_id).map_err(|_| mismatch())?;
    if caller != target {
        return Err(mismatch());
    }
    Ok(())
}

/// Resolve an S1-authenticated agent id (the slug — AGENTMUX_AGENT_ID /
/// bus:register id, e.g. "Agent3") to the agent's DEFINITION id, which is
/// what `db_agent_identity_links.agent_id` stores (== `AgentDefinition.id`;
/// see m0013 and `identity/resolver/inject.rs::resolve_bindings_for_instance`).
///
/// Every link-table operation reached from the App API must go through
/// this: App API callers authenticate with the slug, but writing the slug
/// into the link table either trips the per-channel schema's
/// `FOREIGN KEY (agent_id) REFERENCES db_agents(id)` (v30, #3088 — was
/// `db_agent_definitions(id)`; a definition id still always satisfies it,
/// since every definition has a `db_agents` mirror at the same id) (loud
/// "FOREIGN KEY constraint failed" — the id_store fallback path when the
/// 0011 shared-store backfill hasn't applied), or — on the shared store,
/// whose links table carries no agent_id FK — silently writes a row keyed
/// by slug that the resolver (which reads by definition id) can never
/// match. Reads have the mirror-image bug: listing links by slug always
/// returns empty, so ownership checks reject and unlinks no-op.
///
/// A caller that already holds a definition id passes through unchanged
/// (verified against `agent_def_get`), so internal non-S1 callers of the
/// shared `*_impl` helpers stay valid.
pub(super) fn resolve_agent_definition_id(
    state: &AppState,
    agent_id: &str,
) -> Result<String, String> {
    // The three tiers this used to spell out inline (db_agents by slug, then
    // by id, then the registry) are now `agent_resolve`'s, shared with
    // `resolve_agent_uuid`. The registry tier in particular is load-bearing and
    // was won the hard way — reagentx P1 on PR #2428 (round 4): the common case
    // is a live agent with no `db_agents` row at all (launching an agent does
    // not create one), so `identity.self.accounts`/`IdentityAccounts` stayed
    // live-broken for registry-only agents even after `instance_get_by_slug`
    // existed. See `agent_resolve::resolve_agent_id`.
    crate::backend::agent_resolve::resolve_agent_id(&state.mstore, agent_id)
}

// Current unix time in milliseconds (0 if the clock is before the epoch).
pub(super) use agentmux_common::time::now_ms;

/// Parse + validate a saved per-agent `ui:zoom` content blob for seeding a new
/// agent block's `term:zoom`. Returns `Some(z)` only for a parseable,
/// non-default (≠ 1.0), in-[0.5, 2.0] zoom (the range the frontend enforces in
/// term.tsx); anything else (default, out of range, garbage) returns `None` so
/// the new block opens at the default 1.0. See SPEC_AGENT_ZOOM_PERSISTENCE §4.2.
pub(super) fn parse_seed_zoom(raw: &str) -> Option<f64> {
    let z = raw.trim().parse::<f64>().ok()?;
    if (z - 1.0).abs() > f64::EPSILON && (0.5..=2.0).contains(&z) {
        Some(z)
    } else {
        None
    }
}

#[cfg(test)]
#[path = "tests/cross_channel_tests.rs"]
mod cross_channel_tests;


#[cfg(test)]
#[path = "tests/pane_open_reducer_tests.rs"]
mod pane_open_reducer_tests;

#[cfg(test)]
#[path = "tests/agent_zoom_seed_tests.rs"]
mod agent_zoom_seed_tests;

#[cfg(test)]
#[path = "tests/bundle_upsert_input_tests.rs"]
mod bundle_upsert_input_tests;

// SPEC_AGENT_PANE_HISTORY_ALIGNMENT_2026_08_05.md follow-up: `PresetGet`
// self-mode (backed by `bundle_self_get_impl`) only ever checked the local
// `db_agents` table via `instance_get_by_slug`. A live agent that only
// exists in the global named-agent registry (the common case — launching
// an agent does not create a `db_agents` row, see issue #1836, already
// handled the same way by `native_memory_handlers::memory_dir_for_agent`)
// fell through to `instance: None` and silently returned the generic
// blank/vanilla preset, with nothing to distinguish it from an agent that
// genuinely has no bundle bound. Confirmed live against a real registry-only
// agent named "AgentY": `PresetGet` returned `is_blank: true` even though
// the registry's own `memory_id` was set.
#[cfg(test)]
#[path = "tests/bundle_self_get_registry_fallback_tests.rs"]
mod bundle_self_get_registry_fallback_tests;

#[cfg(test)]
#[path = "tests/identity_self_accounts_tests.rs"]
mod identity_self_accounts_tests;

#[cfg(test)]
#[path = "tests/s1_tests.rs"]
mod s1_tests;

#[cfg(test)]
#[path = "tests/self_owner_tests.rs"]
mod self_owner_tests;
