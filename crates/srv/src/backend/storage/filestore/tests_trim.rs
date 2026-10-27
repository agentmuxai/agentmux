// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A circular file's dead head is deleted (trim.rs); what the window holds
//! reads as before and offsets stay absolute.

use super::core::PART_DATA_SIZE;
use super::trim::first_live_part;
use super::{FileMeta, FileOpts, FileStore};

const PART: i64 = PART_DATA_SIZE as i64;

fn parts_held(store: &FileStore, zone: &str, name: &str) -> i64 {
    store
        .lock_conn()
        .query_row("SELECT COUNT(*) FROM db_file_data WHERE zoneid = ?1 AND name = ?2", [zone, name], |r| r.get(0))
        .unwrap()
}

fn circular_store(maxsize: i64) -> FileStore {
    let store = FileStore::open_in_memory().unwrap();
    store.make_file("z", "term", FileMeta::new(), FileOpts { circular: true, maxsize, ..Default::default() }).unwrap();
    store
}

#[test]
fn the_first_live_part_is_the_one_holding_the_window_start() {
    assert_eq!(first_live_part(10 * PART, 0), 0, "no limit");
    assert_eq!(first_live_part(3 * PART, 4 * PART), 0, "under the window");
    assert_eq!(first_live_part(10 * PART, 4 * PART), 6);
    assert_eq!(first_live_part(10 * PART + 5, 4 * PART), 6, "the start is mid-part 6: part 6 stays");
    assert_eq!(first_live_part(10 * PART - 1, 4 * PART), 5, "the window starts on part 5's last byte");
}

#[test]
fn the_parts_below_the_window_are_deleted_and_the_window_reads_as_before() {
    let store = circular_store(4 * PART);
    let chunk: Vec<u8> = (0..PART as usize).map(|i| (i % 251) as u8).collect();
    for _ in 0..12 {
        store.append_data_at("z", "term", &chunk).unwrap();
    }
    assert_eq!(parts_held(&store, "z", "term"), 12, "appends alone drop nothing");
    let removed = store.trim_circular_head("z", "term").unwrap();
    assert_eq!(removed, 8);
    assert_eq!(parts_held(&store, "z", "term"), 4);

    let file = store.stat("z", "term").unwrap().unwrap();
    assert_eq!(file.size, 12 * PART, "offsets stay absolute");
    assert_eq!(file.data_start_idx(), 8 * PART);
    let held = store.read_file("z", "term").unwrap().unwrap();
    assert_eq!(held.len() as i64, 4 * PART);
    assert_eq!(held, chunk.repeat(4));
    let (from, bytes) = store.read_at("z", "term", 0, 12 * PART).unwrap();
    assert_eq!((from, bytes.len() as i64), (8 * PART, 4 * PART), "a read from before the window starts at the window");
    // "To the end" (size 0) is the whole window too, from offset 0 or any stale
    // offset below it, and from an offset inside it only what follows.
    for stale in [0, 100, 8 * PART - 1] {
        let (from, bytes) = store.read_at("z", "term", stale, 0).unwrap();
        assert_eq!((from, bytes.len() as i64), (8 * PART, 4 * PART), "read to the end from {stale}");
    }
    let (from, bytes) = store.read_at("z", "term", 10 * PART, 0).unwrap();
    assert_eq!((from, bytes.len() as i64), (10 * PART, 2 * PART));
}

#[test]
fn a_file_that_is_not_circular_or_still_fits_loses_nothing() {
    let plain = FileStore::open_in_memory().unwrap();
    plain.make_file("z", "term", FileMeta::new(), FileOpts::default()).unwrap();
    for _ in 0..6 {
        plain.append_data_at("z", "term", &vec![b'x'; PART as usize]).unwrap();
    }
    assert_eq!(plain.trim_circular_head("z", "term").unwrap(), 0);
    assert_eq!(parts_held(&plain, "z", "term"), 6);

    let roomy = circular_store(10 * PART);
    for _ in 0..6 {
        roomy.append_data_at("z", "term", &vec![b'x'; PART as usize]).unwrap();
    }
    assert_eq!(roomy.trim_circular_head("z", "term").unwrap(), 0);
    assert_eq!(roomy.trim_circular_head("z", "missing").unwrap(), 0, "no such file is not an error");
}

#[test]
fn an_existing_file_becomes_circular_in_place_and_the_change_is_seen_at_once() {
    let store = FileStore::open_in_memory().unwrap();
    store.make_file("z", "term", FileMeta::new(), FileOpts::default()).unwrap();
    for _ in 0..9 {
        store.append_data_at("z", "term", &vec![b'y'; PART as usize]).unwrap();
    }
    assert!(!store.stat("z", "term").unwrap().unwrap().opts.circular, "cached as plain");
    assert!(store.set_circular("z", "term", 3 * PART).unwrap());
    let file = store.stat("z", "term").unwrap().unwrap();
    assert!(file.opts.circular && file.opts.maxsize == 3 * PART, "the cached row follows");
    assert!(!store.set_circular("z", "term", 3 * PART).unwrap(), "same options: no change");
    assert_eq!(store.trim_circular_head("z", "term").unwrap(), 6);
    assert_eq!(store.read_file("z", "term").unwrap().unwrap().len() as i64, 3 * PART);

    assert!(store.set_circular("z", "term", 0).unwrap(), "0 lifts the limit");
    assert!(!store.stat("z", "term").unwrap().unwrap().opts.circular);
}

#[test]
fn appending_after_a_trim_continues_at_the_absolute_end() {
    let store = circular_store(2 * PART);
    for _ in 0..5 {
        store.append_data_at("z", "term", &vec![b'a'; PART as usize]).unwrap();
    }
    store.trim_circular_head("z", "term").unwrap();
    let at = store.append_data_at("z", "term", b"tail").unwrap();
    assert_eq!(at, 5 * PART);
    store.trim_circular_head("z", "term").unwrap();
    let held = store.read_file("z", "term").unwrap().unwrap();
    assert!(held.ends_with(b"tail"));
    assert_eq!(held.len() as i64, 2 * PART);
}

#[test]
fn widening_or_lifting_the_window_after_a_trim_does_not_read_back_into_deleted_parts() {
    let store = circular_store(3 * PART);
    let chunk: Vec<u8> = (0..PART as usize).map(|i| (i % 249) as u8).collect();
    for _ in 0..10 {
        store.append_data_at("z", "term", &chunk).unwrap();
    }
    store.trim_circular_head("z", "term").unwrap();
    assert_eq!(store.stat("z", "term").unwrap().unwrap().data_start_idx(), 7 * PART);

    // A bigger window: what survives is still 3 parts, starting where it did.
    store.set_circular("z", "term", 20 * PART).unwrap();
    let file = store.stat("z", "term").unwrap().unwrap();
    assert_eq!((file.data_start_idx(), file.data_length()), (7 * PART, 3 * PART));
    let (from, bytes) = store.read_at("z", "term", 0, 0).unwrap();
    assert_eq!((from, bytes.len() as i64), (7 * PART, 3 * PART));
    assert_eq!(bytes, chunk.repeat(3));

    // No limit at all: the same.
    store.set_circular("z", "term", 0).unwrap();
    let (from, bytes) = store.read_at("z", "term", 0, 0).unwrap();
    assert_eq!((from, bytes.len() as i64), (7 * PART, 3 * PART));
    assert_eq!(store.read_file("z", "term").unwrap().unwrap(), chunk.repeat(3));

    // And it keeps growing from there.
    store.append_data_at("z", "term", b"tail").unwrap();
    let (from, bytes) = store.read_at("z", "term", 0, 0).unwrap();
    assert_eq!(from, 7 * PART);
    assert!(bytes.ends_with(b"tail") && bytes.len() as i64 == 3 * PART + 4);
}

#[test]
fn an_untrimmed_file_has_no_retained_start() {
    let store = circular_store(3 * PART);
    store.append_data_at("z", "term", &vec![b'q'; PART as usize]).unwrap();
    store.trim_circular_head("z", "term").unwrap();
    let file = store.stat("z", "term").unwrap().unwrap();
    assert!(file.meta.get("retainedfrom").is_none());
    assert_eq!(file.data_start_idx(), 0);
}

#[test]
fn a_read_never_returns_an_offset_earlier_than_its_body_while_trims_run() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    let store = Arc::new(circular_store(4 * PART));
    let done = Arc::new(AtomicBool::new(false));
    let writer = {
        let (store, done) = (Arc::clone(&store), Arc::clone(&done));
        std::thread::spawn(move || {
            // A distinct byte per part, so a body's first byte names the part it starts in.
            for i in 0..400u32 {
                store.append_data_at("z", "term", &vec![(i % 251) as u8; PART as usize]).unwrap();
                store.trim_circular_head("z", "term").unwrap();
            }
            done.store(true, Ordering::SeqCst);
        })
    };
    while !done.load(Ordering::SeqCst) {
        let Ok((from, bytes)) = store.read_at("z", "term", 0, 0) else { continue };
        if bytes.is_empty() {
            continue;
        }
        assert_eq!(from % PART, 0, "the window starts on a part here");
        assert_eq!(bytes[0] as i64, (from / PART) % 251, "the body starts where the offset says");
        assert_eq!(bytes.len() as i64 % PART, 0);
    }
    writer.join().unwrap();
}
