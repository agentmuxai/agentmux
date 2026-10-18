// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use crate::backend::storage::filestore::{FileMeta, FileOpts};

fn put(fs: &FileStore, zone: &str, bytes: &[u8]) {
    fs.make_file(zone, OUTPUT_FILE, FileMeta::default(), FileOpts::default()).unwrap();
    fs.append_data(zone, OUTPUT_FILE, bytes).unwrap();
}

fn registry(id: &str) -> Option<AgentInfo> {
    match id {
        "parent" => Some(AgentInfo { name: "Parent".into(), provider: "claude".into(), fork_of: String::new() }),
        "fork" => Some(AgentInfo { name: "Parent 2".into(), provider: "claude".into(), fork_of: "parent".into() }),
        _ => None,
    }
}

/// Current and archived history are counted per agent; a fork names its
/// parent; an agent the registry no longer has is listed as deleted.
#[test]
fn agents_lists_every_agent_with_history_current_and_archived() {
    let fs = FileStore::open_in_memory().unwrap();
    put(&fs, "agent:parent:current", b"0123456789\n");
    put(&fs, "agent:parent:archive:1000", b"archived\n");
    put(&fs, "agent:parent:archive:2000", b"older\n");
    put(&fs, "agent:fork:current", b"fork\n");
    put(&fs, "agent:gone:archive:5", b"kept after delete\n");
    // Not an agent's own history: a staging zone, a block zone.
    put(&fs, "agent:parent:archiving:abc", b"staging\n");
    put(&fs, "block-1", b"pane\n");

    let rows = agents(&fs, registry).unwrap();
    let by = |id: &str| rows.iter().find(|r| r.definition_id == id).unwrap();

    assert_eq!(rows.len(), 3, "{rows:?}");
    let parent = by("parent");
    assert_eq!((parent.current_bytes, parent.archives, parent.archive_bytes), (11, 2, 15));
    assert_eq!((parent.name.as_str(), parent.provider.as_str(), parent.deleted), ("Parent", "claude", false));
    let fork = by("fork");
    assert_eq!((fork.fork_of.as_str(), fork.fork_of_name.as_str()), ("parent", "Parent"));
    let gone = by("gone");
    assert!(gone.deleted && gone.name.is_empty());
    assert_eq!((gone.current_bytes, gone.archives), (0, 1));
}

#[test]
fn agent_zone_takes_only_current_and_archive_zones() {
    assert_eq!(agent_zone("agent:a-1:current"), Some(("a-1", false)));
    assert_eq!(agent_zone("agent:a-1:archive:1790000000000"), Some(("a-1", true)));
    assert_eq!(agent_zone("agent:a-1:archive:"), None);
    assert_eq!(agent_zone("agent:a-1:archiving:x"), None);
    assert_eq!(agent_zone("agent::current"), None);
    assert_eq!(agent_zone("block-1"), None);
}
