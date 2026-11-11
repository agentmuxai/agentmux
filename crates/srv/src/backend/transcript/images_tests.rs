// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use crate::backend::storage::filestore::{FileMeta, FileOpts};
use serde_json::json;

const LIST: &str = "look at these\n\n<attached_files>\nThe user attached 4 files.\n1. a.png — C:\\x\\ab\\0123.v1-e2000.send.png\n2. b.pdf [PDF, 2 pages; text version: /x/y.text.txt] — /x/named/ff/b.pdf\n3. d.png — /x/ef.v1-e2000.send.jpg\n- gone.PNG — (no longer available)\n- gone.txt — (no longer available)\n</attached_files>";

fn user_line(content: Value) -> Vec<u8> {
    let mut l = json!({ "type": "user", "message": { "role": "user", "content": content } }).to_string().into_bytes();
    l.push(b'\n');
    l
}

#[test]
fn a_user_message_counts_its_found_and_its_missing_images_and_no_other_file() {
    assert_eq!(images_in_line(&user_line(json!(LIST))), 3);
    assert_eq!(images_in_line(&user_line(json!([{ "type": "text", "text": LIST }]))), 3, "content blocks too");
    assert_eq!(images_in_line(&user_line(json!("no attachments"))), 0);
}

#[test]
fn only_a_user_lines_list_counts() {
    let said = json!({ "type": "assistant", "message": { "content": [{ "type": "text", "text": LIST }] } }).to_string();
    assert_eq!(images_in_line(said.as_bytes()), 0, "the agent quoting a list is not an image in the conversation");
    assert_eq!(images_in_line(b"not json <attached_files>"), 0);
}

#[test]
fn the_transcript_is_counted_whole_and_then_only_from_where_the_last_count_stopped() {
    let gfs = FileStore::open_in_memory().unwrap();
    let zone = "agent:count-images:current";
    assert_eq!(count_in_zone(&gfs, zone).unwrap(), 0, "no transcript");
    gfs.make_file(zone, FILE, FileMeta::new(), FileOpts::default()).unwrap();
    gfs.append_data(zone, FILE, &user_line(json!(LIST))).unwrap();
    gfs.append_data(zone, FILE, b"{\"type\":\"assistant\",\"text\":\"ok\"}\n").unwrap();
    assert_eq!(count_in_zone(&gfs, zone).unwrap(), 3);
    assert_eq!(count_in_zone(&gfs, zone).unwrap(), 3, "counted again without a read, the same");
    gfs.append_data(zone, FILE, &user_line(json!(LIST))).unwrap();
    assert_eq!(count_in_zone(&gfs, zone).unwrap(), 6, "the new line is added to the old count");
}

#[test]
fn counted_answers_without_reading_and_warm_fills_it_in_the_background() {
    let gfs = Arc::new(FileStore::open_in_memory().unwrap());
    let zone = "agent:warm-images:current";
    assert_eq!(counted(zone), None, "nothing counted yet");
    gfs.make_file(zone, FILE, FileMeta::new(), FileOpts::default()).unwrap();
    gfs.append_data(zone, FILE, &user_line(json!(LIST))).unwrap();
    assert_eq!(counted(zone), None, "asking does not count");
    warm(&gfs, zone);
    warm(&gfs, zone);
    let start = std::time::Instant::now();
    while counted(zone).is_none() && start.elapsed() < std::time::Duration::from_secs(10) {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(counted(zone), Some(3));
}

#[test]
fn a_transcript_made_again_after_an_archive_is_counted_from_its_top() {
    let gfs = FileStore::open_in_memory().unwrap();
    let zone = "agent:count-images-archived:current";
    gfs.make_file(zone, FILE, FileMeta::new(), FileOpts::default()).unwrap();
    gfs.append_data(zone, FILE, &user_line(json!(LIST))).unwrap();
    assert_eq!(count_in_zone(&gfs, zone).unwrap(), 3);
    // The archive deletes the file; the next conversation makes it again, and grows it past where the count stopped.
    gfs.delete_file(zone, FILE).unwrap();
    assert_eq!(count_in_zone(&gfs, zone).unwrap(), 0, "no transcript");
    std::thread::sleep(std::time::Duration::from_millis(5));
    gfs.make_file(zone, FILE, FileMeta::new(), FileOpts::default()).unwrap();
    for _ in 0..3 {
        gfs.append_data(zone, FILE, &user_line(json!("a longer message without any attachment, to pass the old position"))).unwrap();
    }
    assert_eq!(count_in_zone(&gfs, zone).unwrap(), 0, "nothing of the old conversation");
    gfs.append_data(zone, FILE, &user_line(json!(LIST))).unwrap();
    assert_eq!(count_in_zone(&gfs, zone).unwrap(), 3);
}

#[test]
fn a_transcript_rewritten_in_place_is_counted_again_once_it_is_forgotten() {
    let gfs = FileStore::open_in_memory().unwrap();
    let zone = "agent:count-images-rewritten:current";
    gfs.make_file(zone, FILE, FileMeta::new(), FileOpts::default()).unwrap();
    gfs.append_data(zone, FILE, &user_line(json!(LIST))).unwrap();
    assert_eq!(count_in_zone(&gfs, zone).unwrap(), 3);
    // A repair rewrites `output` as longer bytes with the same creation time: the old position points into other content.
    let mut rewritten = user_line(json!("a message that holds no attachment at all, and is longer than the old file was"));
    rewritten.extend(user_line(json!(LIST)));
    gfs.replace_file(zone, FILE, &rewritten, &[]).unwrap();
    forget(zone);
    assert_eq!(count_in_zone(&gfs, zone).unwrap(), 3);
}
