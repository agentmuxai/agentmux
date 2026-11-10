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
