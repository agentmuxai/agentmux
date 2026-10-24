// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The plan's report of slugs more than one agent holds (two uids, one
//! slug). New agents can't get one any more (`HostSlugs` and the shared
//! definitions are checked under the slug allocation lock), but builds before
//! that left some: an agent made in each of two channels picked the same slug,
//! unique in its own `objects.db`, and both were published. Reported, with
//! the id a rename could give each holder after the oldest; nothing here
//! renames anything.

use std::collections::{BTreeMap, HashSet};

use super::home::{fold, has_agent_slugs, has_column, open_ro, place, Home, Install};
use super::plan::{holder_of, Def};
use crate::backend::rpc_types::{ForkSlugDuplicate, ForkSlugDuplicateHolder};
use agentmux_common::slug::is_agent_id;

/// Every slug held by more than one agent: each agent's slug is its shared
/// definition's (the one every reader gives), else its row's for an agent
/// never published. With `read_stores`, every `objects.db` is read for those
/// rows and each later holder gets a proposed id, checked by `holder_of`
/// against everything a run checks and against `taken` (ids this plan already
/// gives); without it, only the shared definitions are compared.
pub(super) fn duplicates(
    home: &Home,
    installs: &[Install],
    defs: &[Def],
    taken: &BTreeMap<String, String>,
    read_stores: bool,
) -> Vec<ForkSlugDuplicate> {
    let mut by_slug: BTreeMap<String, Vec<ForkSlugDuplicateHolder>> = BTreeMap::new();
    let mut add = |slug: &str, holder: ForkSlugDuplicateHolder| {
        let holders = by_slug.entry(fold(slug)).or_default();
        if !holders.iter().any(|h| h.uid == holder.uid) {
            holders.push(holder);
        }
    };
    for d in defs.iter().filter(|d| !d.slug.trim().is_empty()) {
        add(&d.slug, holder(&d.uid, &d.name, d.created_at, "the shared definitions"));
    }
    if read_stores {
        let published: HashSet<&str> = defs.iter().map(|d| d.uid.as_str()).collect();
        let retired = home.definitions().join("retired");
        for install in installs {
            for version in &install.versions {
                for (uid, name, slug, created_at) in unpublished_rows(&version.objects_db()) {
                    // A row of a deleted agent another channel kept isn't an agent.
                    if published.contains(uid.as_str()) || retired.join(format!("{uid}.json")).exists() {
                        continue;
                    }
                    add(&slug, holder(&uid, &name, created_at, &place(install, version)));
                }
            }
        }
    }
    let mut taken = taken.clone();
    let mut out = Vec::new();
    for (slug, mut holders) in by_slug.into_iter().filter(|(_, h)| h.len() > 1) {
        holders.sort_by(|a, b| a.created_at.cmp(&b.created_at).then_with(|| a.uid.cmp(&b.uid)));
        if read_stores {
            for h in holders.iter_mut().skip(1) {
                h.proposed = propose(home, installs, defs, &taken, &h.uid, &slug);
                if let Some(p) = &h.proposed {
                    taken.insert(fold(p), h.uid.clone());
                }
            }
        }
        out.push(ForkSlugDuplicate { slug, holders });
    }
    out
}

fn holder(uid: &str, name: &str, created_at: i64, place: &str) -> ForkSlugDuplicateHolder {
    ForkSlugDuplicateHolder {
        uid: uid.to_string(),
        name: name.to_string(),
        created_at,
        place: place.to_string(),
        proposed: None,
    }
}

/// `(uid, name, slug, created_at)` of every row with a slug in `db`. A store
/// that can't be read gives none: the plan already blocks on it.
fn unpublished_rows(db: &std::path::Path) -> Vec<(String, String, String, i64)> {
    let Ok(conn) = open_ro(db) else { return Vec::new() };
    if !has_agent_slugs(&conn) {
        return Vec::new();
    }
    let col = |c: &str, default: &str| if has_column(&conn, "db_agents", c) { c.to_string() } else { default.to_string() };
    let sql = format!(
        "SELECT id, COALESCE({}, ''), slug, COALESCE({}, 0) FROM db_agents WHERE slug IS NOT NULL AND trim(slug) <> ''",
        col("name", "''"),
        col("created_at", "0"),
    );
    let Ok(mut stmt) = conn.prepare(&sql) else { return Vec::new() };
    let Ok(rows) = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))) else { return Vec::new() };
    rows.flatten().collect()
}

/// The first free id after `slug` for `uid`: `claude-2` → `claude-3`,
/// `forklead` → `forklead-2`.
fn propose(
    home: &Home,
    installs: &[Install],
    defs: &[Def],
    taken: &BTreeMap<String, String>,
    uid: &str,
    slug: &str,
) -> Option<String> {
    let (stem, first) = match slug.rsplit_once('-') {
        Some((stem, n)) if !stem.is_empty() && !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) => {
            (stem, n.parse::<u32>().ok()?.checked_add(1)?)
        }
        _ => (slug, 2),
    };
    (first..first + 100)
        .map(|n| format!("{stem}-{n}"))
        .filter(|c| is_agent_id(c))
        .find(|c| matches!(holder_of(home, installs, defs, taken, uid, c), Ok(None)))
}
