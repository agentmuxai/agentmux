// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The computer-wide sync lock: one install at a time, a crashed holder replaced when its lease ends, a replaced holder
// fenced out, and a store of its own never shared with another.

use rusqlite::Connection;

use super::lock::{acquire, holds, init, release, renew, Lock, LEASE_MS};

const NOW: i64 = 1_800_000_000_000;

fn store() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    init(&conn).unwrap();
    conn
}

#[test]
fn only_one_install_holds_the_lock_at_a_time() {
    let conn = store();
    assert_eq!(acquire(&conn, "a", NOW, LEASE_MS).unwrap(), Lock::Held { generation: 1 });
    assert_eq!(acquire(&conn, "b", NOW + 1, LEASE_MS).unwrap(), Lock::Busy { by: "a".into(), until: NOW + LEASE_MS });
    assert_eq!(acquire(&conn, "a", NOW + 2, LEASE_MS).unwrap(), Lock::Held { generation: 1 }, "the holder asking again keeps it");
    assert!(holds(&conn, "a", 1, NOW + 3).unwrap());
    assert!(!holds(&conn, "b", 1, NOW + 3).unwrap());
}

#[test]
fn a_crashed_holder_is_replaced_when_its_lease_ends_and_is_fenced_out() {
    let conn = store();
    acquire(&conn, "a", NOW, LEASE_MS).unwrap();
    assert!(matches!(acquire(&conn, "b", NOW + LEASE_MS - 1, LEASE_MS).unwrap(), Lock::Busy { .. }), "not before the lease ends");
    assert_eq!(acquire(&conn, "b", NOW + LEASE_MS, LEASE_MS).unwrap(), Lock::Held { generation: 2 });
    // The replaced install finds out when it renews, and no longer holds it.
    assert!(!renew(&conn, "a", 1, NOW + LEASE_MS + 1, LEASE_MS).unwrap());
    assert!(!holds(&conn, "a", 1, NOW + LEASE_MS + 1).unwrap());
    assert!(renew(&conn, "b", 2, NOW + LEASE_MS + 1, LEASE_MS).unwrap());
    assert!(holds(&conn, "b", 2, NOW + LEASE_MS + 2).unwrap());
}

#[test]
fn a_lease_that_ran_out_is_not_held_even_if_nobody_else_took_it() {
    let conn = store();
    let Lock::Held { generation } = acquire(&conn, "a", NOW, LEASE_MS).unwrap() else { panic!("held") };
    assert!(renew(&conn, "a", generation, NOW + LEASE_MS - 1, LEASE_MS).unwrap());
    assert!(!holds(&conn, "a", generation, NOW + 3 * LEASE_MS).unwrap(), "an expired lease signs nothing");
    assert!(!renew(&conn, "a", generation, NOW + 3 * LEASE_MS, LEASE_MS).unwrap(), "and cannot be renewed: it asks again");
    assert_eq!(acquire(&conn, "a", NOW + 3 * LEASE_MS, LEASE_MS).unwrap(), Lock::Held { generation });
}

#[test]
fn giving_the_lock_up_frees_it_and_only_the_holder_can() {
    let conn = store();
    acquire(&conn, "a", NOW, LEASE_MS).unwrap();
    release(&conn, "b", 1).unwrap();
    assert!(holds(&conn, "a", 1, NOW + 1).unwrap(), "another install cannot release it");
    release(&conn, "a", 7).unwrap();
    assert!(holds(&conn, "a", 1, NOW + 1).unwrap(), "nor a stale generation");
    release(&conn, "a", 1).unwrap();
    assert_eq!(acquire(&conn, "b", NOW + 2, LEASE_MS).unwrap(), Lock::Held { generation: 2 });
}

#[test]
fn an_install_with_a_store_of_its_own_never_competes_for_another_stores_lock() {
    let (shared, own) = (store(), store());
    assert_eq!(acquire(&shared, "stable", NOW, LEASE_MS).unwrap(), Lock::Held { generation: 1 });
    assert_eq!(acquire(&own, "dev", NOW, LEASE_MS).unwrap(), Lock::Held { generation: 1 });
    assert!(holds(&shared, "stable", 1, NOW + 1).unwrap() && !holds(&shared, "dev", 1, NOW + 1).unwrap());
}
