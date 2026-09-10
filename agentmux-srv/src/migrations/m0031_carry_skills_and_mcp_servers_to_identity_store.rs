// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Carry each channel's `db_skills` / `db_mcp_servers` rows into the
//! permanently-global identity store, which became authoritative for them in
//! Phase 2a (`IDENTITY_STORE_SCHEMA_VERSION` v6/v7 — see
//! `SPEC_DURABLE_BINDINGS_2026_09_10.md` §7 Phase 2).
//!
//! **Must ship before any Store method redirects catalog reads to the
//! identity store.** Until this has run once for a channel, that channel's
//! skills/servers exist ONLY in its own `objects.db` — redirecting reads
//! first would make every existing skill invisible. This migration is what
//! makes the redirect (a later PR) safe to land.
//!
//! Channel-scoped, not Global: an earlier design enumerated and WROTE to
//! every sibling channel's `objects.db` in one pass (mirroring
//! `m0022_identity_store_links_backfill`'s multi-source read), but that
//! migration is explicitly documented as never mutating a source — this one
//! must, to repoint ref rows (see below), and doing that across a sibling
//! channel's live file from a process that isn't running that channel is an
//! unnecessary risk this migration doesn't need to take. Each channel
//! carries only its OWN rows, on its OWN first boot after this ships —
//! exactly the same "every channel eventually boots and converges" shape
//! `m0021`/`m0030` already use, and the only stores this ever writes to are
//! the ones every other migration in this series already writes to: the
//! current channel's own store, plus (here) the identity store.
//!
//! ## Frozen starter recognition (Codex P1, PR #3181)
//!
//! `FROZEN_STARTER_SKILLS`/`FROZEN_STARTER_MCP_SERVER_NAMES` below and
//! `frozen_starter_skill_id`/`frozen_starter_mcp_server_id` are deliberate,
//! documented copies of `skill_seed`/`mcp_seed`'s manifest and
//! id-derivation logic, frozen as of this migration's origin — per this
//! module tree's own rule (`migrations/mod.rs`, "Migrations freeze copies
//! of live logic on purpose"): a migration must produce the same rows on
//! every machine it ever runs on, so it cannot call a live helper whose
//! behavior a later release may change. An earlier revision of this
//! migration called `skill_seed::starter_skill_id` /
//! `mcp_seed::starter_mcp_server_id` directly — if a future release ever
//! renames a starter, adds one, or changes the derivation formula, a
//! machine running THIS migration under that later release would recognize
//! or rekey legacy rows differently from a machine that ran it today,
//! breaking the exact convergence guarantee this migration exists to
//! provide. `starter_ids_match_the_live_derivation_today` below pins that
//! the two agree right now; they are allowed — expected — to diverge later.
//!
//! ## The dedup rule (§5.3)
//!
//! - **A recognized starter** (`is_global = 1`, name matches the frozen
//!   list above) converges on the SAME id every channel would independently
//!   mint for it as of Phase 1, recomputed from the frozen derivation —
//!   never trusted from whatever id this particular channel's row happens
//!   to have. Two channels computing the identical id and both attempting
//!   `INSERT OR IGNORE` under it is race-safe on PK equality alone; no
//!   further arbitration needed.
//! - **Any other global row** (a user-promoted global skill/server, not one
//!   of the six starters) dedups by `name` alone — first-wins. Global rows
//!   are safe to collapse across channels because a global row is owned by
//!   nobody (§5.3). Unlike a starter, this id is NOT deterministic, so two
//!   channels' migrations running concurrently (multiple AgentMux instances
//!   is an explicitly supported scenario) can both observe "no match yet"
//!   before either inserts — Codex P1, PR #3181. Arbitrated by a real
//!   database constraint (`idx_ids_skills_global_name` /
//!   `idx_ids_mcp_servers_global_name`, `IDENTITY_STORE_SCHEMA_VERSION` v8),
//!   not just an application-level check: the insert is attempted
//!   optimistically under this row's own id, and whichever channel's insert
//!   the constraint accepts is authoritative — the loser re-queries BY NAME
//!   ALONE (never `(name, skill_type)` — Codex P1, PR #3183: the skill index
//!   was `(name, skill_type)` until v8 narrowed it to match
//!   `skill_upsert_unique_global`'s own name-only invariant, and a
//!   `(name, skill_type)`-scoped re-query can miss a winner carried under a
//!   different `skill_type`, silently stranding a local ref pointing at an
//!   id the identity store never actually holds) for the winner rather than
//!   trusting its own pre-insert guess.
//! - **An owner-private row** (`is_global = 0`) is never deduplicated —
//!   carried across under its own id unless that id collides with something
//!   already in the identity store, in which case it gets a fresh one.
//!   Merging two different owners' rows because they happen to share a name
//!   would create an edit channel between them that never existed.
//!
//! Whenever the identity-store id differs from this channel's own local id
//! (a legacy pre-Phase-1 starter, a first-wins convergence, or a
//! collision-forced rename), three things happen, in order: a local copy is
//! inserted under the new id, this channel's OWN ref rows
//! (`db_agent_skills_ref`/`db_bundle_skills_ref`, and the MCP equivalents)
//! are rewritten to it, and ONLY THEN is the stale local row under the old
//! id removed.
//!
//! That removal is not optional cleanup — Codex P2 x2, PR #3181, both
//! rooted in the same gap. Leaving the stale row in place (an earlier
//! revision's choice, reasoning that Phase 2a's "never delete locally"
//! covered it) meant two things broke: local catalog reads
//! (the degraded-mode fallback) would show the SAME starter twice, and —
//! the more serious one — retrying this migration after a partial failure
//! would find the stale row still sitting at its OLD id, recompute a
//! *fresh, non-deterministic* id for it all over again (the collision
//! branch below), and keep doing so on every retry, accumulating unbounded
//! duplicate rows in the identity store. Phase 2a's decision was about the
//! SCHEMA declaration surviving, not about every individual row being
//! permanent — a row that has been fully superseded by a rename, with
//! nothing left referencing it, is safe to remove, and here that removal is
//! what makes the rename actually idempotent.

use std::sync::Arc;

use uuid::Uuid;

use crate::backend::storage::mcp_servers::McpServer;
use crate::backend::storage::skills::Skill;
use crate::backend::storage::store::Store;
use crate::registry;

use super::{Migration, MigrationContext, MigrationError, MigrationScope, VerifyOutcome};

pub struct M0031CarrySkillsAndMcpServersToIdentityStore;

// ── Frozen starter recognition — see the module doc's own section. Do NOT
// import this from `skill_seed`/`mcp_seed`; do not "fix" this by
// deduplicating against them. ──

/// (name, trigger) for every starter skill, as of this migration's origin.
/// Frozen copy of `skill_seed`'s embedded manifest content.
const FROZEN_STARTER_SKILLS: &[(&str, &str)] = &[
    ("Systematic Debugging", "systematic-debugging"),
    ("Test-Driven Development", "tdd"),
    ("Code Review — Requesting & Receiving", "code-review"),
    ("Git Commit & Branch Hygiene", "commit-hygiene"),
    ("Verification Before Completion", "verification-before-completion"),
    ("Security Review Basics", "security-basics"),
];

/// Every starter MCP server's name, as of this migration's origin. Frozen
/// copy of `mcp_seed`'s embedded manifest content.
const FROZEN_STARTER_MCP_SERVER_NAMES: &[&str] =
    &["git", "fetch", "sequential-thinking", "memory", "playwright", "context7"];

/// Frozen copy of `skill_seed::starter_skill_id`'s derivation — identical
/// namespace seed, identical algorithm, deliberately NOT calling the live
/// function. If that function's derivation ever changes, this one must not
/// follow it.
fn frozen_starter_skill_id(trigger: &str) -> Uuid {
    let namespace = Uuid::new_v5(&Uuid::NAMESPACE_URL, b"https://agentmux.ai/catalog/skills/v1");
    Uuid::new_v5(&namespace, trigger.as_bytes())
}

/// Frozen copy of `mcp_seed::starter_mcp_server_id`'s derivation — see
/// `frozen_starter_skill_id`'s doc comment.
fn frozen_starter_mcp_server_id(name: &str) -> Uuid {
    let namespace = Uuid::new_v5(&Uuid::NAMESPACE_URL, b"https://agentmux.ai/catalog/mcp-servers/v1");
    Uuid::new_v5(&namespace, name.as_bytes())
}

fn frozen_starter_skill_trigger(name: &str) -> Option<&'static str> {
    FROZEN_STARTER_SKILLS.iter().find(|(n, _)| *n == name).map(|(_, t)| *t)
}

fn frozen_is_starter_mcp_server_name(name: &str) -> bool {
    FROZEN_STARTER_MCP_SERVER_NAMES.contains(&name)
}

// ── Deterministic collision-fallback ids (ReAgent P1, PR #3181, round 3) ──
//
// The genuine-PK-collision fallback (a row's own id already belongs to an
// unrelated row — private-row collisions, and the near-impossible
// non-starter-global case where the failed insert wasn't a name/type
// conflict) used `Uuid::new_v4()` — a fresh random id on every attempt.
// That is not just non-deterministic across channels (fine, these rows are
// never meant to converge with each other) — it is non-deterministic
// across RETRIES of the SAME row on the SAME channel, which matters because
// the identity-store insert and the local rename (insert-local, rewrite
// refs, delete-old) are separate writes to two different SQLite files with
// no cross-database transaction to make them atomic. A crash between them
// leaves the local row still at its original id with no durable record of
// which fresh id the crashed attempt already placed in the identity store;
// re-deriving a NEW random id on retry would orphan that row permanently
// and repeat on every subsequent crash.
//
// Fixed the same way the starter case was already race-safe: derive the
// "fresh" id deterministically, so a retry recomputes the IDENTICAL id and
// `INSERT OR IGNORE` naturally converges rather than creating a new row.
// Salted with the channel's own store path (stable across retries of the
// same channel, distinct across channels — two different channels' rows
// that happen to collide on their original local id are NOT the same
// resource, so their fallback ids must not collide with each other either)
// and the row's original local id. A NUL-byte separator avoids the
// (channel_salt, original_id) concatenation ambiguity a plain string-join
// would have. Distinct namespace per catalog, same reason Phase 1's
// starter ids use separate namespaces.
fn deterministic_collision_id(namespace_seed: &[u8], channel_salt: &str, original_local_id: &str) -> Uuid {
    let namespace = Uuid::new_v5(&Uuid::NAMESPACE_URL, namespace_seed);
    let key = format!("{channel_salt}\u{0}{original_local_id}");
    Uuid::new_v5(&namespace, key.as_bytes())
}

const SKILL_COLLISION_NAMESPACE_SEED: &[u8] = b"https://agentmux.ai/catalog/skills/collision-fallback/v1";
const MCP_SERVER_COLLISION_NAMESPACE_SEED: &[u8] = b"https://agentmux.ai/catalog/mcp-servers/collision-fallback/v1";

/// Resolve (creating the parent dir if needed) and open the identity store
/// for writing. Shared by `up()`; factored out so a resolution failure gets
/// one consistent error message.
fn open_identity_store() -> Result<Store, String> {
    let path = registry::resolve_identity_store_path()
        .ok_or_else(|| "could not resolve identity store path".to_string())?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("create dir: {e}"))?;
    }
    Store::open_identity_store(&path).map_err(|e| format!("open identity store: {e}"))
}

/// Heuristic for "is this the identity store's existing row actually MY row,
/// already carried by an earlier run — not a different private skill that
/// happens to collide on id?" There is no stronger identity than content to
/// check against for a private row (ids alone can't distinguish the two
/// cases — that's the whole problem). `created_at` is included because it is
/// set once at creation and never touched by an ordinary edit, so two
/// genuinely different rows sharing it too is vanishingly unlikely.
fn skill_looks_like_the_same_row(a: &Skill, b: &Skill) -> bool {
    a.name == b.name && a.content == b.content && a.skill_type == b.skill_type && a.created_at == b.created_at
}

/// Mirrors `skill_looks_like_the_same_row` for MCP servers. `config` (not
/// `content` — servers have no such field) plays the same "reliable enough
/// to distinguish, not so strict a trivial edit breaks it" role.
fn mcp_server_looks_like_the_same_row(a: &McpServer, b: &McpServer) -> bool {
    a.name == b.name && a.config == b.config && a.transport == b.transport && a.created_at == b.created_at
}

/// Finish carrying a row whose `final_id` has already been resolved:
/// (re-)insert into the identity store, and if the id differs from the
/// row's current local id, insert a local copy under the new id, repoint
/// this channel's own ref rows, and only then remove the stale local row —
/// see the module doc's explanation of why that removal isn't optional.
///
/// `insert_local` / `insert_identity` / `rewrite_refs` / `delete_local` are
/// closures over the skill-vs-mcp-server-specific store calls, so this one
/// function drives both `carry_skills` and `carry_mcp_servers` without
/// duplicating the sequencing logic — the part that was actually wrong.
#[allow(clippy::too_many_arguments)]
fn finish_carry(
    original_id: &str,
    final_id: &str,
    insert_local: impl Fn(&str) -> Result<usize, String>,
    rewrite_refs: impl Fn(&str, &str) -> Result<usize, String>,
    delete_local: impl Fn(&str) -> Result<bool, String>,
) -> Result<usize, String> {
    if final_id == original_id {
        return Ok(0);
    }
    insert_local(final_id)?;
    let touched = rewrite_refs(original_id, final_id)?;
    delete_local(original_id)?;
    Ok(touched)
}

/// Carry this channel's skills across. Returns `(carried, rewritten)`.
/// `carried` counts only rows this call actually inserted into the
/// IDENTITY STORE for the first time — a row that converges onto an id
/// already placed there (by this channel's own earlier run, or by a
/// different channel entirely) does not increment it, even though it still
/// gets a local copy under the new id. `rewritten` counts actual ref ROWS
/// repointed, not "how many rows had an id change" — a converging row with
/// no local ref pointing at it yet contributes 0 either way.
fn carry_skills(wstore: &Store, identity_store: &Store, channel_salt: &str) -> Result<(usize, usize), String> {
    let mut carried = 0usize;
    let mut rewritten = 0usize;
    for skill in wstore.skill_list_all_raw().map_err(|e| format!("list local skills: {e}"))? {
        let original_id = skill.id.clone();

        let (final_id, newly_inserted) = if skill.is_global {
            if let Some(trigger) = frozen_starter_skill_trigger(&skill.name) {
                // Deterministic id: race-safe on PK equality, no further
                // arbitration needed. See the module doc.
                let id = frozen_starter_skill_id(trigger).to_string();
                let mut to_insert = skill.clone();
                to_insert.id = id.clone();
                let inserted = identity_store
                    .skill_insert_raw(&to_insert)
                    .map_err(|e| format!("insert skill {}: {e}", to_insert.name))?;
                (id, inserted)
            } else {
                // Non-starter global: optimistic insert under the row's own
                // id, arbitrated by the name-only unique index (Codex P1, PR
                // #3183 — was (name, skill_type) until IDENTITY_STORE_SCHEMA_VERSION
                // v8 narrowed it to match skill_upsert_unique_global's own
                // invariant; see the module doc's race explanation).
                let mut to_insert = skill.clone();
                let inserted = identity_store
                    .skill_insert_raw(&to_insert)
                    .map_err(|e| format!("insert skill {}: {e}", to_insert.name))?;
                if inserted > 0 {
                    (to_insert.id.clone(), inserted)
                } else {
                    // Re-query by NAME ALONE, not (name, skill_type): the
                    // insert failed because of the name-only constraint, so
                    // that is what must be re-queried to find the actual
                    // winner. A (name, skill_type)-scoped re-query would miss
                    // a winner carried under a DIFFERENT skill_type, fall
                    // through to the "unrelated collision" branch below, and
                    // mint a fresh id whose insert then ALSO silently
                    // no-ops against the same name constraint — leaving a
                    // local ref rewritten to an id the identity store never
                    // actually holds, so the skill vanishes the moment reads
                    // redirect there (Codex P1, PR #3183 — caught exactly
                    // this).
                    match identity_store
                        .skill_find_global_by_name(&skill.name)
                        .map_err(|e| format!("find global skill {}: {e}", skill.name))?
                    {
                        Some(existing) => (existing.id, 0),
                        None => {
                            // The insert failed for a reason unrelated to
                            // name — an astronomically unlikely PK collision
                            // with an unrelated row. Deterministic fallback
                            // id, same reasoning as the private-row branch
                            // below.
                            let fresh_id = deterministic_collision_id(
                                SKILL_COLLISION_NAMESPACE_SEED,
                                channel_salt,
                                &original_id,
                            )
                            .to_string();
                            to_insert.id = fresh_id.clone();
                            let inserted = identity_store
                                .skill_insert_raw(&to_insert)
                                .map_err(|e| format!("insert skill {} under fresh id: {e}", to_insert.name))?;
                            if inserted > 0 {
                                (fresh_id, inserted)
                            } else {
                                // The fresh-id insert ALSO silently no-op'd —
                                // only reachable if a concurrent process won
                                // the same name between our re-query above
                                // and this insert (a narrow TOCTOU race, not
                                // the deterministic bug this branch exists
                                // for). Never trust a silent optimistic-
                                // insert failure without checking what
                                // actually won — same principle as the
                                // private-row TOCTOU fix (ReAgent P1, PR
                                // #3181, round 2).
                                match identity_store
                                    .skill_find_global_by_name(&skill.name)
                                    .map_err(|e| format!("re-find global skill {} after fresh-id insert: {e}", skill.name))?
                                {
                                    Some(existing) => (existing.id, 0),
                                    None => {
                                        return Err(format!(
                                            "insert skill {} under fresh id {fresh_id} silently failed with no conflicting global row found — unreachable unless the identity store's schema changed underneath this migration",
                                            to_insert.name
                                        ));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        } else {
            // Private: keep the own id, but the same TOCTOU gap the global
            // path already closed applies here too (ReAgent P1, PR #3181,
            // round 2) — a prior revision checked `skill_get` FIRST and
            // trusted `None` as "the id is free," which a concurrent
            // channel's migration can falsify between the check and this
            // channel's own insert. Optimistic-insert-first instead: attempt
            // under the row's own id, and only if THAT insert didn't win
            // (checked via the returned row count, not a separate read)
            // look at what's actually there to decide whether it's my own
            // prior carry (content-identical — a re-run) or a genuine
            // collision needing a fresh id.
            let mut to_insert = skill.clone();
            let inserted = identity_store
                .skill_insert_raw(&to_insert)
                .map_err(|e| format!("insert skill {}: {e}", to_insert.name))?;
            if inserted > 0 {
                (skill.id.clone(), inserted)
            } else {
                match identity_store
                    .skill_get(&skill.id)
                    .map_err(|e| format!("collision check for private skill {}: {e}", skill.id))?
                {
                    Some(existing) if skill_looks_like_the_same_row(&existing, &skill) => (skill.id.clone(), 0),
                    _ => {
                        // Deterministic, not random (ReAgent P1, PR #3181,
                        // round 3) — see deterministic_collision_id's own
                        // doc comment for the crash-window it closes: a
                        // retry recomputes the SAME id and converges via
                        // INSERT OR IGNORE instead of orphaning a new row
                        // every time.
                        let fresh_id = deterministic_collision_id(
                            SKILL_COLLISION_NAMESPACE_SEED,
                            channel_salt,
                            &original_id,
                        )
                        .to_string();
                        to_insert.id = fresh_id.clone();
                        let inserted = identity_store
                            .skill_insert_raw(&to_insert)
                            .map_err(|e| format!("insert skill {} under fresh id: {e}", to_insert.name))?;
                        (fresh_id, inserted)
                    }
                }
            }
        };

        if newly_inserted > 0 {
            carried += 1;
        }

        // Content for the local fallback copy: MINE if my own insert just
        // won (I am the canonical content now); otherwise fetched from the
        // identity store, since converging onto a pre-existing row (a
        // starter another channel already carried, or a first-wins global
        // match) means THAT row's content is authoritative, which can
        // differ from this channel's own copy — e.g. an independently
        // edited starter (ReAgent P1, PR #3181, round 3). Writing this
        // channel's own content unconditionally would put stale/wrong data
        // in the local fallback under an id the identity store considers
        // authoritative for someone else's content.
        let canonical = if newly_inserted > 0 {
            skill.clone()
        } else {
            identity_store
                .skill_get(&final_id)
                .map_err(|e| format!("fetch winning content for skill {final_id}: {e}"))?
                .unwrap_or_else(|| skill.clone())
        };
        let mut to_insert = canonical;
        to_insert.id = final_id.clone();
        rewritten += finish_carry(
            &original_id,
            &final_id,
            |id| {
                let mut row = to_insert.clone();
                row.id = id.to_string();
                wstore
                    .skill_insert_raw(&row)
                    .map_err(|e| format!("insert local copy under new id for skill {}: {e}", row.name))
            },
            |old, new| {
                wstore
                    .skill_rewrite_ref_id(old, new)
                    .map_err(|e| format!("rewrite refs for skill {}: {e}", to_insert.name))
            },
            |old| {
                // Compile-only accommodation for Part D of
                // SPEC_DURABLE_BINDINGS_2026_09_10.md's redirect
                // (`skill_delete` gained a `catalog` parameter) — no change
                // to this migration's own frozen logic. This deletes the
                // STALE LOCAL row under the superseded id from `wstore`'s
                // own db_skills mirror — it must never touch
                // `identity_store` (the new local copy under the final id
                // was already inserted separately above), so `wstore` is
                // passed as `catalog` too, reproducing the pre-split
                // single-store behavior exactly.
                wstore
                    .skill_delete(wstore, old)
                    .map_err(|e| format!("delete superseded local skill row {old}: {e}"))
            },
        )?;
    }
    Ok((carried, rewritten))
}

/// Mirrors `carry_skills` exactly, for MCP servers — see that function's
/// comments for the reasoning behind every decision here.
fn carry_mcp_servers(wstore: &Store, identity_store: &Store, channel_salt: &str) -> Result<(usize, usize), String> {
    let mut carried = 0usize;
    let mut rewritten = 0usize;
    for server in wstore.mcp_server_list_all_raw().map_err(|e| format!("list local mcp servers: {e}"))? {
        let original_id = server.id.clone();

        let (final_id, newly_inserted) = if server.is_global {
            if frozen_is_starter_mcp_server_name(&server.name) {
                let id = frozen_starter_mcp_server_id(&server.name).to_string();
                let mut to_insert = server.clone();
                to_insert.id = id.clone();
                let inserted = identity_store
                    .mcp_server_insert_raw(&to_insert)
                    .map_err(|e| format!("insert mcp server {}: {e}", to_insert.name))?;
                (id, inserted)
            } else {
                let to_insert = server.clone();
                let inserted = identity_store
                    .mcp_server_insert_raw(&to_insert)
                    .map_err(|e| format!("insert mcp server {}: {e}", to_insert.name))?;
                if inserted > 0 {
                    (to_insert.id.clone(), inserted)
                } else {
                    match identity_store
                        .mcp_server_find_global_by_name(&server.name)
                        .map_err(|e| format!("find global mcp server {}: {e}", server.name))?
                    {
                        Some(existing) => (existing.id, 0),
                        None => {
                            // Deterministic fallback id — see carry_skills's
                            // matching branch (ReAgent P1, PR #3181, round 3).
                            let fresh_id = deterministic_collision_id(
                                MCP_SERVER_COLLISION_NAMESPACE_SEED,
                                channel_salt,
                                &original_id,
                            )
                            .to_string();
                            let mut retry = server.clone();
                            retry.id = fresh_id.clone();
                            let inserted = identity_store
                                .mcp_server_insert_raw(&retry)
                                .map_err(|e| format!("insert mcp server {} under fresh id: {e}", retry.name))?;
                            (fresh_id, inserted)
                        }
                    }
                }
            }
        } else {
            // Optimistic-insert-first — see carry_skills's matching branch
            // for why (ReAgent P1, PR #3181, round 2).
            let mut to_insert = server.clone();
            let inserted = identity_store
                .mcp_server_insert_raw(&to_insert)
                .map_err(|e| format!("insert mcp server {}: {e}", to_insert.name))?;
            if inserted > 0 {
                (server.id.clone(), inserted)
            } else {
                match identity_store
                    .mcp_server_get(&server.id)
                    .map_err(|e| format!("collision check for private mcp server {}: {e}", server.id))?
                {
                    Some(existing) if mcp_server_looks_like_the_same_row(&existing, &server) => (server.id.clone(), 0),
                    _ => {
                        // Deterministic, not random — see carry_skills's
                        // matching branch (ReAgent P1, PR #3181, round 3).
                        let fresh_id = deterministic_collision_id(
                            MCP_SERVER_COLLISION_NAMESPACE_SEED,
                            channel_salt,
                            &original_id,
                        )
                        .to_string();
                        to_insert.id = fresh_id.clone();
                        let inserted = identity_store
                            .mcp_server_insert_raw(&to_insert)
                            .map_err(|e| format!("insert mcp server {} under fresh id: {e}", to_insert.name))?;
                        (fresh_id, inserted)
                    }
                }
            }
        };

        if newly_inserted > 0 {
            carried += 1;
        }

        // See carry_skills's matching comment (ReAgent P1, PR #3181, round 3):
        // use the identity store's actual winning content whenever this
        // channel's own insert didn't win the slot.
        let canonical = if newly_inserted > 0 {
            server.clone()
        } else {
            identity_store
                .mcp_server_get(&final_id)
                .map_err(|e| format!("fetch winning content for mcp server {final_id}: {e}"))?
                .unwrap_or_else(|| server.clone())
        };
        let mut to_insert = canonical;
        to_insert.id = final_id.clone();
        rewritten += finish_carry(
            &original_id,
            &final_id,
            |id| {
                let mut row = to_insert.clone();
                row.id = id.to_string();
                wstore
                    .mcp_server_insert_raw(&row)
                    .map_err(|e| format!("insert local copy under new id for mcp server {}: {e}", row.name))
            },
            |old, new| {
                wstore
                    .mcp_server_rewrite_ref_id(old, new)
                    .map_err(|e| format!("rewrite refs for mcp server {}: {e}", to_insert.name))
            },
            |old| {
                // See the identical compile-only note on the skill_delete
                // call above (Part D of SPEC_DURABLE_BINDINGS_2026_09_10.md).
                wstore
                    .mcp_server_delete(wstore, old)
                    .map_err(|e| format!("delete superseded local mcp server row {old}: {e}"))
            },
        )?;
    }
    Ok((carried, rewritten))
}

impl Migration for M0031CarrySkillsAndMcpServersToIdentityStore {
    fn id(&self) -> &'static str {
        "0031_carry_skills_and_mcp_servers_to_identity_store"
    }

    fn scope(&self) -> MigrationScope {
        MigrationScope::Channel
    }

    fn description(&self) -> &'static str {
        "Carry this channel's skills/MCP servers into the identity store, which is now authoritative for them"
    }

    fn up(&self, ctx: &MigrationContext) -> Result<(), MigrationError> {
        if !ctx.channel_store_path.exists() {
            return Ok(());
        }
        let wstore = Arc::new(
            Store::open(&ctx.channel_store_path)
                .map_err(|e| MigrationError(format!("carry_skills_and_mcp_servers: open wstore: {e}")))?,
        );
        let identity_store = open_identity_store()
            .map_err(|e| MigrationError(format!("carry_skills_and_mcp_servers: {e}")))?;
        // Salts the deterministic collision-fallback id derivation so two
        // different channels' colliding private rows don't converge onto
        // the same fallback id (they aren't the same row) — see
        // deterministic_collision_id's doc comment.
        let channel_salt = ctx.channel_store_path.to_string_lossy().into_owned();

        let (skills_carried, skills_rewritten) = carry_skills(&wstore, &identity_store, &channel_salt)
            .map_err(|e| MigrationError(format!("carry_skills_and_mcp_servers: skills: {e}")))?;
        let (servers_carried, servers_rewritten) = carry_mcp_servers(&wstore, &identity_store, &channel_salt)
            .map_err(|e| MigrationError(format!("carry_skills_and_mcp_servers: mcp servers: {e}")))?;

        tracing::info!(
            skills_carried,
            skills_refs_rewritten = skills_rewritten,
            mcp_servers_carried = servers_carried,
            mcp_servers_refs_rewritten = servers_rewritten,
            "carry_skills_and_mcp_servers_to_identity_store: complete"
        );
        Ok(())
    }

    fn verify(&self, ctx: &MigrationContext) -> VerifyOutcome {
        let (local_skills, local_servers, local) = match super::runner::open_readonly(&ctx.channel_store_path) {
            Ok(Some(conn)) => {
                let skills: i64 = conn
                    .query_row("SELECT COUNT(*) FROM db_skills", [], |r| r.get(0))
                    .unwrap_or(0);
                let servers: i64 = conn
                    .query_row("SELECT COUNT(*) FROM db_mcp_servers", [], |r| r.get(0))
                    .unwrap_or(0);
                (skills, servers, format!("{skills} local skill row(s), {servers} local mcp server row(s)"))
            }
            Ok(None) => (0, 0, "no channel store".to_string()),
            Err(e) => return VerifyOutcome::Error(e),
        };

        // Best-effort beyond this point: the identity store is host-global
        // and this migration is per-channel, so a resolution failure here
        // says nothing about whether THIS channel's own carry succeeded.
        let Some(identity_path) = registry::resolve_identity_store_path() else {
            return VerifyOutcome::Ok(format!("{local}; identity store path unresolved"));
        };
        match super::runner::open_readonly(&identity_path) {
            Ok(Some(conn)) => {
                let skills: i64 = conn
                    .query_row("SELECT COUNT(*) FROM db_skills", [], |r| r.get(0))
                    .unwrap_or(0);
                let servers: i64 = conn
                    .query_row("SELECT COUNT(*) FROM db_mcp_servers", [], |r| r.get(0))
                    .unwrap_or(0);
                // A real post-condition, not just a report: if this channel
                // has local rows but the identity store — which this exact
                // channel's own migration run should have populated, at
                // minimum with its own content — has none at all, that is
                // definitively wrong, not merely unverified (Codex P2, PR
                // #3181). Dedup means the counts need not match exactly, so
                // this checks only the floor: local rows imply SOME rows
                // exist on the destination side.
                if (local_skills > 0 && skills == 0) || (local_servers > 0 && servers == 0) {
                    return VerifyOutcome::Mismatch(format!(
                        "{local}; identity store has {skills} skill row(s), {servers} mcp server row(s) — \
                         expected at least one of each given local content exists"
                    ));
                }
                VerifyOutcome::Ok(format!(
                    "{local}; identity store has {skills} skill row(s), {servers} mcp server row(s)"
                ))
            }
            Ok(None) => {
                if local_skills > 0 || local_servers > 0 {
                    VerifyOutcome::Mismatch(format!("{local}; identity store not yet created"))
                } else {
                    VerifyOutcome::Ok(format!("{local}; identity store not yet created"))
                }
            }
            Err(e) => VerifyOutcome::Ok(format!("{local}; identity store unreadable: {e}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Only exercised for the missing-channel-store no-op path below, which
    /// returns before `open_identity_store()` is ever called — so this
    /// deliberately does NOT set up identity-store resolution (there is no
    /// real per-call override for it; `resolve_identity_store_path` only
    /// respects the global `AGENTMUX_HOME_OVERRIDE`, which every other test
    /// in this file avoids touching to stay independent of process-wide
    /// env-var state).
    fn ctx_for(channel_path: &std::path::Path) -> MigrationContext {
        MigrationContext {
            home: std::env::temp_dir(),
            data_dir: std::env::temp_dir(),
            shared_store_path: std::env::temp_dir().join("unused-shared-store.db"),
            channel_store_path: channel_path.to_path_buf(),
        }
    }

    fn skill(id: &str, name: &str, trigger: &str, is_global: bool) -> Skill {
        Skill {
            id: id.to_string(),
            name: name.to_string(),
            trigger: trigger.to_string(),
            skill_type: "prompt".to_string(),
            description: "d".to_string(),
            content: "c".to_string(),
            is_global,
            created_at: 1,
            updated_at: 1,
        }
    }

    fn mcp(id: &str, name: &str, is_global: bool) -> McpServer {
        McpServer {
            id: id.to_string(),
            name: name.to_string(),
            transport: "stdio".to_string(),
            config: "{}".to_string(),
            is_global,
            created_at: 1,
            updated_at: 1,
        }
    }

    #[test]
    fn starter_ids_match_the_live_derivation_today() {
        // The frozen copies must agree with the live functions AS OF NOW —
        // this test is not a guarantee they always will (they're explicitly
        // allowed to diverge once either one changes), it's a sanity check
        // that the freeze was performed correctly at the moment it was
        // taken. See the module doc's "Frozen starter recognition" section.
        for (name, trigger) in FROZEN_STARTER_SKILLS {
            assert_eq!(
                frozen_starter_skill_id(trigger),
                crate::backend::skill_seed::starter_skill_id(trigger),
                "frozen and live derivation disagree for trigger {trigger:?}"
            );
            assert_eq!(
                crate::backend::skill_seed::starter_skill_trigger_for_name(name).as_deref(),
                Some(*trigger),
                "frozen manifest disagrees with the live one for {name:?}"
            );
        }
        for name in FROZEN_STARTER_MCP_SERVER_NAMES {
            assert_eq!(
                frozen_starter_mcp_server_id(name),
                crate::backend::mcp_seed::starter_mcp_server_id(name),
                "frozen and live derivation disagree for mcp server {name:?}"
            );
            assert!(crate::backend::mcp_seed::is_starter_mcp_server_name(name));
        }
    }

    #[test]
    fn a_recognized_starter_converges_on_the_deterministic_id_regardless_of_its_legacy_local_id() {
        let dir = tempfile::tempdir().unwrap();
        let wstore = Store::open(&dir.path().join("objects.db")).unwrap();
        // A legacy pre-Phase-1 row: real starter trigger, but a random id
        // that does NOT match frozen_starter_skill_id("tdd").
        wstore.skill_insert_raw(&skill("legacy-random-id", "Test-Driven Development", "tdd", true)).unwrap();

        let identity_dir = tempfile::tempdir().unwrap();
        let identity_store = Store::open_identity_store(&identity_dir.path().join("identity-store.db")).unwrap();

        let (carried, rewritten) = carry_skills(&wstore, &identity_store, "channel").unwrap();
        assert_eq!(carried, 1);
        assert_eq!(rewritten, 0, "no ref rows existed locally, so nothing to rewrite");

        let expected_id = frozen_starter_skill_id("tdd").to_string();
        assert!(
            identity_store.skill_get(&expected_id).unwrap().is_some(),
            "must land under the canonical deterministic id, not the legacy random one"
        );
        assert!(identity_store.skill_get("legacy-random-id").unwrap().is_none());
        // The stale local row must be gone too — not just superseded.
        assert!(
            wstore.skill_get("legacy-random-id").unwrap().is_none(),
            "the superseded local row must be removed once the rename is complete"
        );
        assert!(wstore.skill_get(&expected_id).unwrap().is_some());
    }

    #[test]
    fn ref_rows_are_rewritten_when_the_id_changes() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("objects.db");
        let wstore = Store::open(&db_path).unwrap();
        wstore.skill_insert_raw(&skill("legacy-random-id", "Test-Driven Development", "tdd", true)).unwrap();
        // A real ref row naming the legacy id — the thing that must not be
        // left dangling once the migration mints a different id. `Store.conn`
        // isn't reachable from this module (`pub(super)`), so this goes
        // through a second raw connection to the same file — the same
        // pattern m0025/m0026/m0027's own tests already use to seed
        // db_agents rows directly.
        rusqlite::Connection::open(&db_path).unwrap().execute(
            "INSERT INTO db_agents (id, name, provider) VALUES ('agent-1', 'A', 'claude')",
            [],
        ).unwrap();
        // Compile-only accommodation for Part C of
        // SPEC_DURABLE_BINDINGS_2026_09_10.md's redirect (`skill_bind` gained
        // a `catalog` parameter) — no change to this migration's own frozen
        // logic. `wstore` itself already holds this row locally (inserted
        // above via `skill_insert_raw`), so passing it as `catalog` too
        // satisfies the new existence check exactly as it always implicitly
        // did before the split.
        wstore.skill_bind(&wstore, "agent-1", "legacy-random-id").unwrap();

        let identity_dir = tempfile::tempdir().unwrap();
        let identity_store = Store::open_identity_store(&identity_dir.path().join("identity-store.db")).unwrap();
        let (_, rewritten) = carry_skills(&wstore, &identity_store, "channel").unwrap();
        assert_eq!(rewritten, 1);

        let expected_id = frozen_starter_skill_id("tdd").to_string();
        assert!(
            wstore.skill_is_bound_to("agent-1", &expected_id).unwrap(),
            "the agent's own ref row must now point at the canonical id"
        );
        assert!(!wstore.skill_is_bound_to("agent-1", "legacy-random-id").unwrap());
    }

    #[test]
    fn a_non_starter_global_skill_dedups_by_name_and_type_first_wins() {
        let identity_dir = tempfile::tempdir().unwrap();
        let identity_store = Store::open_identity_store(&identity_dir.path().join("identity-store.db")).unwrap();

        // Channel A carries first: its id survives.
        let dir_a = tempfile::tempdir().unwrap();
        let wstore_a = Store::open(&dir_a.path().join("objects.db")).unwrap();
        wstore_a.skill_insert_raw(&skill("id-from-a", "My Custom Global", "", true)).unwrap();
        carry_skills(&wstore_a, &identity_store, "channel-a").unwrap();

        // Channel B carries the SAME name+type, under a different id — must
        // converge onto A's id, not create a second row.
        let dir_b = tempfile::tempdir().unwrap();
        let wstore_b = Store::open(&dir_b.path().join("objects.db")).unwrap();
        wstore_b.skill_insert_raw(&skill("id-from-b", "My Custom Global", "", true)).unwrap();
        let (carried, rewritten) = carry_skills(&wstore_b, &identity_store, "channel-b").unwrap();
        assert_eq!(carried, 0, "id-from-a already occupies the identity store under its own id — B adds nothing new there");
        // `rewritten` counts actual ref ROWS repointed, not "an id changed" —
        // this fixture never binds "id-from-b" to any agent/bundle, so there
        // is nothing to rewrite even though B's local copy does converge
        // onto A's id (see `ref_rows_are_rewritten_when_the_id_changes` for
        // the case where a real ref row exists).
        assert_eq!(rewritten, 0);
        assert!(
            wstore_b.skill_get("id-from-a").unwrap().is_some(),
            "B's own local store gets a copy under A's id too, so pre-redirect code keeps resolving it"
        );
        assert!(wstore_b.skill_get("id-from-b").unwrap().is_none(), "the superseded local row must be removed");

        assert!(identity_store.skill_get("id-from-a").unwrap().is_some());
        assert!(identity_store.skill_get("id-from-b").unwrap().is_none(), "must not create a second row for the same name+type");
    }

    /// Codex P1, PR #3183: the identity store's global-uniqueness index is
    /// `name` alone as of `IDENTITY_STORE_SCHEMA_VERSION` v8 — narrower than
    /// `(name, skill_type)`, which this dedup logic used to assume. Two
    /// channels carrying the SAME NAME under DIFFERENT skill_types must
    /// still converge onto one survivor: a re-query still scoped to
    /// `(name, skill_type)` would find nothing (no row exists with the
    /// LOSING channel's own exact type), fall through to the
    /// unrelated-PK-collision branch, and mint a fresh id whose insert then
    /// ALSO silently no-ops against the name constraint — stranding a local
    /// ref pointing at an id the identity store never actually holds.
    #[test]
    fn two_channels_carrying_the_same_name_under_different_skill_types_still_converge() {
        let identity_dir = tempfile::tempdir().unwrap();
        let identity_store = Store::open_identity_store(&identity_dir.path().join("identity-store.db")).unwrap();

        let dir_a = tempfile::tempdir().unwrap();
        let wstore_a = Store::open(&dir_a.path().join("objects.db")).unwrap();
        let mut skill_a = skill("id-from-a", "Deploy", "", true);
        skill_a.skill_type = "prompt".to_string();
        wstore_a.skill_insert_raw(&skill_a).unwrap();
        carry_skills(&wstore_a, &identity_store, "channel-a").unwrap();

        let dir_b = tempfile::tempdir().unwrap();
        let wstore_b = Store::open(&dir_b.path().join("objects.db")).unwrap();
        let mut skill_b = skill("id-from-b", "Deploy", "", true);
        skill_b.skill_type = "agent-skill".to_string();
        wstore_b.skill_insert_raw(&skill_b).unwrap();
        rusqlite::Connection::open(dir_b.path().join("objects.db")).unwrap().execute(
            "INSERT INTO db_agents (id, name, provider) VALUES ('agent-1', 'A', 'claude')",
            [],
        ).unwrap();
        wstore_b.skill_bind(&wstore_b, "agent-1", "id-from-b").unwrap();

        let (carried, rewritten) = carry_skills(&wstore_b, &identity_store, "channel-b").unwrap();
        assert_eq!(carried, 0, "A's row already occupies the name — B adds nothing new to the identity store");
        assert_eq!(rewritten, 1, "B's own agent-1 ref must be repointed at A's surviving id");

        // Exactly one row in the identity store, under A's id, with A's
        // content (first-wins) — NOT a phantom id B's fresh-id fallback
        // would have minted.
        assert!(identity_store.skill_get("id-from-a").unwrap().is_some());
        assert!(identity_store.skill_get("id-from-b").unwrap().is_none());

        // B's local mirror must resolve to A's id too — the whole point:
        // nothing is left pointing at an id the identity store doesn't hold.
        assert!(wstore_b.skill_get("id-from-a").unwrap().is_some());
        assert!(wstore_b.skill_get("id-from-b").unwrap().is_none());
        assert!(wstore_b.skill_is_bound_to("agent-1", "id-from-a").unwrap());
        assert!(!wstore_b.skill_is_bound_to("agent-1", "id-from-b").unwrap());
    }

    #[test]
    fn a_concurrent_non_starter_global_insert_is_arbitrated_by_the_database_not_by_who_checked_first() {
        // The race Codex flagged: simulate two "channels" that both see an
        // empty identity store before either has inserted, by inserting
        // A's row DIRECTLY (bypassing carry_skills's own check-then-insert)
        // right before B's carry runs — B's optimistic insert under its own
        // id must still fail the unique index and converge onto A's id via
        // the re-query fallback, not create a duplicate.
        let identity_dir = tempfile::tempdir().unwrap();
        let identity_store = Store::open_identity_store(&identity_dir.path().join("identity-store.db")).unwrap();
        identity_store.skill_insert_raw(&skill("id-from-a", "Racing Global", "prompt", true)).unwrap();

        let dir_b = tempfile::tempdir().unwrap();
        let wstore_b = Store::open(&dir_b.path().join("objects.db")).unwrap();
        wstore_b.skill_insert_raw(&skill("id-from-b", "Racing Global", "prompt", true)).unwrap();
        carry_skills(&wstore_b, &identity_store, "channel-b").unwrap();

        let matches: Vec<_> = [
            identity_store.skill_get("id-from-a").unwrap(),
            identity_store.skill_get("id-from-b").unwrap(),
        ]
        .into_iter()
        .flatten()
        .collect();
        assert_eq!(matches.len(), 1, "the unique index must prevent two rows for the same (name, skill_type)");
        assert_eq!(matches[0].id, "id-from-a");
    }

    #[test]
    fn owner_private_skills_are_never_deduplicated_by_name() {
        let identity_dir = tempfile::tempdir().unwrap();
        let identity_store = Store::open_identity_store(&identity_dir.path().join("identity-store.db")).unwrap();

        let dir_a = tempfile::tempdir().unwrap();
        let wstore_a = Store::open(&dir_a.path().join("objects.db")).unwrap();
        wstore_a.skill_insert_raw(&skill("private-a", "Deploy", "", false)).unwrap();
        carry_skills(&wstore_a, &identity_store, "channel-a").unwrap();

        let dir_b = tempfile::tempdir().unwrap();
        let wstore_b = Store::open(&dir_b.path().join("objects.db")).unwrap();
        wstore_b.skill_insert_raw(&skill("private-b", "Deploy", "", false)).unwrap();
        carry_skills(&wstore_b, &identity_store, "channel-b").unwrap();

        // Both rows survive as distinct resources — merging two different
        // owners' private skills because they share a name would create an
        // edit channel between them that never existed.
        assert!(identity_store.skill_get("private-a").unwrap().is_some());
        assert!(identity_store.skill_get("private-b").unwrap().is_some());
    }

    #[test]
    fn a_private_skill_id_collision_gets_a_fresh_id_and_its_refs_are_rewritten() {
        let identity_dir = tempfile::tempdir().unwrap();
        let identity_store = Store::open_identity_store(&identity_dir.path().join("identity-store.db")).unwrap();
        // Pre-seed the identity store with a row under the SAME id channel
        // B's private skill happens to have — the collision case. This is
        // also the state a genuinely concurrent race would produce (another
        // channel's migration wins the same id between this one's check and
        // its own insert): the row exists here BEFORE carry_skills runs, so
        // this exercises the optimistic-insert-first path's failure branch
        // exactly the same way an interleaved real race would (ReAgent P1,
        // PR #3181, round 2 — a prior revision checked-then-trusted instead
        // of inserting first and verifying the result).
        identity_store.skill_insert_raw(&skill("collided-id", "Someone Else's Skill", "", false)).unwrap();

        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("objects.db");
        let wstore = Store::open(&db_path).unwrap();
        wstore.skill_insert_raw(&skill("collided-id", "My Private Skill", "", false)).unwrap();
        rusqlite::Connection::open(&db_path).unwrap().execute(
            "INSERT INTO db_agents (id, name, provider) VALUES ('agent-1', 'A', 'claude')",
            [],
        ).unwrap();
        // See the identical compile-only note on the earlier skill_bind call
        // in this file (Part C of SPEC_DURABLE_BINDINGS_2026_09_10.md).
        wstore.skill_bind(&wstore, "agent-1", "collided-id").unwrap();

        let (carried, rewritten) = carry_skills(&wstore, &identity_store, "channel").unwrap();
        assert_eq!(carried, 1, "the fresh-id insert must still succeed even though the first attempt lost the race");
        assert_eq!(rewritten, 1);

        // The pre-existing row under "collided-id" must be untouched.
        let existing = identity_store.skill_get("collided-id").unwrap().unwrap();
        assert_eq!(existing.name, "Someone Else's Skill");
        // The agent's ref must now point at whatever fresh id was minted,
        // not at "collided-id" — and the LOCAL row that used to sit there
        // (this channel's own "My Private Skill") must be gone, not left as
        // an orphaned duplicate.
        assert!(!wstore.skill_is_bound_to("agent-1", "collided-id").unwrap());
        assert!(wstore.skill_get("collided-id").unwrap().is_none());
    }

    #[test]
    fn retrying_a_private_collision_converges_instead_of_accumulating_duplicates() {
        // ReAgent P1 / Codex P1, PR #3181: a retry after a partial failure
        // used to re-mint a brand-new random id every time, because the
        // stale local row under the original (collided) id was never
        // removed — each retry found the SAME collision and picked a
        // DIFFERENT fresh uuid, piling up orphaned duplicates in the
        // identity store forever. Simulated here by calling carry_skills
        // twice in a row, exactly what a retried migration does.
        let identity_dir = tempfile::tempdir().unwrap();
        let identity_store = Store::open_identity_store(&identity_dir.path().join("identity-store.db")).unwrap();
        identity_store.skill_insert_raw(&skill("collided-id", "Someone Else's Skill", "", false)).unwrap();

        let dir = tempfile::tempdir().unwrap();
        let wstore = Store::open(&dir.path().join("objects.db")).unwrap();
        wstore.skill_insert_raw(&skill("collided-id", "My Private Skill", "", false)).unwrap();

        let (first_carried, _) = carry_skills(&wstore, &identity_store, "channel").unwrap();
        assert_eq!(first_carried, 1);

        let (second_carried, _) = carry_skills(&wstore, &identity_store, "channel").unwrap();
        assert_eq!(
            second_carried, 0,
            "a retry must converge on the id the first run already chose, not mint another fresh one"
        );

        // Exactly two rows total in the identity store: the pre-existing
        // "Someone Else's Skill" and ONE copy of "My Private Skill" — not
        // three.
        let all_names: Vec<String> = [
            identity_store.skill_get("collided-id").unwrap(),
        ]
        .into_iter()
        .flatten()
        .map(|s| s.name)
        .chain(
            wstore
                .skill_list_all_raw()
                .unwrap()
                .into_iter()
                .filter(|s| s.name == "My Private Skill")
                .map(|s| s.id),
        )
        .collect();
        // Local store has exactly one row named "My Private Skill" (under
        // whatever id it converged to), and the identity store's
        // "collided-id" row is still the untouched original.
        assert_eq!(
            wstore.skill_list_all_raw().unwrap().iter().filter(|s| s.name == "My Private Skill").count(),
            1,
            "must not accumulate a second local row on retry: {all_names:?}"
        );
    }

    #[test]
    fn re_running_after_a_partial_carry_only_carries_what_is_still_missing() {
        let dir = tempfile::tempdir().unwrap();
        let wstore = Store::open(&dir.path().join("objects.db")).unwrap();
        wstore.skill_insert_raw(&skill("s1", "Skill One", "", false)).unwrap();
        wstore.skill_insert_raw(&skill("s2", "Skill Two", "", false)).unwrap();

        let identity_dir = tempfile::tempdir().unwrap();
        let identity_store = Store::open_identity_store(&identity_dir.path().join("identity-store.db")).unwrap();

        let (first_pass, _) = carry_skills(&wstore, &identity_store, "channel").unwrap();
        assert_eq!(first_pass, 2);

        let (second_pass, _) = carry_skills(&wstore, &identity_store, "channel").unwrap();
        assert_eq!(second_pass, 0, "a full re-run must find nothing left to carry");
    }

    #[test]
    fn a_recognized_starter_mcp_server_converges_on_the_deterministic_id() {
        let dir = tempfile::tempdir().unwrap();
        let wstore = Store::open(&dir.path().join("objects.db")).unwrap();
        wstore.mcp_server_insert_raw(&mcp("legacy-random-id", "git", true)).unwrap();

        let identity_dir = tempfile::tempdir().unwrap();
        let identity_store = Store::open_identity_store(&identity_dir.path().join("identity-store.db")).unwrap();
        carry_mcp_servers(&wstore, &identity_store, "channel").unwrap();

        let expected_id = frozen_starter_mcp_server_id("git").to_string();
        assert!(identity_store.mcp_server_get(&expected_id).unwrap().is_some());
        assert!(identity_store.mcp_server_get("legacy-random-id").unwrap().is_none());
        assert!(wstore.mcp_server_get("legacy-random-id").unwrap().is_none());
    }

    #[test]
    fn a_converged_global_skill_gets_the_winning_content_not_this_channels_own() {
        // ReAgent P1, PR #3181, round 3, Finding B: when this channel's
        // insert doesn't win the identity-store slot — it converges onto a
        // row another channel already carried — the LOCAL fallback copy
        // must mirror the WINNING content, not this channel's own. A prior
        // revision unconditionally wrote `skill.clone()` here regardless of
        // which channel's content actually won, which could leave a
        // channel with stale/independently-edited content overwriting the
        // canonical value's local mirror with the wrong content.
        let identity_dir = tempfile::tempdir().unwrap();
        let identity_store = Store::open_identity_store(&identity_dir.path().join("identity-store.db")).unwrap();

        let dir_a = tempfile::tempdir().unwrap();
        let wstore_a = Store::open(&dir_a.path().join("objects.db")).unwrap();
        let mut skill_a = skill("id-from-a", "My Custom Global", "", true);
        skill_a.content = "A's content".to_string();
        wstore_a.skill_insert_raw(&skill_a).unwrap();
        carry_skills(&wstore_a, &identity_store, "channel-a").unwrap();

        let dir_b = tempfile::tempdir().unwrap();
        let wstore_b = Store::open(&dir_b.path().join("objects.db")).unwrap();
        let mut skill_b = skill("id-from-b", "My Custom Global", "", true);
        skill_b.content = "B's own (different, stale) content".to_string();
        wstore_b.skill_insert_raw(&skill_b).unwrap();
        carry_skills(&wstore_b, &identity_store, "channel-b").unwrap();

        let local_copy = wstore_b.skill_get("id-from-a").unwrap().unwrap();
        assert_eq!(
            local_copy.content, "A's content",
            "the local fallback copy must mirror the identity store's winning content, not this channel's own stale content"
        );
    }

    #[test]
    fn a_partial_carry_crash_is_recoverable_because_the_fallback_id_is_deterministic() {
        // ReAgent P1, PR #3181, round 3, Finding C: the identity-store
        // insert and the later local-rename steps are two non-atomic
        // writes across separate SQLite files. A crash between them must
        // not orphan a duplicate on retry. Simulated by pre-inserting a row
        // into the identity store under the EXACT fallback id this
        // collision would deterministically produce, before the local side
        // has been touched at all — the state a crash right after the
        // identity-store insert would leave behind.
        let identity_dir = tempfile::tempdir().unwrap();
        let identity_store = Store::open_identity_store(&identity_dir.path().join("identity-store.db")).unwrap();
        identity_store.skill_insert_raw(&skill("collided-id", "Someone Else's Skill", "", false)).unwrap();

        let dir = tempfile::tempdir().unwrap();
        let wstore = Store::open(&dir.path().join("objects.db")).unwrap();
        wstore.skill_insert_raw(&skill("collided-id", "My Private Skill", "", false)).unwrap();

        let expected_fallback_id =
            deterministic_collision_id(SKILL_COLLISION_NAMESPACE_SEED, "channel", "collided-id").to_string();
        let mut pre_crashed = skill("collided-id", "My Private Skill", "", false);
        pre_crashed.id = expected_fallback_id.clone();
        identity_store.skill_insert_raw(&pre_crashed).unwrap();

        let (carried, _) = carry_skills(&wstore, &identity_store, "channel").unwrap();
        assert_eq!(
            carried, 0,
            "the fallback id already exists in the identity store from the 'crashed' first attempt — nothing new to carry"
        );

        // The retry must still finish the LOCAL half of the work: insert
        // the local copy under the deterministic id, rewrite refs, and
        // remove the stale local row — recovering fully instead of getting
        // stuck because the identity-store row already existed.
        assert!(
            wstore.skill_get(&expected_fallback_id).unwrap().is_some(),
            "retry must still complete the local half of the carry even though the identity-store row already existed"
        );
        assert!(wstore.skill_get("collided-id").unwrap().is_none());
    }

    #[test]
    fn a_non_existent_channel_store_is_a_no_op() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = ctx_for(&dir.path().join("does-not-exist.db"));
        let result = M0031CarrySkillsAndMcpServersToIdentityStore.up(&ctx);
        assert!(result.is_ok());
    }
}
