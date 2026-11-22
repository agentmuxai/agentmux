// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! An entry's link ids on several machines. A link id is the opaque name a deep link gives a Vault entry
//! (`agentmux://vault/entry/<link id>`): it says nothing about the entry and each machine gives one on first use. A link
//! made on one machine of a shared Vault has to open the entry on the others, so the ids travel in the entry's sealed body.
//!
//! The build guarantees:
//! - When two machines each gave the same entry an id before they first synced, the entry keeps both: one is its own id,
//!   the other an alias, and a link made with either opens the entry on both machines.
//! - An id is never reassigned: one that names another entry here, or that named an entry that was removed, is not taken
//!   for this one.
//! - The set is small and bounded ([`MAX_LINKS`]) and the same on every machine whatever order the records arrive in.
//! - A machine keeps the id it gave an entry as that entry's own: it never changes under a link it showed.
//!
//! Pure functions over ids: the store tells which entry owns an id and which ids were retired through [`LinkOwners`].

/// The most ids an entry keeps, its own and its aliases together.
pub const MAX_LINKS: usize = 4;

/// The prefix of every link id.
const PREFIX: &str = "e_";

/// Whether `s` has the shape of a link id: the prefix and 16 lowercase letters and digits. Anything else in a received
/// body is dropped.
pub fn is_link_id(s: &str) -> bool {
    s.strip_prefix(PREFIX).is_some_and(|rest| rest.len() == 16 && rest.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit()))
}

/// The link ids of one entry on this machine.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Links {
    /// The id this machine gave the entry, or took from another machine if it had none. It is the one the pane shows.
    pub primary: Option<String>,
    /// The others, in order, none equal to the primary.
    pub aliases: Vec<String>,
}

impl Links {
    /// Every id: the entry's own first, then the aliases. This is what the entry's sync body carries.
    pub fn ids(&self) -> Vec<String> {
        self.primary.iter().chain(self.aliases.iter()).cloned().collect()
    }

    pub fn opens(&self, id: &str) -> bool {
        self.primary.as_deref() == Some(id) || self.aliases.iter().any(|a| a == id)
    }
}

/// What the store knows of link ids, besides this entry's own.
pub trait LinkOwners {
    /// The entry that holds `id` here now, if any.
    fn owner_of(&self, id: &str) -> Option<String>;
    /// Whether `id` once named an entry that has been removed. A removed entry's ids are kept for good for this.
    fn retired(&self, id: &str) -> bool;
}

/// The ids of `entry_id` after a record for it arrived carrying `incoming`: what this machine had, plus the ids it did not
/// have, except those it must not take. An id held by another entry here, or retired, is refused; so is anything that
/// does not have the shape of an id. The result does not depend on the order records arrive in: the aliases are kept in
/// sorted order and cut to the bound.
pub fn settle(owners: &dyn LinkOwners, entry_id: &str, local: &Links, incoming: &[String]) -> Links {
    let had = local.ids();
    let mut taken: Vec<String> = Vec::new();
    for id in incoming {
        if !is_link_id(id) || had.contains(id) || taken.contains(id) {
            continue;
        }
        let held_elsewhere = owners.owner_of(id).is_some_and(|owner| owner != entry_id);
        if held_elsewhere || owners.retired(id) {
            continue;
        }
        taken.push(id.clone());
    }
    // An entry with no id of its own yet takes the first one it was sent, the sender's own.
    let primary = local.primary.clone().or_else(|| (!taken.is_empty()).then(|| taken.remove(0)));
    let mut aliases: Vec<String> = local.aliases.iter().chain(taken.iter()).filter(|a| Some(*a) != primary.as_ref() && is_link_id(a)).cloned().collect();
    aliases.sort();
    aliases.dedup();
    aliases.truncate(MAX_LINKS.saturating_sub(usize::from(primary.is_some())));
    Links { primary, aliases }
}

#[cfg(test)]
#[path = "sync_links_tests.rs"]
mod tests;
