// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Which parts of which entries an agent may have filled into a web form, and the
//! kind of form field each belongs in
//! (docs/specs/SPEC_VAULT_PANE_AGENTS_USE_SECRETS_UNSEEN_2026_10_19.md §6.1, §6.3).
//!
//! The slot names are the ones the host's `vault_slots.rs` gives a field it
//! recognises; a field fitting the entry's slot is what lets "Always on this
//! site" be offered and used.

use super::kinds::{self, Kind};

/// What `fill` with `vault` names: an entry, and one of its parts.
#[derive(Debug, PartialEq, Eq)]
pub struct Target<'a> {
    pub entry_id: &'a str,
    pub part: &'static str,
}

/// Splits `"id"` or `"id.part"`; an id has no `.`.
pub fn split(vault: &str) -> (&str, Option<&str>) {
    match vault.trim().split_once('.') {
        Some((id, part)) => (id, Some(part)),
        None => (vault.trim(), None),
    }
}

/// The part to fill (the named one, else the entry's main one), or why not.
/// Passwords and one-time codes are never filled (spec §13 question 2).
pub fn fillable_part(kind: &'static Kind, part: Option<&str>) -> Result<&'static str, String> {
    if kind.name == "login" {
        return Err("a login's password and one-time code aren't filled from the Vault: the person types them (agents never fill a password)".into());
    }
    let wanted = part.unwrap_or(kind.parts[0]);
    kind.parts
        .iter()
        .find(|p| **p == wanted)
        .copied()
        .ok_or_else(|| format!("a {} entry has no part {wanted:?}; its parts are {}", kind.name, kind.parts.join(", ")))
}

/// The kind of form field this part belongs in, or `None` when no field can be
/// known to fit it (an ID of another sort, a note), so the person is always asked.
pub fn slot(kind: &Kind, part: &str) -> Option<&'static str> {
    Some(match (kind.name, part) {
        ("ssn" | "itin" | "tin", _) => "tax-id",
        ("ein", _) => "ein",
        ("date-of-birth", _) => "date-of-birth",
        ("passport", _) => "passport",
        ("drivers-licence", _) => "drivers-licence",
        ("card", "number") => "card-number",
        ("card", "expiry") => "card-expiry",
        ("card", "cvc") => "card-cvc",
        ("card", "name") => "card-name",
        ("bank-account", "account") => "bank-account",
        ("bank-account", "routing") => "bank-routing",
        ("api-key", _) => "api-key",
        _ => return None,
    })
}

/// What a part is called to the person.
pub fn what(kind: &Kind, part: &str) -> String {
    let kind_label = match kind.name {
        "ssn" => "Social Security number",
        "itin" => "ITIN",
        "tin" => "taxpayer ID",
        "ein" => "EIN",
        "date-of-birth" => "date of birth",
        "passport" => "passport number",
        "drivers-licence" => "driver's licence number",
        "other-id" => "ID number",
        "api-key" => "API key",
        "note" => "secret note",
        other => other,
    };
    match (kind.name, part) {
        ("card", p) => format!("card {}", match p { "number" => "number", "expiry" => "expiry", "cvc" => "security code", _ => "holder name" }),
        ("bank-account", "routing") => "bank routing number".to_string(),
        ("bank-account", _) => "bank account number".to_string(),
        _ => kind_label.to_string(),
    }
}

/// Every kind, for the tests.
#[cfg(test)]
pub fn all_kinds() -> &'static [Kind] {
    kinds::KINDS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_id_and_an_optional_part_are_split() {
        assert_eq!(split("ssn-alex"), ("ssn-alex", None));
        assert_eq!(split(" card-company.number "), ("card-company", Some("number")));
    }

    #[test]
    fn a_login_is_never_filled_and_a_part_must_exist() {
        let login = kinds::kind("login").unwrap();
        assert!(fillable_part(login, None).unwrap_err().contains("password"));
        assert!(fillable_part(login, Some("totp")).is_err());
        let card = kinds::kind("card").unwrap();
        assert_eq!(fillable_part(card, None), Ok("number"));
        assert_eq!(fillable_part(card, Some("cvc")), Ok("cvc"));
        assert!(fillable_part(card, Some("pin")).unwrap_err().contains("its parts are"));
        assert_eq!(fillable_part(kinds::kind("ssn").unwrap(), None), Ok("value"));
    }

    #[test]
    fn every_fillable_part_is_named_and_most_have_a_slot() {
        for k in all_kinds().iter().filter(|k| k.name != "login") {
            for p in k.parts {
                assert!(!what(k, p).is_empty());
            }
        }
        let card = kinds::kind("card").unwrap();
        assert_eq!(slot(card, "number"), Some("card-number"));
        assert_eq!(slot(card, "cvc"), Some("card-cvc"));
        assert_eq!(slot(kinds::kind("ssn").unwrap(), "value"), Some("tax-id"));
        assert_eq!(slot(kinds::kind("itin").unwrap(), "value"), Some("tax-id"));
        assert_eq!(slot(kinds::kind("other-id").unwrap(), "value"), None, "an ID of another sort always asks");
        assert_eq!(slot(kinds::kind("note").unwrap(), "text"), None);
    }

    #[test]
    fn the_slots_are_the_ones_the_host_names() {
        // crates/cef/src/browser_api/vault_slots.rs is where a field gets its slots.
        let host = include_str!("../../../../cef/src/browser_api/vault_slots.rs");
        for k in all_kinds().iter().filter(|k| k.name != "login") {
            for p in k.parts {
                if let Some(s) = slot(k, p) {
                    assert!(host.contains(&format!("= \"{s}\";")), "the host doesn't know the slot {s}");
                }
            }
        }
    }
}
