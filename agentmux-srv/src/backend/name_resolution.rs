// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The one place a typed agent name is interpreted (identity M3,
//! `docs/specs/SPEC_AGENT_IDENTITY_CARRIED_NOT_DERIVED_2026_09_23.md` §5).
//!
//! **Callable only from surfaces where a human or a model typed the name** —
//! today the `POST /agentmux/agents/resolve` endpoint the MCP server calls
//! before it enqueues work or schedules a cron job. No internal code path may
//! call this: `scripts/check-name-resolver-callers.sh` is the CI grep gate
//! that enforces it (§9.3), so §2 rule 1 is mechanical rather than a
//! convention that erodes.
//!
//! **Ambiguity is surfaced, never guessed** (§5.2). A name held by two agents
//! is returned as `Ambiguous` with enough about each candidate for a model to
//! retry by UID and for a human to act on. This is deliberately the opposite
//! of `agent_resolve::resolve_agent_id`, which collapses "several" into
//! "none" because its callers are internal and have nobody to ask.
//!
//! **What is consulted, in order.** (1) A typed value that *is* a UID. (2) The
//! live registry, by name — every registered block whose display or stable
//! name matches, with its UID when it has one. (3) The store, by slug and by
//! display name (`name`, `instance_name`, case-insensitively) over
//! non-template, non-hidden rows — the agents that exist but are not running
//! right now. Live and stored candidates that share a UID are one candidate.
//!
//! Spec §1.1 measured that a display name never resolves through the
//! exact-case slug tier. That finding is about a resolver that has to *pick*;
//! this one *lists*, so it can afford to match display names: a single match
//! resolves, a collision is refused with the candidates, and nothing is ever
//! misrouted.

use serde::Serialize;

use crate::backend::reactive::handler::{LookupOutcome, ReactiveHandler};
use crate::backend::storage::store::Store;

/// One agent that a typed name could mean.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct AgentCandidate {
    /// `db_agents.id` when the agent has a row; `None` for a live block with
    /// no row (a quick-launch pane, a PTY shell) — addressable by name only.
    pub uid: Option<String>,
    /// What a human would recognise it by.
    pub name: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub slug: String,
    /// Set when the candidate is registered right now.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_id: Option<String>,
    pub live: bool,
}

/// The answer to "which agent did they mean?" (§5.1).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "resolution", rename_all = "snake_case")]
pub enum NameResolution {
    /// Exactly one agent matches, and it has a UID.
    One {
        uid: String,
        candidate: AgentCandidate,
    },
    /// Several do. Carries enough to ask the human which.
    Ambiguous {
        candidates: Vec<AgentCandidate>,
    },
    /// Exactly one agent matches, but it has no UID (no `db_agents` row):
    /// reachable by name, not storable as an identity. Not in the spec's
    /// three-variant enum — added because such agents exist (M0's finding)
    /// and collapsing them into `None` would tell the caller the agent does
    /// not exist when it does.
    Unidentified {
        candidate: AgentCandidate,
    },
    None,
}

/// Resolve `typed` to a UID, or to the reasons it cannot be resolved.
///
/// `registry` is the live in-memory registry (`get_global_handler()` in
/// production; a fresh `ReactiveHandler` in tests). `store` is the
/// per-channel object store that holds `db_agents`.
pub(crate) fn resolve_name_to_uid(
    store: &Store,
    registry: &ReactiveHandler,
    typed: &str,
) -> NameResolution {
    let typed = typed.trim();
    if typed.is_empty() {
        return NameResolution::None;
    }

    // (1) The typed value is itself a UID.
    if let Ok(Some(def)) = store.agent_def_get(typed) {
        if def.is_seeded == 0 {
            let candidate = AgentCandidate {
                uid: Some(def.id.clone()),
                name: if def.name.is_empty() {
                    def.slug.clone()
                } else {
                    def.name.clone()
                },
                slug: def.slug.clone(),
                block_id: registry.get_agent(typed).map(|r| r.block_id),
                live: registry.get_agent(typed).is_some(),
            };
            return NameResolution::One {
                uid: def.id,
                candidate,
            };
        }
    }

    let mut candidates: Vec<AgentCandidate> = Vec::new();

    // (2) Live registry, by name.
    let live: Vec<crate::backend::reactive::AgentRegistration> =
        match registry.lookup_by_name(typed) {
            LookupOutcome::One(reg) => vec![reg],
            LookupOutcome::Ambiguous(regs) => regs,
            LookupOutcome::NotFound => Vec::new(),
        };
    for reg in live {
        // A registration made before its row existed carries no UID (spec
        // Q1); take the one on its block's row, so the agent and its own row
        // are one candidate — not an ambiguity with itself (review on #3563).
        // Identity by block, not by name.
        let uid = reg
            .uid
            .clone()
            .or_else(|| crate::backend::agent_resolve::uid_for_block(store, &reg.block_id));
        candidates.push(AgentCandidate {
            uid,
            name: reg.agent_id.clone(),
            slug: String::new(),
            block_id: Some(reg.block_id.clone()),
            live: true,
        });
    }

    // (3) The store, by slug and by display name.
    match store.agents_matching_name(typed) {
        Ok(rows) => {
            for row in rows {
                if let Some(existing) = candidates
                    .iter_mut()
                    .find(|c| c.uid.as_deref() == Some(row.id.as_str()))
                {
                    // Same agent, seen live already: enrich, don't duplicate.
                    if existing.slug.is_empty() {
                        existing.slug = row.slug.clone();
                    }
                    continue;
                }
                candidates.push(AgentCandidate {
                    uid: Some(row.id),
                    name: if row.instance_name.is_empty() {
                        row.name
                    } else {
                        row.instance_name
                    },
                    slug: row.slug,
                    block_id: None,
                    live: false,
                });
            }
        }
        Err(e) => {
            // A store fault must not turn into a confident UID from half the
            // evidence: answer `None`, so the caller stores no UID and the
            // row keeps the name path (counted) — degraded, never misrouted.
            tracing::warn!(name = %typed, error = %e, "resolve_name_to_uid: store read failed — not resolving");
            record("resolve.store_error");
            return NameResolution::None;
        }
    }

    match candidates.len() {
        // Only the outcomes that leave a row on the name path are counted
        // (§9.2 reads these as "must reach zero"); `One`, `None` and
        // `Ambiguous` are answers, not fallbacks.
        0 => NameResolution::None,
        1 => {
            let candidate = candidates.remove(0);
            match candidate.uid.clone() {
                Some(uid) => NameResolution::One { uid, candidate },
                None => {
                    record("resolve.unidentified");
                    NameResolution::Unidentified { candidate }
                }
            }
        }
        _ => NameResolution::Ambiguous { candidates },
    }
}

fn record(site: &'static str) {
    crate::backend::agent_resolve::record_uid_fallback(site);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::storage::agents::test_agent_def;

    const UID_Y: &str = "4f3c0000-0000-4000-8000-000000000a91";
    const UID_UP: &str = "9b2e0000-0000-4000-8000-0000000007d4";

    fn store_with(rows: &[(&str, &str, &str)]) -> Store {
        let store = Store::open_in_memory().unwrap();
        for (id, name, slug) in rows {
            let mut def = test_agent_def(id, name, "claude", "agent", 1, "");
            def.slug = slug.to_string();
            store.agent_def_insert(&mut def).unwrap();
        }
        store
    }

    fn registry() -> ReactiveHandler {
        ReactiveHandler::new()
    }

    #[test]
    fn a_uid_resolves_to_itself() {
        let store = store_with(&[(UID_Y, "AgentY", "agenty")]);
        let r = resolve_name_to_uid(&store, &registry(), UID_Y);
        assert!(
            matches!(r, NameResolution::One { ref uid, .. } if uid == UID_Y),
            "{r:?}"
        );
    }

    /// The §1.1 finding, inverted for a resolver that LISTS: a display name
    /// that the exact-case slug tier could never resolve resolves here when
    /// exactly one agent bears it, because there is nothing to misroute.
    #[test]
    fn a_unique_display_name_resolves_to_the_rows_uid() {
        let store = store_with(&[(UID_Y, "AgentY", "agenty")]);
        for typed in ["AgentY", "agenty", "AGENTY"] {
            let r = resolve_name_to_uid(&store, &registry(), typed);
            assert!(
                matches!(r, NameResolution::One { ref uid, .. } if uid == UID_Y),
                "{typed}: {r:?}"
            );
        }
    }

    /// Two agents whose names collide are refused WITH both candidates.
    #[test]
    fn colliding_names_are_ambiguous_with_both_candidates_listed() {
        let store = store_with(&[(UID_Y, "AgentY", "agenty"), (UID_UP, "AGENTY", "agenty-2")]);
        let r = resolve_name_to_uid(&store, &registry(), "agenty");
        let NameResolution::Ambiguous { candidates } = r else {
            panic!("expected Ambiguous, got {r:?}");
        };
        let uids: Vec<&str> = candidates.iter().filter_map(|c| c.uid.as_deref()).collect();
        assert!(
            uids.contains(&UID_Y) && uids.contains(&UID_UP),
            "{candidates:?}"
        );
        // …while the collision-resolved slug alone is exact.
        let r = resolve_name_to_uid(&store, &registry(), "agenty-2");
        assert!(
            matches!(r, NameResolution::One { ref uid, .. } if uid == UID_UP),
            "{r:?}"
        );
    }

    /// A live registration and its stored row are ONE candidate, marked live
    /// with its block — not two.
    #[test]
    fn a_live_agent_and_its_row_merge_into_one_live_candidate() {
        let store = store_with(&[(UID_Y, "AgentY", "agenty")]);
        let reg = registry();
        reg.register_agent_full("AgentY", "block-y", None, 0, None, Some(UID_Y), "test.m3")
            .unwrap();
        let r = resolve_name_to_uid(&store, &reg, "AgentY");
        let NameResolution::One { uid, candidate } = r else {
            panic!("expected One, got {r:?}");
        };
        assert_eq!(uid, UID_Y);
        assert!(candidate.live);
        assert_eq!(candidate.block_id.as_deref(), Some("block-y"));
        assert_eq!(candidate.slug, "agenty");
    }

    fn bind_block(store: &Store, block_id: &str, agent_uid: &str) {
        let mut block = crate::backend::obj::Block {
            oid: block_id.to_string(),
            parentoref: String::new(),
            version: 0,
            runtimeopts: None,
            stickers: None,
            meta: {
                let mut m = crate::backend::obj::MetaMapType::new();
                m.insert("view".to_string(), serde_json::json!("agent"));
                m.insert("agentId".to_string(), serde_json::json!(agent_uid));
                m
            },
            subblockids: None,
        };
        store.insert(&mut block).unwrap();
    }

    /// An agent that registered before its row existed carries no UID (spec
    /// Q1). It must still be ONE candidate with its own row — its UID taken
    /// from the row on its block — not ambiguous with itself (review on
    /// #3563: an agent scheduling cron to itself by name was refused).
    #[test]
    fn a_live_agent_registered_without_a_uid_merges_with_its_own_row_by_block() {
        let store = store_with(&[(UID_Y, "AgentY", "agenty")]);
        bind_block(&store, "block-y", UID_Y);
        let reg = registry();
        reg.register_agent_full("AgentY", "block-y", None, 0, None, None, "test.m3")
            .unwrap();
        let r = resolve_name_to_uid(&store, &reg, "AgentY");
        let NameResolution::One { uid, candidate } = r else {
            panic!("expected One, got {r:?}");
        };
        assert_eq!(uid, UID_Y);
        assert!(candidate.live);
        assert_eq!(candidate.block_id.as_deref(), Some("block-y"));
    }

    /// …but a uid-less live block that is NOT that row's block (a
    /// quick-launch pane that happens to share the name) stays a separate
    /// candidate: the merge is by block identity, never by name.
    #[test]
    fn a_same_named_pane_on_another_block_is_still_ambiguous_with_the_row() {
        let store = store_with(&[(UID_Y, "AgentY", "agenty")]);
        let reg = registry();
        reg.register_agent_full("AgentY", "block-pane", None, 0, None, None, "test.m3")
            .unwrap();
        let r = resolve_name_to_uid(&store, &reg, "AgentY");
        let NameResolution::Ambiguous { candidates } = r else {
            panic!("expected Ambiguous, got {r:?}");
        };
        assert_eq!(candidates.len(), 2, "{candidates:?}");
    }

    /// A live block with no row (quick-launch pane) is `Unidentified`: it
    /// exists and is reachable by name, but has no UID to store.
    #[test]
    fn a_live_block_without_a_row_is_unidentified_not_none() {
        let store = store_with(&[]);
        let reg = registry();
        reg.register_agent_full("Scratch", "block-s", None, 0, None, None, "test.m3")
            .unwrap();
        let r = resolve_name_to_uid(&store, &reg, "scratch");
        assert!(
            matches!(r, NameResolution::Unidentified { ref candidate } if candidate.block_id.as_deref() == Some("block-s")),
            "{r:?}"
        );
    }

    #[test]
    fn unknown_and_empty_names_are_none() {
        let store = store_with(&[(UID_Y, "AgentY", "agenty")]);
        assert_eq!(
            resolve_name_to_uid(&store, &registry(), "nobody"),
            NameResolution::None
        );
        assert_eq!(
            resolve_name_to_uid(&store, &registry(), "   "),
            NameResolution::None
        );
    }

    /// Templates are prototypes, never targets (spec §1.2): a provider key
    /// typed as a name must not resolve to the template row.
    #[test]
    fn a_template_row_is_not_a_candidate() {
        let store = Store::open_in_memory().unwrap();
        let mut tpl = test_agent_def("claude", "Claude", "claude", "agent", 1, "");
        tpl.is_seeded = 1;
        store.agent_def_insert(&mut tpl).unwrap();
        assert_eq!(
            resolve_name_to_uid(&store, &registry(), "claude"),
            NameResolution::None
        );
    }
}
