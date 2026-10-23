// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The display names an agent was renamed from, as addresses that still reach
//! it: the former slugs' rules (`former_slugs`), applied to the names
//! `db_agent_former_names` holds. Three triggers on `db_agents` write it, so no
//! rename path can miss one: `trg_agents_former_name` (a root's `name`) and
//! `trg_agents_former_instance_name` (`instance_name`), both in
//! `run_object_schema`, and `trg_agents_former_branch_label` (a fork's
//! `branch_label`, `FORMER_BRANCH_LABEL_TRIGGER`). A display name is addressed
//! by its route name (`agent_route_name`: "Loap #3" is `Loap-3`), so that is
//! the key here, folded.
//!
//! One resolver answers for both kinds, [`Store::former_name_owner`]: a name
//! resolves when exactly one agent lists it, as a former slug or as a former
//! display name, and no agent holds it now as its slug or as a display name's
//! route. A current name always wins, its own agent's included: an agent
//! renamed back to an old name answers to it as a current name.
//!
//! **Only routing reads these**, as for former slugs: nothing signs or is
//! served keys under one, and a sender claiming one is counted unverified
//! (`refuse_former_slug_sender`). The generic lookups that authenticate and
//! act as an agent never ask.
//!
//! Former display names are this channel's only: the shared definitions carry
//! former slugs, not former display names.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::time::Instant;

use super::*;

/// The key a display name is addressed by: its route name, folded. `None`
/// for a name no message can address (blank, or with an `@`).
pub(super) fn route_key(name: &str) -> Option<String> {
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    let route = agentmux_common::slug::agent_route_name(name);
    agentmux_common::slug::is_agent_id(&route).then(|| route.to_lowercase())
}

fn fold(name: &str) -> String {
    name.trim().to_lowercase()
}

/// The names every agent holds now and every former name with the agents that
/// list it, read in one pass.
pub(super) struct FormerNameIndex {
    /// Every slug and display route an agent holds now, folded: this
    /// channel's rows, and every shared definition's (an agent made in
    /// another channel may have no row here).
    current: HashSet<String>,
    /// Former name key → every uid that lists it, slug or display.
    owners: HashMap<String, BTreeSet<String>>,
    /// `(uid, key, recorded_at)` for each `db_agent_former_names` row (a
    /// former `name`, `instance_name` or `branch_label`, one trigger each).
    display: Vec<(String, String, i64)>,
}

impl FormerNameIndex {
    pub(super) fn read(conn: &rusqlite::Connection, shared: &SharedSlugs) -> Result<Self, StoreError> {
        let mut current: HashSet<String> = shared.entries().map(|(_, n)| fold(&n.slug)).collect();
        current.extend(shared.entries().flat_map(|(_, n)| n.display.iter().filter_map(|d| route_key(d))));
        let mut stmt = conn.prepare(
            "SELECT id, COALESCE(slug, ''), name, COALESCE(instance_name, ''), COALESCE(branch_label, '') FROM db_agents",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?, r.get::<_, String>(4)?))
        })?;
        for row in rows {
            let (id, slug, name, instance_name, label) = row?;
            current.insert(fold(&canonical_slug(&id, &slug, shared.slug(&id))));
            current.extend([name, instance_name, label].iter().filter_map(|n| route_key(n)));
        }
        current.remove("");

        let mut owners: HashMap<String, BTreeSet<String>> = HashMap::new();
        for (id, names) in shared.entries() {
            for f in &names.former {
                owners.entry(fold(f)).or_default().insert(id.to_string());
            }
        }
        let mut stmt = conn.prepare("SELECT agent_uid, slug FROM db_agent_former_slugs")?;
        for row in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
            let (uid, slug) = row?;
            owners.entry(fold(&slug)).or_default().insert(uid);
        }
        let mut display = Vec::new();
        let mut stmt = conn.prepare("SELECT agent_id, name, recorded_at FROM db_agent_former_names")?;
        for row in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?)))? {
            let (uid, name, at) = row?;
            if let Some(key) = route_key(&name) {
                owners.entry(key.clone()).or_default().insert(uid.clone());
                display.push((uid, key, at));
            }
        }
        owners.remove("");
        Ok(Self { current, owners, display })
    }

    /// The one agent `key` is a former name of: not held now, and listed by
    /// exactly one agent (several are logged and resolve none).
    pub(super) fn owner(&self, key: &str) -> Option<&str> {
        if key.is_empty() || self.current.contains(key) {
            return None;
        }
        let uids = self.owners.get(key)?;
        if uids.len() > 1 {
            tracing::warn!(name = key, owners = ?uids, "former name listed by several agents; resolving none");
            return None;
        }
        uids.iter().next().map(String::as_str)
    }

    /// Every uid that lists `key` as a former name, held now or not.
    fn listers(&self, key: &str) -> Option<&BTreeSet<String>> {
        self.owners.get(key)
    }

    /// The former display names that aren't held now: [`FormerDisplaySet`]'s
    /// contents.
    fn display_keys(&self) -> HashSet<String> {
        self.display.iter().map(|(_, k, _)| k.clone()).filter(|k| !self.current.contains(k)).collect()
    }
}

/// Whether another agent than `own_id` lists `folded` as a former display
/// name: taken for a new slug (`resolve_slug_collision`).
pub(super) fn former_display_name_of_another(
    conn: &rusqlite::Connection,
    folded: &str,
    own_id: &str,
) -> rusqlite::Result<bool> {
    let mut stmt = conn.prepare("SELECT name FROM db_agent_former_names WHERE agent_id != ?1")?;
    let names = stmt.query_map(params![own_id], |r| r.get::<_, String>(0))?;
    for name in names {
        if route_key(&name?).as_deref() == Some(folded) {
            return Ok(true);
        }
    }
    Ok(false)
}

impl Store {
    /// Whether `key` may be a former display name not held now: in the set,
    /// kept for [`super::former_slugs`]' `KNOWN_FOR` and cleared by a rename
    /// through this store. A read failure answers "maybe", so the full lookup
    /// decides.
    fn former_display_known(&self, key: &str) -> bool {
        if let Some((at, keys)) = self.former_slugs.display.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            if at.elapsed() < super::former_slugs::KNOWN_FOR {
                return keys.contains(key);
            }
        }
        self.former_slugs.read();
        let shared = self.shared_slugs();
        let index = {
            let conn = self.conn.lock().unwrap();
            FormerNameIndex::read(&conn, &shared)
        };
        let Ok(index) = index else {
            return true;
        };
        let keys = index.display_keys();
        let hit = keys.contains(key);
        *self.former_slugs.display.lock().unwrap_or_else(|e| e.into_inner()) = Some((Instant::now(), keys));
        hit
    }

    /// The key `name` is looked up under, when it may be a former name of
    /// either kind; `None` costs no store access once the sets are warm.
    pub(super) fn former_name_candidate(&self, name: &str) -> Option<String> {
        let key = route_key(name)?;
        (self.former_slug_known(&key) || self.former_display_known(&key)).then_some(key)
    }

    /// The agent `name` is a former slug or former display name of, with its
    /// current slug (see the module doc). For routing only: a delivery target,
    /// never a name that authenticates or acts.
    pub fn former_name_owner(&self, name: &str) -> Result<Option<FormerSlugOwner>, StoreError> {
        let Some(key) = self.former_name_candidate(name) else {
            return Ok(None);
        };
        self.former_slugs.read();
        let shared = self.shared_slugs();
        let uid = {
            let conn = self.conn.lock().unwrap();
            FormerNameIndex::read(&conn, &shared)?.owner(&key).map(str::to_string)
        };
        let Some(uid) = uid else {
            return Ok(None);
        };
        let slug = match self.agent_names_by_id(&uid)? {
            Some(row) => Some(row.slug),
            None => shared.slug(&uid).map(str::to_string),
        };
        Ok(slug.filter(|s| !s.trim().is_empty()).map(|slug| FormerSlugOwner { uid, slug }))
    }

    /// The agents that list `name`'s route as a former slug or a former
    /// display name, whether or not anyone holds it now: a name kept for
    /// them, which no other agent takes.
    pub fn former_name_listers(&self, name: &str) -> Result<BTreeSet<String>, StoreError> {
        let Some(key) = route_key(name) else {
            return Ok(BTreeSet::new());
        };
        let shared = self.shared_slugs();
        let conn = self.conn.lock().unwrap();
        Ok(FormerNameIndex::read(&conn, &shared)?.listers(&key).cloned().unwrap_or_default())
    }

    /// Whether `name`'s route is any agent's former name: a former slug
    /// ([`Self::former_slug_reserved`]) or a former display name. A read
    /// failure counts as reserved, as for former slugs.
    pub fn former_name_reserved(&self, name: &str) -> bool {
        let Some(key) = route_key(name) else {
            return false;
        };
        if self.former_slug_reserved(&key) {
            return true;
        }
        let conn = self.conn.lock().unwrap();
        former_display_name_of_another(&conn, &key, "").unwrap_or(true)
    }

    /// Every former display name that resolves to its agent now
    /// ([`Self::former_name_owner`]'s rule), as `(uid, key, recorded_at)`,
    /// the latest time per name. For the relay's grace subscription
    /// (`live_names`), alongside [`Self::agent_former_slugs_all`].
    pub fn agent_former_display_names_routable(&self) -> Result<Vec<(String, String, i64)>, StoreError> {
        let shared = self.shared_slugs();
        let conn = self.conn.lock().unwrap();
        let index = FormerNameIndex::read(&conn, &shared)?;
        let mut latest: std::collections::BTreeMap<(String, String), i64> = Default::default();
        for (uid, key, at) in &index.display {
            if index.owner(key) == Some(uid.as_str()) {
                let t = latest.entry((uid.clone(), key.clone())).or_insert(*at);
                *t = (*t).max(*at);
            }
        }
        Ok(latest.into_iter().map(|((uid, key), at)| (uid, key, at)).collect())
    }

    /// A rename through this store may have retired a display name: forget
    /// the former-name sets so the next lookup sees it.
    pub(super) fn former_names_changed(&self) {
        self.former_slugs.invalidate();
    }
}

#[cfg(test)]
#[path = "former_names_tests.rs"]
mod tests;
