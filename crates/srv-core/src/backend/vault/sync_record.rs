// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A Vault entry as the body of a sync record (docs/specs/SPEC_VAULT_SHARED_ACROSS_MACHINES_LAN_AND_CLOUD_2026_11_01.md
//! sections 6.1 and 6.6): what goes inside the seal of [`super::sync_seal`], and what a machine that receives one makes
//! of it.
//!
//! The body holds the entry's kind, label, plain fields, tags and secret parts, and when it was made and last changed.
//! It leaves out what is the machine's own: who a login is shared with (the agents of one machine, `shares::SHARED`),
//! when the entry was last used here, and the grants (a table of their own, which never syncs). The values are in clear
//! inside the body, so the body is wiped when dropped and never printed.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

use super::seal::Parts;
use super::shares;
use super::signins::{PERSONAL, PROFILE};
use crate::backend::browser_profiles_store::BrowserProfile;
use crate::backend::storage::vault::VaultRow;

/// The plain fields that name this machine's agents, which a synced entry neither sends nor overwrites.
pub const LOCAL_FIELDS: [&str; 1] = [shares::SHARED];

/// The plain field that names the browser profile a sign-in belongs to, in place of `profile`: a profile's id is made on
/// one machine and means nothing on another, its name is what the person sees on both.
pub const PROFILE_NAME: &str = "profile_name";
/// On a sign-in received for a profile this machine does not have: the name of that profile. The sign-in is shown under
/// Personal, marked with it.
pub const PROFILE_FROM: &str = "profile_from";

/// The person's browser profiles of one machine, by id and by name.
pub trait Profiles {
    /// The name of the person's profile `id`; `None` for an agent's own profile and for one that does not exist.
    fn name_of(&self, id: &str) -> Option<String>;
    /// The id of the person's profile called `name` (the same name in any case), if this machine has one.
    fn id_of(&self, name: &str) -> Option<String>;
}

/// The profiles of this machine, as `browser_profiles_store::read_profiles` lists them.
pub struct Known<'a>(pub &'a [BrowserProfile]);

impl Profiles for Known<'_> {
    fn name_of(&self, id: &str) -> Option<String> {
        self.0.iter().find(|p| p.id == id && p.owner_agent.is_none()).map(|p| p.name.clone())
    }
    fn id_of(&self, name: &str) -> Option<String> {
        self.0.iter().find(|p| p.owner_agent.is_none() && p.name.eq_ignore_ascii_case(name)).map(|p| p.id.clone())
    }
}

/// An entry as a sync record carries it.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct EntryBody {
    pub kind: String,
    pub label: String,
    pub fields: BTreeMap<String, String>,
    pub tags: Vec<String>,
    /// The secret parts by name, empty for a link and for an entry backed by AWS Secrets Manager: such an entry is sent
    /// as the reference only, and each machine that can reads the secret itself.
    pub parts: BTreeMap<String, String>,
    pub created_at: i64,
    pub updated_at: i64,
}

impl Drop for EntryBody {
    fn drop(&mut self) {
        self.parts.values_mut().for_each(|v| v.zeroize());
    }
}

/// What a log may say of a body: its label and which parts it holds, never a value.
impl std::fmt::Debug for EntryBody {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EntryBody").field("kind", &self.kind).field("label", &self.label).field("parts", &self.parts.keys().collect::<Vec<_>>()).finish_non_exhaustive()
    }
}

impl EntryBody {
    /// The body of `row`, whose parts are `parts` (opened with this machine's key by the caller).
    pub fn of_row(row: &VaultRow, parts: &Parts, profiles: &dyn Profiles) -> EntryBody {
        let mut fields: BTreeMap<String, String> = serde_json::from_str(&row.fields).unwrap_or_default();
        for name in LOCAL_FIELDS {
            fields.remove(name);
        }
        // A machine's profile id never leaves it: the profile goes by name (a sign-in that can't be named stays here, see
        // `stays_here`, so none arrives without one).
        let from = fields.remove(PROFILE_FROM);
        if let Some(id) = fields.get(PROFILE).and_then(|p| p.strip_prefix("profile:")).map(str::to_string) {
            fields.remove(PROFILE);
            if let Some(name) = profiles.name_of(&id) {
                fields.insert(PROFILE_NAME.to_string(), name);
            }
        } else if let Some(name) = from {
            // A sign-in that arrived for a profile this machine lacks is still that profile's: it goes on by its name, so
            // an edit made here does not move it to Personal for every machine that has the profile.
            fields.remove(PROFILE);
            fields.insert(PROFILE_NAME.to_string(), name);
        }
        EntryBody {
            kind: row.kind.clone(),
            label: row.label.clone(),
            fields,
            tags: serde_json::from_str(&row.tags).unwrap_or_default(),
            parts: parts.0.clone(),
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }

    /// The row this body makes on this machine, in the Vault `vault_id`. `sealed` is its parts sealed with this
    /// machine's own key (`seal::seal`), and `local` the row it replaces, if there is one: the fields that are this
    /// machine's own and the time of its last use here stay as they were.
    pub fn into_row(&self, id: &str, vault_id: &str, sealed: Vec<u8>, local: Option<&VaultRow>, profiles: &dyn Profiles) -> VaultRow {
        let mut fields = self.fields.clone();
        for name in LOCAL_FIELDS {
            fields.remove(name);
        }
        // The profile by name: the profile of that name here, or Personal marked with the name. A raw profile id in a
        // body is another machine's and is not trusted.
        fields.remove(PROFILE_FROM);
        let named = fields.remove(PROFILE_NAME);
        if fields.get(PROFILE).is_some_and(|p| p != PERSONAL) {
            fields.remove(PROFILE);
        }
        if let Some(name) = named {
            match profiles.id_of(&name) {
                Some(pid) => {
                    fields.insert(PROFILE.to_string(), format!("profile:{pid}"));
                }
                None => {
                    fields.insert(PROFILE.to_string(), PERSONAL.to_string());
                    fields.insert(PROFILE_FROM.to_string(), name);
                }
            }
        }
        let kept: BTreeMap<String, String> = local.map(|r| serde_json::from_str(&r.fields).unwrap_or_default()).unwrap_or_default();
        for name in LOCAL_FIELDS {
            if let Some(own) = kept.get(name) {
                fields.insert(name.to_string(), own.clone());
            }
        }
        VaultRow {
            id: id.to_string(),
            kind: self.kind.clone(),
            label: self.label.clone(),
            fields: serde_json::to_string(&fields).unwrap_or_else(|_| "{}".to_string()),
            tags: serde_json::to_string(&self.tags).unwrap_or_else(|_| "[]".to_string()),
            sealed,
            created_at: self.created_at,
            updated_at: self.updated_at,
            last_used_at: local.map_or(0, |r| r.last_used_at),
            vault_id: vault_id.to_string(),
        }
    }
}

/// Whether `row` is one this machine keeps to itself: a sign-in in the own browsing profile of an agent, or in a profile
/// that has no name here. Such an entry would land under Personal on another machine, where more of the person's tabs may
/// use it than the profile it was saved for, so it does not go.
pub fn stays_here(row: &VaultRow, profiles: &dyn Profiles) -> bool {
    let fields: BTreeMap<String, String> = serde_json::from_str(&row.fields).unwrap_or_default();
    fields.get(PROFILE).and_then(|p| p.strip_prefix("profile:")).is_some_and(|id| profiles.name_of(id).is_none())
}

#[cfg(test)]
#[path = "sync_record_tests.rs"]
mod tests;
