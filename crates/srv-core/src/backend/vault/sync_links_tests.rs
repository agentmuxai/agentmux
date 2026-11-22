// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! An entry's link ids across machines: both ids kept when two machines each gave one, none reassigned or reused, a bound,
//! and the same set whatever the order. Every id is invented.

use std::collections::HashMap;

use super::*;
use crate::backend::vault::sync_record::EntryBody;
use crate::backend::vault::sync_seal::{open_record, seal_record, Place, Version};
use crate::backend::vault::seal;

const A: &str = "e_aaaaaaaaaaaaaaaa";
const B: &str = "e_bbbbbbbbbbbbbbbb";
const C: &str = "e_cccccccccccccccc";

#[derive(Default)]
struct Owners {
    held: HashMap<String, String>,
    retired: Vec<String>,
}

impl LinkOwners for Owners {
    fn owner_of(&self, id: &str) -> Option<String> {
        self.held.get(id).cloned()
    }
    fn retired(&self, id: &str) -> bool {
        self.retired.iter().any(|r| r == id)
    }
}

fn own(id: &str) -> Links {
    Links { primary: Some(id.to_string()), aliases: vec![] }
}

fn ids(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

#[test]
fn two_machines_that_each_gave_the_entry_an_id_keep_both_and_a_link_made_on_either_opens_it_on_both() {
    // Machine 1 gave the entry A and machine 2 gave it B, before either synced.
    let none = Owners::default();
    let on_1 = settle(&none, "entry-1", &own(A), &ids(&[B]));
    let on_2 = settle(&none, "entry-1", &own(B), &ids(&[A]));
    assert_eq!((on_1.primary.as_deref(), on_1.aliases.clone()), (Some(A), ids(&[B])), "each keeps the id it showed as its own");
    assert_eq!((on_2.primary.as_deref(), on_2.aliases.clone()), (Some(B), ids(&[A])));
    for links in [&on_1, &on_2] {
        assert!(links.opens(A) && links.opens(B), "a link made with either id opens the entry here");
    }
    // The same record arriving again changes nothing.
    assert_eq!(settle(&none, "entry-1", &on_1, &ids(&[B, A])), on_1);
}

#[test]
fn a_machine_with_no_id_for_the_entry_takes_the_one_it_was_sent() {
    let got = settle(&Owners::default(), "entry-1", &Links::default(), &ids(&[B, A]));
    assert_eq!((got.primary.as_deref(), got.aliases), (Some(B), ids(&[A])), "the sender's own first");
}

#[test]
fn an_id_is_never_reassigned_to_another_entry() {
    let mut owners = Owners::default();
    owners.held.insert(A.to_string(), "entry-2".to_string());
    // A record says entry-1 has A, but A names entry-2 here: refused, and entry-2 keeps it.
    let got = settle(&owners, "entry-1", &own(B), &ids(&[A, C]));
    assert!(!got.opens(A));
    assert_eq!((got.primary.as_deref(), got.aliases), (Some(B), ids(&[C])));
    // The owner itself is not refused.
    assert!(settle(&owners, "entry-2", &Links::default(), &ids(&[A])).opens(A));
}

#[test]
fn a_removed_entrys_ids_are_not_reused() {
    let owners = Owners { held: HashMap::new(), retired: ids(&[A]) };
    let got = settle(&owners, "entry-9", &own(B), &ids(&[A]));
    assert!(!got.opens(A));
    assert_eq!(got, own(B));
    // Even with no id of its own, an entry does not take a retired one.
    assert_eq!(settle(&owners, "entry-9", &Links::default(), &ids(&[A])), Links::default());
}

#[test]
fn what_is_not_an_id_is_dropped_and_the_set_is_bounded_and_the_same_in_any_order() {
    let none = Owners::default();
    let junk = ids(&["", "e_short", "x_aaaaaaaaaaaaaaaa", "e_AAAAAAAAAAAAAAAA", "e_aaaaaaaaaaaaaaaaa", "e_aaaaaaaaaaaaaa/.."]);
    assert_eq!(settle(&none, "entry-1", &own(A), &junk), own(A));

    let many: Vec<String> = (0..10).map(|i| format!("e_{i:016x}")).collect();
    let forward = settle(&none, "entry-1", &own(A), &many);
    let mut reversed = many.clone();
    reversed.reverse();
    let backward = settle(&none, "entry-1", &own(A), &reversed);
    assert_eq!(forward, backward, "the order records arrive in does not matter");
    assert_eq!(forward.ids().len(), MAX_LINKS);
    assert_eq!(forward.primary.as_deref(), Some(A), "its own id stays");
}

#[test]
fn link_ids_travel_in_the_sealed_body_and_nowhere_in_the_clear() {
    let plain: EntryBody = serde_json::from_str(r#"{"kind":"k","label":"l","fields":{},"tags":[],"parts":{},"created_at":1,"updated_at":2}"#).unwrap();
    let body = plain.with_link_ids(ids(&[A, B]));
    let (key, v) = (seal::test_key(), Version::new(5_000, 1, "machine-a"));
    let place = Place { vault: "vault-1", record: "entry-1", version: &v, base: None, epoch: 1 };
    let sealed = seal_record(&key, &place, &body).unwrap();
    assert!(!sealed.windows(A.len()).any(|w| w == A.as_bytes()), "the ids are inside the seal");
    let opened: EntryBody = open_record(&key, &place, &sealed).unwrap();
    assert_eq!(opened.link_ids, ids(&[A, B]));
    // A body from a machine that sends none opens as one with none.
    let none: EntryBody = serde_json::from_str(r#"{"kind":"k","label":"l","fields":{},"tags":[],"parts":{},"created_at":1,"updated_at":2}"#).unwrap();
    assert!(none.link_ids.is_empty());
}
