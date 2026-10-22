// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Export the Vault to a password-protected KeePass database (`.kdbx`, KDBX 4)
//! (docs/specs/SPEC_VAULT_PANE_AGENTS_USE_SECRETS_UNSEEN_2026_10_19.md §8.3), which the
//! person can open in KeePassXC, KeePassDX or Strongbox. The file is for them to take
//! elsewhere, so it is as plain a KeePass database as can be made:
//! - a login is a login (title, user name, password, address, notes, and its one-time-code
//!   seed in the `otp` field KeePassXC reads);
//! - a note is an entry whose notes are its text;
//! - every other kind is an entry whose secret is a protected custom field named for it
//!   (`SSN`, `Card number`, `API key`, …: [`kdbx_names`]), with plain details as unprotected
//!   custom fields and the entry's tags as KeePass tags and as groups (a tag `A/B` is the group
//!   `A` > `B`, the first tag choosing the group).
//!
//! The importer reads the same names, so a database leaves and comes back as the same entries.

use std::collections::BTreeMap;
use std::path::Path;

use keepass::config::{DatabaseConfig, KdfConfig};
use keepass::db::{GroupId, Value};
use keepass::{Database, DatabaseKey};

use super::import::Incoming;
use super::kdbx_names::{self, COMPANIONS, DETAILS};
use super::transfer::{gather, wipe, write_atomically, ExportResult, MIN_PASSPHRASE};
use super::{record, PERSON};
use crate::backend::storage::store::Store;

/// What a database written for another app uses to stretch its password: KeePassXC's own default
/// (Argon2d, 64 MiB, 10 iterations), well inside what the importer accepts (§8.7).
fn default_kdf() -> KdfConfig {
    KdfConfig::Argon2 { iterations: 10, memory: 64 * 1024 * 1024, parallelism: 2, version: argon2::Version::Version13 }
}

/// Writes the chosen entries (all when `ids` is `None`) to `path` as a KeePass database opened by
/// `passphrase`.
pub fn export(store: &Store, ids: Option<&[String]>, passphrase: &str, path: &Path) -> Result<ExportResult, String> {
    export_with(store, ids, passphrase, path, default_kdf())
}

pub(super) fn export_with(store: &Store, ids: Option<&[String]>, passphrase: &str, path: &Path, kdf: KdfConfig) -> Result<ExportResult, String> {
    if passphrase.chars().count() < MIN_PASSPHRASE {
        return Err(format!("a passphrase is at least {MIN_PASSPHRASE} characters"));
    }
    let entries = gather(store, ids)?;
    let count = entries.len();
    let bytes = build(&entries, passphrase, kdf);
    wipe(entries);
    write_atomically(path, &bytes?)?;
    let file_name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    record(store, "", "", PERSON, "exported", &file_name, &format!("{count} entries to KeePass"));
    Ok(ExportResult { count, file_name })
}

/// The database's bytes.
pub(super) fn build(entries: &[Incoming], passphrase: &str, kdf: KdfConfig) -> Result<Vec<u8>, String> {
    let mut config = DatabaseConfig::default();
    config.kdf_config = kdf;
    let mut db = Database::with_config(config);
    let mut groups: BTreeMap<String, GroupId> = BTreeMap::new();
    for e in entries {
        let group = group_for(&mut db, &mut groups, e.tags.first().map(String::as_str).unwrap_or(""));
        let Some(mut g) = db.group_mut(group) else { return Err("couldn't build the KeePass database".into()) };
        g.add_entry().edit(|entry| fill(entry, e));
    }
    let mut out = Vec::new();
    db.save(&mut out, DatabaseKey::new().with_password(passphrase)).map_err(|e| format!("couldn't write the KeePass database: {e}"))?;
    Ok(out)
}

/// The group a tag path names, made on first use; the root for no tag.
fn group_for(db: &mut Database, made: &mut BTreeMap<String, GroupId>, tag: &str) -> GroupId {
    let mut at = db.root().id();
    let mut so_far = String::new();
    for name in tag.split('/').map(str::trim).filter(|n| !n.is_empty()) {
        if !so_far.is_empty() {
            so_far.push('/');
        }
        so_far.push_str(name);
        at = match made.get(&so_far) {
            Some(id) => *id,
            None => {
                let id = match db.group_mut(at) {
                    Some(mut parent) => parent.add_group().edit(|g| g.name = name.to_string()).id(),
                    None => at,
                };
                made.insert(so_far.clone(), id);
                id
            }
        };
    }
    at
}

fn fill(entry: &mut keepass::db::Entry, e: &Incoming) {
    let part = |name: &str| e.parts.get(name).map(String::as_str).filter(|v| !v.is_empty());
    let field = |name: &str| e.fields.get(name).map(String::as_str).filter(|v| !v.is_empty());
    entry.set_unprotected("Title", e.label.clone());
    entry.tags = e.tags.iter().map(|t| t.replace([';', ','], " ")).collect();
    if let Some(notes) = field("notes") {
        entry.set_unprotected("Notes", notes);
    }
    match e.kind.as_str() {
        "login" => {
            if let Some(u) = field("username") {
                entry.set_unprotected("UserName", u);
            }
            if let Some(s) = field("site") {
                entry.set_unprotected("URL", s);
            }
            if let Some(p) = part("password") {
                entry.set_protected("Password", p);
            }
            if let Some(t) = part("totp") {
                entry.set("otp", Value::protected(otp_uri(&e.label, t)));
            }
        }
        "note" => {
            if let Some(text) = part("text") {
                entry.set_unprotected("Notes", text);
            }
        }
        kind => {
            if let Some(p) = kdbx_names::primary_for(kind) {
                if let Some(v) = part(p.part) {
                    entry.set_protected(p.title, v);
                }
            }
            for c in COMPANIONS.iter().filter(|c| c.kind == kind) {
                if let Some(v) = part(c.part) {
                    entry.set_protected(c.title, v);
                }
            }
            for d in DETAILS {
                if let Some(v) = field(d.name) {
                    entry.set_unprotected(d.title, v);
                }
            }
        }
    }
}

/// A one-time-code seed as the `otpauth://` address KeePassXC reads; an address stays as it is.
fn otp_uri(label: &str, seed: &str) -> String {
    let seed = seed.trim();
    if seed.starts_with("otpauth://") {
        return seed.to_string();
    }
    let mut name = String::new();
    for b in label.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            name.push(b as char);
        } else {
            name.push_str(&format!("%{b:02X}"));
        }
    }
    format!("otpauth://totp/{name}?secret={}", seed.replace(' ', "").to_uppercase())
}

#[cfg(test)]
#[path = "kdbx_export_tests.rs"]
mod tests;
