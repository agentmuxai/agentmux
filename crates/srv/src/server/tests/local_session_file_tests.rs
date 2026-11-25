// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use crate::server::local_session::{Collected, Grant, RouteClass, SessionStore, SESSION_IDLE_LIFE, SESSION_MAX_LIFE};
use std::time::Instant;

const NOW: u64 = 1_800_000_000;

fn rec(n: u8, born: u64, seen: u64) -> Record {
    Record { cookie: [n; 32], token: [n.wrapping_add(100); 32], born_secs: born, last_seen_secs: seen }
}

fn provided(key: &str) -> KeyBinding {
    KeyBinding::Provided(key.as_bytes().to_vec())
}

fn wall() -> u64 {
    agentmux_common::time::now_secs_u64()
}

// ---- the bytes ----

#[test]
fn a_file_round_trips_under_both_bindings() {
    for binding in [KeyBinding::Generated, provided("a-key-of-at-least-thirty-two-characters")] {
        let records = vec![rec(1, NOW - 500, NOW - 100), rec(2, NOW - 90, NOW - 10)];
        let bytes = encode(&binding, NOW, &records);
        assert_eq!(decode(&binding, NOW, &bytes), Ok(records));
        assert_eq!(decode(&binding, NOW + 5, &bytes).map(|r| r.len()), Ok(2));
    }
    // No session is a valid file too.
    assert_eq!(decode(&KeyBinding::Generated, NOW, &encode(&KeyBinding::Generated, NOW, &[])), Ok(vec![]));
}

#[test]
fn a_truncated_or_extended_or_foreign_file_is_refused() {
    let b = KeyBinding::Generated;
    let bytes = encode(&b, NOW, &[rec(1, NOW - 50, NOW - 10)]);
    for cut in [0, 5, HEADER_LEN, bytes.len() - 1, bytes.len() - TRAILER_LEN] {
        assert_eq!(decode(&b, NOW, &bytes[..cut]), Err(Refused::Damaged), "cut at {cut}");
    }
    let mut longer = bytes.clone();
    longer.push(0);
    assert_eq!(decode(&b, NOW, &longer), Err(Refused::Damaged));
    let mut foreign = bytes.clone();
    foreign[0] ^= 0xff;
    assert_eq!(decode(&b, NOW, &foreign), Err(Refused::Damaged));
    assert_eq!(decode(&b, NOW, b"not a sessions file at all, but long enough to have a header and a trailer"), Err(Refused::Damaged));
}

#[test]
fn a_changed_byte_anywhere_is_refused() {
    for b in [KeyBinding::Generated, provided("a-key-of-at-least-thirty-two-characters")] {
        let bytes = encode(&b, NOW, &[rec(1, NOW - 50, NOW - 10), rec(2, NOW - 40, NOW - 5)]);
        for at in 0..bytes.len() {
            let mut t = bytes.clone();
            t[at] ^= 0x01;
            assert!(decode(&b, NOW, &t).is_err(), "a change at byte {at} was accepted");
        }
    }
}

#[test]
fn a_count_that_does_not_match_the_length_is_refused() {
    let b = KeyBinding::Generated;
    let mut bytes = encode(&b, NOW, &[rec(1, NOW - 50, NOW - 10)]);
    bytes[17..21].copy_from_slice(&u32::MAX.to_be_bytes());
    assert_eq!(decode(&b, NOW, &bytes), Err(Refused::Damaged));
    // A count above the limit is refused before anything is allocated for it.
    let mut big = encode(&b, NOW, &[]);
    big[17..21].copy_from_slice(&((MAX_SESSIONS as u32) + 1).to_be_bytes());
    assert_eq!(decode(&b, NOW, &big), Err(Refused::Damaged));
}

#[test]
fn a_file_made_under_another_key_or_another_kind_of_key_is_refused() {
    let key_a = provided("the-first-key-of-at-least-thirty-two-chars");
    let key_b = provided("the-second-key-of-at-least-thirty-two-chars");
    let records = [rec(1, NOW - 50, NOW - 10)];
    let by_a = encode(&key_a, NOW, &records);
    assert!(decode(&key_a, NOW, &by_a).is_ok());
    assert_eq!(decode(&key_b, NOW, &by_a), Err(Refused::Damaged));
    assert_eq!(decode(&KeyBinding::Generated, NOW, &by_a), Err(Refused::Damaged));
    let by_generated = encode(&KeyBinding::Generated, NOW, &records);
    assert_eq!(decode(&key_a, NOW, &by_generated), Err(Refused::Damaged));
}

#[test]
fn a_file_saved_after_now_is_refused_because_the_clock_moved_back() {
    let b = KeyBinding::Generated;
    let bytes = encode(&b, NOW, &[rec(1, NOW - 50, NOW - 10)]);
    assert!(decode(&b, NOW, &bytes).is_ok());
    assert_eq!(decode(&b, NOW - 1, &bytes), Err(Refused::ClockMovedBack));
    assert_eq!(decode(&b, 0, &bytes), Err(Refused::ClockMovedBack));
}

#[test]
fn a_time_that_cannot_be_is_refused() {
    let b = KeyBinding::Generated;
    // Used before it was confirmed.
    assert_eq!(decode(&b, NOW, &encode(&b, NOW, &[rec(1, NOW - 10, NOW - 50)])), Err(Refused::Inconsistent));
    // Used after the file was saved.
    assert_eq!(decode(&b, NOW, &encode(&b, NOW, &[rec(1, NOW - 10, NOW + 1)])), Err(Refused::Inconsistent));
}

#[test]
fn the_file_holds_the_digests_only_and_no_text_of_what_they_come_from() {
    let store = SessionStore::default();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(FILE_NAME);
    let now = Instant::now();
    store.keep_in(path.clone(), KeyBinding::Generated, now, wall());
    let grant = signed_in(&store, now);
    let bytes = std::fs::read(&path).unwrap();
    let hay = String::from_utf8_lossy(&bytes);
    assert!(!hay.contains(&grant.cookie) && !hay.contains(&grant.token));
    let cookie_digest = Sha256::digest(grant.cookie.as_bytes());
    assert!(bytes.windows(32).any(|w| w == cookie_digest.as_slice()), "the cookie's digest is what is kept");
}

// ---- the file ----

#[test]
fn a_file_is_written_whole_and_owner_only_and_read_back() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(FILE_NAME);
    let b = KeyBinding::Generated;
    assert_eq!(load(&path, &b, NOW), Ok(None), "no file is not a refusal");
    let records = vec![rec(3, NOW - 80, NOW - 20)];
    save(&path, &b, NOW, &records).unwrap();
    assert_eq!(load(&path, &b, NOW), Ok(Some(records.clone())));
    // The temp file is gone and a second save replaces the first.
    save(&path, &b, NOW, &[rec(4, NOW - 8, NOW - 2)]).unwrap();
    assert_eq!(load(&path, &b, NOW).unwrap().unwrap()[0].cookie, [4; 32]);
    let left: Vec<_> = std::fs::read_dir(dir.path()).unwrap().map(|e| e.unwrap().file_name()).collect();
    assert_eq!(left, vec![std::ffi::OsString::from(FILE_NAME)]);
}

#[test]
fn a_refused_file_is_deleted() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(FILE_NAME);
    let b = KeyBinding::Generated;
    save(&path, &b, NOW, &[rec(1, NOW - 80, NOW - 20)]).unwrap();
    assert_eq!(load(&path, &b, NOW - 1000), Err(Refused::ClockMovedBack));
    assert!(!path.exists());
    std::fs::write(&path, b"junk").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    assert!(load(&path, &b, NOW).is_err());
    assert!(!path.exists());
}

#[cfg(unix)]
#[test]
fn a_file_or_folder_others_can_open_is_refused() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(FILE_NAME);
    let b = KeyBinding::Generated;
    for mode in [0o640, 0o644, 0o660, 0o666, 0o604] {
        save(&path, &b, NOW, &[rec(1, NOW - 80, NOW - 20)]).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
        assert_eq!(load(&path, &b, NOW), Err(Refused::TooOpen), "mode {mode:o}");
        assert!(!path.exists());
    }
    save(&path, &b, NOW, &[rec(1, NOW - 80, NOW - 20)]).unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o777)).unwrap();
    assert_eq!(load(&path, &b, NOW), Err(Refused::TooOpen), "a folder anyone can write to");
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
}

#[cfg(windows)]
#[test]
fn a_file_someone_else_can_open_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(FILE_NAME);
    let b = KeyBinding::Generated;
    save(&path, &b, NOW, &[rec(1, NOW - 80, NOW - 20)]).unwrap();
    assert!(private_enough(&path).is_ok(), "a file the writer made is the user's alone");
    // Everyone may read it.
    agentmux_common::owner_only::apply_sddl(&path, "D:P(A;;FA;;;WD)").unwrap();
    assert_eq!(load(&path, &b, NOW), Err(Refused::TooOpen));
    assert!(!path.exists());
}

// ---- the store ----

fn signed_in(store: &SessionStore, now: Instant) -> Grant {
    let code = store.mint_code(now);
    let entered = store.enter(&code, now).expect("a fresh code is accepted");
    assert!(store.decide(&code, true, now));
    match store.collect(&entered.handle, now) {
        Collected::Granted(g) => g,
        _ => panic!("a confirmed session is handed over"),
    }
}

fn admitted(store: &SessionStore, g: &Grant, now: Instant) -> bool {
    store.admit(Some(&g.cookie), Some(&g.token), RouteClass::Both, now)
}

/// Put `bytes` back as the file, as the writer made it (owner-only).
fn restore(path: &std::path::Path, bytes: &[u8]) {
    std::fs::write(path, bytes).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
}

/// The store a restart makes: new memory, the same file.
fn restarted(path: &std::path::Path, binding: KeyBinding, wall_secs: u64) -> SessionStore {
    let store = SessionStore::default();
    store.keep_in(path.to_path_buf(), binding, Instant::now(), wall_secs);
    store
}

#[test]
fn a_session_survives_a_restart_and_a_stranger_still_does_not_get_in() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(FILE_NAME);
    let first = SessionStore::default();
    first.keep_in(path.clone(), KeyBinding::Generated, Instant::now(), wall());
    let grant = signed_in(&first, Instant::now());
    assert!(admitted(&first, &grant, Instant::now()));
    drop(first);

    let second = restarted(&path, KeyBinding::Generated, wall() + 60);
    assert!(admitted(&second, &grant, Instant::now()), "the same cookie and token are still signed in");
    assert!(!second.admit(Some(&grant.cookie), Some("another-token"), RouteClass::Both, Instant::now()), "the token must match");
    assert!(!second.admit(Some("another-cookie"), Some(&grant.token), RouteClass::Both, Instant::now()), "the cookie must match");
    assert_eq!(second.session_count(Instant::now()), 1);
}

#[test]
fn a_session_past_its_idle_limit_or_its_absolute_limit_is_not_taken_back() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(FILE_NAME);
    let first = SessionStore::default();
    first.keep_in(path.clone(), KeyBinding::Generated, Instant::now(), wall());
    let grant = signed_in(&first, Instant::now());
    let saved = std::fs::read(&path).unwrap();

    // Idle: 12 hours and a second after its last use.
    restore(&path, &saved);
    let idle = restarted(&path, KeyBinding::Generated, wall() + SESSION_IDLE_LIFE.as_secs() + 1);
    assert!(!admitted(&idle, &grant, Instant::now()));

    // Just inside the idle limit it comes back.
    restore(&path, &saved);
    let inside = restarted(&path, KeyBinding::Generated, wall() + SESSION_IDLE_LIFE.as_secs() - 5);
    assert!(admitted(&inside, &grant, Instant::now()));

    // Absolute: the file is made with a session confirmed a week ago and used a minute ago.
    let b = KeyBinding::Generated;
    let now = wall();
    let g = Grant { cookie: "cookie-of-a-week-ago".into(), token: "token-of-a-week-ago".into() };
    let record = Record {
        cookie: Sha256::digest(g.cookie.as_bytes()).into(),
        token: Sha256::digest(g.token.as_bytes()).into(),
        born_secs: now - SESSION_MAX_LIFE.as_secs() - 10,
        last_seen_secs: now - 60,
    };
    save(&path, &b, now, &[record]).unwrap();
    let old = restarted(&path, b, now);
    assert!(!admitted(&old, &g, Instant::now()), "however much it was used, a session ends a week after it was confirmed");
}

#[test]
fn a_clock_that_moved_back_clears_every_session_and_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(FILE_NAME);
    let first = SessionStore::default();
    first.keep_in(path.clone(), KeyBinding::Generated, Instant::now(), wall());
    let grant = signed_in(&first, Instant::now());
    drop(first);
    let second = restarted(&path, KeyBinding::Generated, wall() - 3600);
    assert!(!admitted(&second, &grant, Instant::now()));
    assert_eq!(second.session_count(Instant::now()), 0);
    assert!(!path.exists(), "the file is gone, not left to be used when the clock catches up");
}

#[test]
fn a_changed_provided_key_clears_every_session_and_a_generated_key_does_not_matter() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(FILE_NAME);
    let key = provided("the-first-key-of-at-least-thirty-two-chars");
    let first = SessionStore::default();
    first.keep_in(path.clone(), key.clone(), Instant::now(), wall());
    let grant = signed_in(&first, Instant::now());
    drop(first);
    // The same key: it stays.
    let same = restarted(&path, key.clone(), wall());
    assert!(admitted(&same, &grant, Instant::now()));
    drop(same);
    // Another key: they all end, and the file with them.
    let other = restarted(&path, provided("the-second-key-of-at-least-thirty-two-chars"), wall());
    assert!(!admitted(&other, &grant, Instant::now()));
    assert!(!path.exists());
    // A key srv generated is new at every start; sessions made under one are kept whatever the next one is.
    let g1 = SessionStore::default();
    g1.keep_in(path.clone(), KeyBinding::Generated, Instant::now(), wall());
    let grant = signed_in(&g1, Instant::now());
    drop(g1);
    assert!(admitted(&restarted(&path, KeyBinding::Generated, wall()), &grant, Instant::now()));
}

#[test]
fn a_damaged_file_starts_the_store_empty_and_works_after() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(FILE_NAME);
    let first = SessionStore::default();
    first.keep_in(path.clone(), KeyBinding::Generated, Instant::now(), wall());
    let grant = signed_in(&first, Instant::now());
    drop(first);
    let mut bytes = std::fs::read(&path).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 0xff;
    restore(&path, &bytes);
    let second = restarted(&path, KeyBinding::Generated, wall());
    assert!(!admitted(&second, &grant, Instant::now()), "nothing from a file that fails its check is used");
    let fresh = signed_in(&second, Instant::now());
    assert!(admitted(&second, &fresh, Instant::now()));
    drop(second);
    assert!(admitted(&restarted(&path, KeyBinding::Generated, wall()), &fresh, Instant::now()), "and a new sign-in is kept as before");
}

#[test]
fn signing_out_everywhere_ends_every_session_and_deletes_the_file_and_one_sign_out_ends_one() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(FILE_NAME);
    let store = SessionStore::default();
    store.keep_in(path.clone(), KeyBinding::Generated, Instant::now(), wall());
    let a = signed_in(&store, Instant::now());
    let b = signed_in(&store, Instant::now());
    assert!(path.exists());
    // One browser signs out: the other is still kept across a restart.
    assert!(store.revoke(crate::server::local_session::SessionId::of_cookie(&a.cookie), Instant::now()));
    let after_one = restarted(&path, KeyBinding::Generated, wall());
    assert!(!admitted(&after_one, &a, Instant::now()));
    assert!(admitted(&after_one, &b, Instant::now()));
    // Every browser signs out: nothing is left to keep.
    assert_eq!(after_one.revoke_all(), 1);
    assert!(!path.exists());
    assert!(!admitted(&restarted(&path, KeyBinding::Generated, wall()), &b, Instant::now()));
}

#[test]
fn a_use_is_written_at_most_every_few_minutes_and_a_confirm_at_once() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(FILE_NAME);
    let store = SessionStore::default();
    let t0 = Instant::now();
    store.keep_in(path.clone(), KeyBinding::Generated, t0, wall());
    let grant = signed_in(&store, t0);
    let written = std::fs::read(&path).unwrap();
    // A use a few seconds later does not rewrite the file.
    assert!(admitted(&store, &grant, t0 + Duration::from_secs(30)));
    assert_eq!(std::fs::read(&path).unwrap(), written);
    // One after the interval does, with a later time of last use.
    assert!(admitted(&store, &grant, t0 + SAVE_EVERY + Duration::from_secs(1)));
    assert_ne!(std::fs::read(&path).unwrap(), written);
}

#[test]
fn a_store_with_no_file_given_keeps_nothing() {
    let store = SessionStore::default();
    let grant = signed_in(&store, Instant::now());
    assert!(admitted(&store, &grant, Instant::now()));
    assert_eq!(store.revoke_all(), 1);
}
