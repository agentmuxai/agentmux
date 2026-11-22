// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A sealed Vault record: it opens only at the place it was sealed for, shows nothing readable, survives a re-seal under
//! a new epoch, and travels in pieces. Every id and value here is invented.

use std::collections::BTreeMap;

use super::*;
use crate::backend::vault::seal;

const SECRET: &str = "sync-sentinel-6120-pv";
const LABEL: &str = "Invoice provider key";

#[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
struct Body {
    label: String,
    parts: BTreeMap<String, String>,
}

fn body() -> Body {
    Body { label: LABEL.into(), parts: [("value".to_string(), SECRET.to_string())].into() }
}

fn other_key() -> Key {
    Zeroizing::new([9u8; 32])
}

fn version() -> Version {
    Version::new(1_795_000_000_000, 2, "machine-a")
}

fn at(v: &Version) -> Place<'_> {
    Place { vault: "vault-1", record: "entry-1", version: v, epoch: 3 }
}

fn contains(haystack: &[u8], needle: &str) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle.as_bytes())
}

#[test]
fn a_record_opens_at_its_place_and_shows_nothing_readable() {
    let (key, v) = (seal::test_key(), version());
    let sealed = seal_record(&key, &at(&v), &body()).unwrap();
    assert!(!contains(&sealed, SECRET) && !contains(&sealed, LABEL) && !contains(&sealed, "value"), "the label and the value are inside the seal");
    assert_eq!(open_record::<Body>(&key, &at(&v), &sealed).unwrap(), body());
    // A random nonce: the same record sealed twice is not the same bytes.
    assert_ne!(sealed, seal_record(&key, &at(&v), &body()).unwrap());
}

#[test]
fn a_record_does_not_open_at_another_place_or_with_another_key_or_changed() {
    let (key, v) = (seal::test_key(), version());
    let sealed = seal_record(&key, &at(&v), &body()).unwrap();
    let later = Version::new(v.wall_ms + 1, 0, "machine-a");
    let places = [
        Place { vault: "vault-2", ..at(&v) },
        Place { record: "entry-2", ..at(&v) },
        Place { version: &later, ..at(&v) },
        Place { epoch: 4, ..at(&v) },
    ];
    for place in &places {
        let said = open_record::<Body>(&key, place, &sealed).unwrap_err();
        assert!(said.contains("can't be opened"), "{said}");
        assert!(!said.contains(SECRET));
    }
    assert!(open_record::<Body>(&other_key(), &at(&v), &sealed).is_err());
    let mut changed = sealed.clone();
    *changed.last_mut().unwrap() ^= 1;
    assert!(open_record::<Body>(&key, &at(&v), &changed).is_err());
    for short in [&sealed[..5], &[][..]] {
        assert!(open_record::<Body>(&key, &at(&v), short).is_err());
    }
    let mut format = sealed.clone();
    format[0] = 2;
    assert!(open_record::<Body>(&key, &at(&v), &format).is_err());
}

#[test]
fn an_id_that_could_run_two_parts_of_the_place_together_is_refused() {
    let (key, v) = (seal::test_key(), version());
    for bad in ["a:b", "", "a b", "é"] {
        assert!(seal_record(&key, &Place { vault: bad, ..at(&v) }, &body()).is_err(), "{bad:?}");
        assert!(seal_record(&key, &Place { record: bad, ..at(&v) }, &body()).is_err(), "{bad:?}");
        let machine = Version::new(1, 0, bad);
        assert!(seal_record(&key, &Place { version: &machine, ..at(&v) }, &body()).is_err(), "{bad:?}");
    }
    assert!(seal_record(&key, &Place { vault: &"v".repeat(129), ..at(&v) }, &body()).is_err());
}

#[test]
fn versions_are_ordered_by_wall_time_then_counter_then_machine() {
    let a = Version::new(100, 5, "machine-b");
    assert!(Version::new(101, 0, "machine-a") > a, "wall time first");
    assert!(Version::new(100, 6, "machine-a") > a, "then the counter");
    assert!(Version::new(100, 5, "machine-c") > a, "then the machine, so two versions are never equal");
    assert_eq!(a.text(), "100.5.machine-b");
}

#[test]
fn a_new_epoch_reseals_a_record_without_changing_what_it_says() {
    let (old, new, v) = (seal::test_key(), other_key(), version());
    let sealed = seal_record(&old, &at(&v), &body()).unwrap();
    let to = Place { epoch: 4, ..at(&v) };
    let again = reseal(&old, &at(&v), &new, &to, &sealed).unwrap();
    assert_eq!(open_record::<Body>(&new, &to, &again).unwrap(), body());
    // What was sealed for the old epoch and key no longer opens under the new, and the new doesn't open under the old.
    assert!(open_record::<Body>(&new, &at(&v), &again).is_err());
    assert!(open_record::<Body>(&old, &to, &again).is_err());
    // A record sealed for another place is not re-sealed into this one.
    assert!(reseal(&old, &Place { record: "entry-2", ..at(&v) }, &new, &to, &sealed).is_err());
}

#[test]
fn a_record_goes_in_pieces_of_at_most_48_kib_and_comes_back_whole() {
    let (key, v) = (seal::test_key(), version());
    let big = Body { label: LABEL.into(), parts: [("value".to_string(), "x".repeat(150 * 1024))].into() };
    let sealed = seal_record(&key, &at(&v), &big).unwrap();
    let pieces = split(&sealed);
    assert!(pieces.len() >= 4 && pieces.iter().all(|p| p.len() <= MAX_CHUNK));
    assert_eq!(join(&pieces).unwrap(), sealed);
    assert_eq!(open_record::<Body>(&key, &at(&v), &join(&pieces).unwrap()).unwrap(), big);

    // Pieces that `split` could not have made are refused.
    assert!(join(&[]).is_err());
    assert!(join(&[vec![0u8; MAX_CHUNK + 1]]).is_err());
    assert!(join(&[vec![0u8; 10], vec![0u8; MAX_CHUNK]]).is_err(), "a short piece before the last");
    assert!(join(&[vec![0u8; MAX_CHUNK], Vec::new()]).is_err());
    let too_many: Vec<Vec<u8>> = (0..(MAX_SEALED / MAX_CHUNK + 2)).map(|_| vec![0u8; MAX_CHUNK]).collect();
    assert!(join(&too_many).is_err());
}

#[test]
fn a_record_over_the_size_limit_is_not_sealed() {
    let (key, v) = (seal::test_key(), version());
    let huge = vec![b'x'; MAX_SEALED];
    assert!(seal_raw(&key, &at(&v), &huge).unwrap_err().contains("at most 8 MiB"));
}
