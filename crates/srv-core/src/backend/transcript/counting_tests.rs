// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;

use super::*;
use crate::backend::storage::filestore::{FileMeta, FileOpts};

const ZONE: &str = "agent:count-test:current";

fn store_with(lines: usize) -> Arc<FileStore> {
    let fs = Arc::new(FileStore::open_in_memory().unwrap());
    fs.make_file(ZONE, "output", FileMeta::default(), FileOpts::default()).unwrap();
    let data: String = (0..lines).map(|i| format!("{{\"type\":\"assistant\",\"i\":{i}}}\n")).collect();
    fs.append_lines(ZONE, "output", data.as_bytes()).unwrap();
    fs
}

fn drop_epoch(fs: &FileStore) {
    fs.lock_conn().execute("UPDATE db_wave_file SET gen = NULL, lines = NULL, lines_size = NULL, lines_tail = NULL, lines_modts = NULL, lines_rev = NULL WHERE name = 'output'", []).unwrap();
}

#[test]
fn a_file_without_an_epoch_is_counted_and_a_counted_one_is_left_alone() {
    let fs = store_with(50);
    drop_epoch(&fs);
    assert!(fs.line_state(ZONE, "output").unwrap().unwrap().counted.is_none());

    assert!(ensure_counted(&fs, ZONE, Duration::from_secs(10)));
    let first = fs.line_state(ZONE, "output").unwrap().unwrap().counted.unwrap();
    assert_eq!(first.lines, 50);

    assert!(ensure_counted(&fs, ZONE, Duration::from_secs(10)));
    let again = fs.line_state(ZONE, "output").unwrap().unwrap().counted.unwrap();
    assert_eq!(again.gen, first.gen, "a counted file is not counted again");
}

#[test]
fn readers_of_one_file_share_one_count() {
    let key = "shared-key".to_string();
    let started = Arc::new(AtomicUsize::new(0));
    let (release, gate) = mpsc::channel::<()>();

    let s = started.clone();
    // The first reader starts the work, which holds until released.
    let first = std::thread::spawn({
        let key = key.clone();
        move || {
            run_once(key, Duration::from_secs(10), move || {
                s.fetch_add(1, Ordering::SeqCst);
                gate.recv().unwrap();
            })
        }
    });
    while started.load(Ordering::SeqCst) == 0 {
        std::thread::sleep(Duration::from_millis(5));
    }
    // A second reader finds it running: it does not start the work again, and gives up waiting.
    let s2 = started.clone();
    let second = run_once(key.clone(), Duration::from_millis(50), move || {
        s2.fetch_add(100, Ordering::SeqCst);
    });
    assert!(!second, "the count is still running");
    assert_eq!(started.load(Ordering::SeqCst), 1, "the work ran once");

    release.send(()).unwrap();
    assert!(first.join().unwrap(), "the first reader saw it end");
    // Over: the next reader that needs a count starts its own.
    assert!(run_once(key, Duration::from_secs(10), || {}));
}

#[test]
fn a_count_still_running_leaves_the_file_uncounted_for_the_caller() {
    let fs = store_with(10);
    drop_epoch(&fs);
    // Another reader's count of this file is in progress and has not finished.
    let (release, gate) = mpsc::channel::<()>();
    let key = flight_key(&fs, ZONE);
    let holder = std::thread::spawn(move || {
        run_once(key, Duration::from_secs(10), move || {
            gate.recv().unwrap();
        })
    });
    std::thread::sleep(Duration::from_millis(100));

    assert!(!ensure_counted(&fs, ZONE, Duration::from_millis(50)));
    assert!(fs.line_state(ZONE, "output").unwrap().unwrap().counted.is_none());

    release.send(()).unwrap();
    assert!(holder.join().unwrap());
}
