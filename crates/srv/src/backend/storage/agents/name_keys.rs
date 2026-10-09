// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Slugs and the name-keyed rows (`KEY_NAME`s) an agent's names map to:
//! collision-free slugs, tombstoning a renamed agent's keys, and finding
//! agents by name.

use super::*;

/// One row `agents_matching_name` found (identity M3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentNameMatch {
    pub id: String,
    pub name: String,
    pub slug: String,
    pub instance_name: String,
}

/// The first of `base`, `base-2`, `base-3`, … that no `db_agents` row holds.
///
/// Shared by `agent_def_insert_local_only` and `instance_create` so the two
/// write paths cannot drift: `db_agents.slug` carries no `UNIQUE` constraint
/// (deliberately — see `migrations.rs`), so uniqueness is only ever as good as
/// the agreement between every path that inserts one. It held in only one of
/// them until #3497 §4.
///
/// Callers must already hold the connection lock — the scan and the INSERT
/// have to be one critical section, or two concurrent inserts race to the
/// same candidate.
///
/// There is deliberately no "exclude this row id" option. It looks necessary
/// for `instance_create`'s `ON CONFLICT(id) DO UPDATE` path — a row
/// re-resolving its own slug would otherwise collide with itself — but that
/// clause does not assign `slug` at all, so on the conflict path the computed
/// value is discarded and an exclusion could never be observed (ReAgent P2 on
/// #3514). On the insert path the row does not exist yet, so there is nothing
/// to exclude either. If `slug` is ever added to that `DO UPDATE SET`, this
/// becomes real and needs revisiting.
pub(super) fn resolve_slug_collision(conn: &rusqlite::Connection, base: &str) -> rusqlite::Result<String> {
    let mut candidate = base.to_string();
    let mut n: u32 = 2;
    loop {
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM db_agents WHERE slug = ?1",
            params![candidate],
            |row| row.get(0),
        )?;
        if count == 0 {
            return Ok(candidate);
        }
        candidate = format!("{base}-{n}");
        n += 1;
    }
}

/// Every table keyed to an agent id that a deleted `db_agents` row leaves
/// behind, purged explicitly (docs/specs/SPEC_AGENT_DELETE_2026_09_16.md
/// §5.1). Called with the caller's connection lock already held, inside the
/// same critical section as the `db_agents` DELETE. Returns rows removed.
///
/// Six of these declare `ON DELETE CASCADE` on `db_agents(id)` and do fire
/// on their own — `configure_and_migrate` sets `PRAGMA foreign_keys=ON` on
/// the production connection, contrary to §5.1's premise (and to
/// `managed.rs`'s "only ever set in tests" comment, which is stale for this
/// store). They are listed anyway: the rest carry no FK at all —
/// `db_agent_credentials` and the two signing-key tables hold live secret
/// material, so leaving them behind is a security-hygiene gap, not just
/// clutter — and one obviously-complete list beats two half-lists where a
/// reader has to know which half a new table belongs in to tell whether it
/// was handled.
///
/// **Runs against either physical database.** Four of these
/// (`db_agent_identity_links`, `db_agent_credentials`,
/// `db_agent_native_memory`, `db_agent_native_memory_versions`) are created
/// by BOTH `run_object_schema` and `run_identity_store_schema`, and per
/// `SPEC_IDENTITY_STORE_SPLIT_2026_08_17.md` the identity store holds the
/// live rows — `listrecentsessions` reads links from
/// `identity_store.agent_identity_list_all()`, not from the object store's
/// same-named table. Purging only the object store would leave an agent's
/// real credentials and account links on disk after a delete that promises
/// to remove them. Tables absent from whichever connection this runs on are
/// skipped, so one list serves both.
///
/// `db_work_queue.target_agent` is deliberately absent: releasing work
/// claimed against a deleted agent is a state change, not a row to drop —
/// open question §8.3 of the spec above.
/// Record the lowercased names `id` is known by — the keys its legacy
/// name-keyed signing keys may sit under — in `db_agent_key_tombstones`,
/// before its row is deleted (identity M4a, spec §6.5.4). A template row is
/// skipped: templates are never deleted through here as agents, and their
/// names are shared by design. Idempotent; a missing row records nothing.
pub(super) fn tombstone_key_names(conn: &rusqlite::Connection, id: &str) -> Result<(), StoreError> {
    let names: Option<(String, String, String)> = conn
        .query_row(
            "SELECT slug, name, COALESCE(instance_name, '') FROM db_agents WHERE id = ?1 AND is_template = 0",
            params![id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            e => Err(e),
        })?;
    let Some((slug, name, instance_name)) = names else {
        return Ok(());
    };
    let now = agentmux_common::time::now_secs();
    // The same names, and the same "another agent may sign under it" rule,
    // as the key purge (M4d-1): a tombstone records a name whose keys were
    // the dead agent's, so it must never be planted against a live agent's
    // (ReAgent on #3633). Folded as the key tables fold (`to_lowercase`);
    // compared in Rust, never with SQLite's ASCII-only `lower()`. Deleting
    // the second "AgentY" must not tombstone the first one's `agenty`
    // (adversarial review of #3571).
    let mut candidates = key_names_of(&slug, &name, &instance_name);
    for former in former_names(conn, id)? {
        candidates.extend(key_names_of("", &former, ""));
    }
    for n in candidates {
        let n = n.trim().to_string();
        if n.is_empty() || key_name_used_by_another(conn, id, &n)? {
            continue;
        }
        conn.execute(
            "INSERT OR REPLACE INTO db_agent_key_tombstones (name, deleted_uid, deleted_at) VALUES (?1, ?2, ?3)",
            params![n, id, now],
        )?;
    }
    Ok(())
}

/// Delete the signing keys filed under every name `id` may have signed as —
/// its jekt HMAC key and its LAN and WAN keypairs, which are keyed by name,
/// not UID. M4a-3 (spec §6.5.4) deleted them under the slug; M4d-1
/// (§6.5.10) also under the display name, `instance_name` and the fallback
/// ids ([`key_names_of`]). Without this a later agent that takes a name is
/// handed the dead agent's keys by `ensure`'s `INSERT OR IGNORE`, and signs
/// as it. Read before the row is deleted; a name another row may sign under
/// ([`key_name_used_by_another`]) is that agent's key and is left alone.
/// Folded as the key tables fold (`to_lowercase`). A template row is
/// skipped, as in [`tombstone_key_names`]. Idempotent.
///
/// W3-S (`SPEC_WAN_JEKT_VERIFICATION_2026_09_24.md` §2.2): agent WAN keys now
/// live in the channel-wide `wan.db`, so the same names are deleted there
/// too — otherwise a new agent of the name would inherit the key across an
/// upgrade. A `wan.db` failure fails the delete rather than being skipped:
/// every statement here is idempotent, so a retry converges, whereas a skipped
/// purge would hand the key to the next agent of that name. Lock order is
/// `objects.db` (held by the caller) → `wan.db`, as `wan_identity.rs`
/// documents.
pub(super) fn purge_name_keyed_keys(
    conn: &rusqlite::Connection,
    id: &str,
    wan: Option<&super::super::wan_identity::WanIdentityStore>,
) -> Result<usize, StoreError> {
    let names: Option<(String, String, String)> = conn
        .query_row(
            "SELECT slug, name, COALESCE(instance_name, '') FROM db_agents WHERE id = ?1 AND is_template = 0",
            params![id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            e => Err(e),
        })?;
    let Some((slug, name, instance_name)) = names else {
        return Ok(0);
    };
    let mut removed = 0;
    let mut candidates = key_names_of(&slug, &name, &instance_name);
    for former in former_names(conn, id)? {
        for n in key_names_of("", &former, "") {
            if !candidates.contains(&n) {
                candidates.push(n);
            }
        }
    }
    let mut purged_names = Vec::new();
    for key_name in candidates {
        if key_name_used_by_another(conn, id, &key_name)? {
            continue;
        }
        removed += delete_key_rows(conn, &key_name)?;
        purged_names.push(key_name);
    }
    if let Some(wan) = wan {
        removed += wan.agent_keys_delete(&purged_names)?;
    }
    if removed > 0 {
        tracing::info!(
            agent = id,
            removed,
            "identity M4d-1: deleted the dead agent's name-keyed signing keys"
        );
    }
    Ok(removed)
}

/// Every name a key row could be filed under for this agent, folded as the
/// key tables fold (`to_lowercase`) — identity M4d-1 (spec §6.5.10). The
/// slug exactly as sent (M4a-3); the display name and `instance_name`,
/// trimmed, as the tombstone records them; and `agent.open`'s fallback id,
/// the display name lowercased with non-alphanumerics made `-` (used when a
/// definition has no slug: `Zed Bot` is keyed `zed-bot`).
pub(super) fn key_names_of(slug: &str, name: &str, instance_name: &str) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    let mut push = |n: String| {
        if !n.trim().is_empty() && !names.contains(&n) {
            names.push(n);
        }
    };
    push(slug.to_lowercase());
    push(name.trim().to_lowercase());
    push(instance_name.trim().to_lowercase());
    // Both fallbacks of the display name and of `instance_name`: an agent
    // renamed after its first launch keeps its launch name there, and that
    // name's fallback may still key its signing material (Codex P1 on #3633).
    for n in [name, instance_name] {
        if !n.trim().is_empty() {
            push(agent_open_fallback_id(n));
            push(frontend_fallback_id(n));
        }
    }
    names
}

/// `agent.open`'s id for a definition with no slug (`agent_open.rs`).
pub(super) fn agent_open_fallback_id(name: &str) -> String {
    agentmux_common::slug::path_slug(name)
}

/// The frontend's id for a launch with no slug — a template-created agent's
/// first session. ASCII only, one dash per UTF-16 code unit
/// (`agentmux_common::slug::js_ascii_slug`, Codex P1 on #3633).
pub(super) fn frontend_fallback_id(name: &str) -> String {
    agentmux_common::slug::js_ascii_slug(name)
}

/// Every display / instance name `id` has had before its current ones
/// (`db_agent_former_names`, recorded by trigger on every rename path).
pub(super) fn former_names(conn: &rusqlite::Connection, id: &str) -> Result<Vec<String>, StoreError> {
    let mut stmt = conn.prepare("SELECT name FROM db_agent_former_names WHERE agent_id = ?1")?;
    let rows = stmt.query_map(params![id], |r| r.get::<_, String>(0))?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

/// Whether another row may sign under `folded`: holds it as its slug, or its
/// display name or `instance_name` gives that fallback id (`agent.open`'s or
/// the frontend's).
/// Any row, whatever its slug: a template-created agent's first session
/// signs under the fallback id although its row has a derived slug (review
/// of #3633), so two same-named agents share that key. Such a key is left.
/// **Templates count**: `agent.open` of a template and template-based
/// continuations sign under the template's slug. Folded as the key tables
/// fold (`to_lowercase`), not SQLite's ASCII-only `lower()`: slugs `Ä` and
/// `ä` share the key filed under `ä` (Codex P2 on #3575).
pub(super) fn key_name_used_by_another(
    conn: &rusqlite::Connection,
    id: &str,
    folded: &str,
) -> Result<bool, StoreError> {
    let mut stmt =
        conn.prepare("SELECT slug, name, COALESCE(instance_name, '') FROM db_agents WHERE id != ?1")?;
    let rows = stmt.query_map(params![id], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?))
    })?;
    for row in rows {
        let (slug, name, instance_name) = row?;
        let fallback_of = |n: &str| {
            !n.trim().is_empty() && (agent_open_fallback_id(n) == folded || frontend_fallback_id(n) == folded)
        };
        if slug.to_lowercase() == folded || fallback_of(&name) || fallback_of(&instance_name) {
            return Ok(true);
        }
    }
    // A live agent's former names too: its running session may still sign
    // under an earlier name's fallback until it relaunches.
    let mut stmt = conn.prepare("SELECT name FROM db_agent_former_names WHERE agent_id != ?1")?;
    let formers = stmt.query_map(params![id], |r| r.get::<_, String>(0))?;
    for n in formers {
        let n = n?;
        if n.trim().to_lowercase() == folded
            || agent_open_fallback_id(&n) == folded
            || frontend_fallback_id(&n) == folded
        {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Delete one name's rows from the three name-keyed key tables.
pub(super) fn delete_key_rows(conn: &rusqlite::Connection, name: &str) -> Result<usize, StoreError> {
    let mut removed = 0;
    for table in [
        "db_agent_jekt_keys",
        "db_agent_lan_keys",
        "db_agent_wan_keys",
    ] {
        let present: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
            params![table],
            |r| r.get(0),
        )?;
        if present {
            // Table names are the constants above, never caller input.
            removed += conn.execute(
                &format!("DELETE FROM {table} WHERE agent_id = ?1"),
                params![name],
            )?;
        }
    }
    Ok(removed)
}

/// Tombstoned names, for tests (M4d is the production reader).
#[cfg(test)]
pub(crate) fn key_tombstones_for_tests(conn: &rusqlite::Connection) -> Vec<(String, String)> {
    let mut stmt = conn
        .prepare("SELECT name, deleted_uid FROM db_agent_key_tombstones ORDER BY name")
        .unwrap();
    stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}
