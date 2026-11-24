// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Site grants as sync records: a stable id that names neither the entry nor the site, the check that a grant is for the
//! entry it was made for, and that the later grant or revoke wins. Invented values.

use super::*;
use crate::backend::vault::seal;
use crate::backend::vault::sync_seal::{open_record, seal_record, Place};

fn v(wall: i64, machine: &str) -> Version {
    Version::new(wall, 0, machine)
}

#[test]
fn a_grants_id_is_the_same_everywhere_and_says_neither_the_entry_nor_the_site() {
    let id = record_id("entry-1", "https://billing.example.com");
    assert_eq!(id, record_id("entry-1", "https://billing.example.com"));
    assert!(id.starts_with("g-") && id.len() == 34, "{id}");
    assert!(id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'));
    assert!(!id.contains("entry") && !id.contains("billing"));
    assert_ne!(id, record_id("entry-2", "https://billing.example.com"));
    assert_ne!(id, record_id("entry-1", "https://other.example.com"));
    // The parts are length-prefixed, so moving a character from one to the other is another grant.
    assert_ne!(record_id("ab", "c"), record_id("a", "bc"));
}

#[test]
fn a_grant_applies_only_to_the_entry_it_was_made_for() {
    let g = GrantBody::new("entry-1", "https://billing.example.com", 9_000, 1_000);
    assert!(g.applies_to(Some(1_000)));
    assert!(!g.applies_to(Some(2_000)), "a re-made entry of the same id doesn't inherit the site");
    assert!(!g.applies_to(None), "no such entry here yet: the grant waits");
    assert_eq!(g.record_id(), record_id("entry-1", "https://billing.example.com"));
}

#[test]
fn the_later_grant_or_revoke_wins_and_a_repeat_changes_nothing() {
    let (old, new) = (v(100, "a"), v(200, "b"));
    assert_eq!(merge(None, &[], &old), Decision::Applied);
    assert_eq!(merge(Some(&old), &[old.clone()], &new), Decision::Applied);
    assert_eq!(merge(Some(&new), &[old.clone(), new.clone()], &old), Decision::KeptLocal, "an older one");
    assert_eq!(merge(Some(&new), &[new.clone()], &new), Decision::KeptLocal, "the same one");
    // A revoke on one machine and a grant on another at the same instant: the machine name breaks the tie the same way
    // everywhere.
    let (a, b) = (Version::new(300, 1, "a"), Version::new(300, 1, "b"));
    assert_eq!(merge(Some(&a), &[a.clone()], &b), Decision::Applied);
    assert_eq!(merge(Some(&b), &[b.clone()], &a), Decision::KeptLocal);
}

#[test]
fn a_grant_goes_through_the_seal_with_nothing_readable_outside() {
    let g = GrantBody::new("entry-1", "https://billing.example.com", 9_000, 1_000);
    let (key, ver) = (seal::test_key(), v(9_000, "machine-a"));
    let id = g.record_id();
    let place = Place { vault: "vault-1", record: &id, version: &ver, base: None, epoch: 1 };
    let sealed = seal_record(&key, &place, &g).unwrap();
    assert!(!sealed.windows(7).any(|w| w == b"billing"));
    assert_eq!(open_record::<GrantBody>(&key, &place, &sealed).unwrap(), g);
}
