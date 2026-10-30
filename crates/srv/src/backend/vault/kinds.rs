// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What the Vault can hold: each kind of entry, the section it is listed
//! in, its secret parts (sealed, never returned to an agent) and its plain
//! fields (stored as they are; the pane shows them).
//! docs/specs/SPEC_VAULT_PANE_AGENTS_USE_SECRETS_UNSEEN_2026_10_19.md §5.2.
//!
//! The pane's forms (`frontend/app/view/vault/vault-kinds.ts`) list the same
//! kinds; srv refuses a part or field a kind doesn't have.

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ts_rs::TS)]
#[serde(rename_all = "kebab-case")]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub enum VaultSection {
    Personal,
    Cards,
    ApiKeys,
    Logins,
    Notes,
}

pub struct Kind {
    pub name: &'static str,
    pub section: VaultSection,
    /// Sealed. The first one is the entry's main value: its tail is shown.
    pub parts: &'static [&'static str],
    /// Plain fields, besides the label every entry has.
    pub fields: &'static [&'static str],
    /// Whether the pane shows the main value's last characters.
    pub shows_tail: bool,
}

const ID_FIELDS: &[&str] = &["owner", "issuer", "expires", "notes"];

pub const KINDS: &[Kind] = &[
    Kind { name: "ssn", section: VaultSection::Personal, parts: &["value"], fields: &["owner", "notes"], shows_tail: true },
    Kind { name: "itin", section: VaultSection::Personal, parts: &["value"], fields: &["owner", "notes"], shows_tail: true },
    Kind { name: "tin", section: VaultSection::Personal, parts: &["value"], fields: &["owner", "notes"], shows_tail: true },
    Kind { name: "ein", section: VaultSection::Personal, parts: &["value"], fields: &["owner", "notes"], shows_tail: true },
    Kind { name: "date-of-birth", section: VaultSection::Personal, parts: &["value"], fields: &["owner", "notes"], shows_tail: false },
    Kind { name: "passport", section: VaultSection::Personal, parts: &["value"], fields: ID_FIELDS, shows_tail: true },
    Kind { name: "drivers-licence", section: VaultSection::Personal, parts: &["value"], fields: ID_FIELDS, shows_tail: true },
    Kind { name: "other-id", section: VaultSection::Personal, parts: &["value"], fields: ID_FIELDS, shows_tail: true },
    Kind { name: "card", section: VaultSection::Cards, parts: &["number", "expiry", "cvc", "name"], fields: &["owner", "notes"], shows_tail: true },
    Kind { name: "bank-account", section: VaultSection::Cards, parts: &["account", "routing"], fields: &["owner", "bank", "notes"], shows_tail: true },
    Kind {
        name: "api-key",
        section: VaultSection::ApiKeys,
        parts: &["value"],
        fields: &["service", "env_name", "expires", "notes"],
        shows_tail: true,
    },
    Kind { name: "login", section: VaultSection::Logins, parts: &["password", "totp"], fields: &["site", "username", "expires", "notes"], shows_tail: false },
    Kind { name: "note", section: VaultSection::Notes, parts: &["text"], fields: &["expires"], shows_tail: false },
];

pub fn kind(name: &str) -> Option<&'static Kind> {
    KINDS.iter().find(|k| k.name == name)
}

/// The main value's last characters, for the pane only: four of a long
/// value, two of a short one, none of one too short to hide anything.
pub fn tail(kind: &Kind, main: &str) -> String {
    if !kind.shows_tail {
        return String::new();
    }
    let chars: Vec<char> = main.chars().filter(|c| c.is_alphanumeric()).collect();
    let n = match chars.len() {
        0..=4 => 0,
        5..=7 => 2,
        _ => 4,
    };
    chars[chars.len() - n..].iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_has_a_main_value_and_a_unique_name() {
        let mut names = std::collections::HashSet::new();
        for k in KINDS {
            assert!(!k.parts.is_empty(), "{} has no secret part", k.name);
            assert!(names.insert(k.name), "{} is listed twice", k.name);
            for f in k.fields {
                assert!(!k.parts.contains(f), "{}: {f} is both a part and a field", k.name);
            }
        }
    }

    #[test]
    fn a_tail_shows_little_and_nothing_of_a_short_value() {
        let ssn = kind("ssn").unwrap();
        assert_eq!(tail(ssn, "123-45-6789"), "6789");
        assert_eq!(tail(ssn, "12345"), "45");
        assert_eq!(tail(ssn, "1234"), "");
        assert_eq!(tail(kind("login").unwrap(), "hunter2hunter2"), "", "a password shows no tail");
        assert_eq!(tail(kind("date-of-birth").unwrap(), "1990-01-01"), "");
    }
}
