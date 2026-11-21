// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Which runtime wrote the words of each message a viewer is shown (docs/specs/SPEC_MESSAGES_CARRY_THE_RUNTIME_OF_THEIR_WRITER_2026_11_20.md).
//!
//! A viewer decides what a message may open (a path in its text, an image it names) by where its words came
//! from: words a container wrote may be shown only from the container's own folders. Two kinds of message
//! carry their writer's runtime to the viewer as `amx.writer.runtime` and no more:
//!
//! - A line stamped by srv (`origin_sig::stamp_writer`) already says it, and [`vet_line`](super::origin_sig::vet_line)
//!   keeps it when the stamp verifies.
//! - A line with no verified `amx` in a host agent's fork whose copied history was a container's (or whose
//!   mark does not verify): the words may be a container's, so they are attributed to one.
//! - A message another agent sent (a jekt, a forwarded message): its words are the sender's. A sender that
//!   srv has no record of, or one that runs in a container, counts as a container.
//!
//! Only ever tightening: a message is never made looser than the viewer's own pane, and nothing here marks a
//! message as the host's.

use serde_json::{json, Value};

use crate::backend::agent_session::fork_history::{fork_history_cover, runtime_of_agent};
use crate::backend::agent_session::global_transcript_store;
use crate::backend::storage::filestore::FileStore;
use crate::backend::storage::store::Store;

use super::origin_sig::{vet_lines, WriterRuntime};

/// [`vet_lines`], then each message's writer runtime as above, for the lines of `owner`'s transcript read for a viewer.
/// `block_fs` is the block's own store; a fork's mark is read from the global transcript store when there is one.
pub fn vet_for_viewer(mstore: &Store, block_fs: &FileStore, owner: &str, lines: &mut [String]) {
    vet_lines(owner, lines);
    let gfs = global_transcript_store().map_or(block_fs, |g| g.as_ref());
    let unstamped_container = unstamped_lines_are_a_containers(mstore, gfs, owner);
    attribute_lines(unstamped_container, &|from| runtime_of_agent(mstore, from), lines);
}

/// Whether a line of `owner`'s transcript with no verified `amx` may be a container's: `owner` is a host agent that
/// is a fork, and where its copied history came from is a container's or is not on record (a mark that is not sealed
/// or doesn't verify). An agent that runs in a container is not asked: its whole pane is confined already.
fn unstamped_lines_are_a_containers(mstore: &Store, gfs: &FileStore, owner: &str) -> bool {
    if runtime_of_agent(mstore, owner) != WriterRuntime::Host {
        return false;
    }
    let zone = crate::backend::agent_session::agent_current_zone(owner);
    if gfs.read_file(&zone, crate::backend::agent_session::fork_history::FORK_MARK_FILE).ok().flatten().is_none() {
        return false;
    }
    fork_history_cover(gfs, &zone).is_none_or(|cover| cover.runtime == WriterRuntime::Container)
}

/// Marks, in each of `lines` (already vetted), the messages whose words a container may have written
/// (`amx.writer.runtime = "container"`, with `attributed: true`). `sender` gives the runtime of the agent a
/// message names as its sender. A line that is not one of those is left as it is.
pub fn attribute_lines(unstamped_container: bool, sender: &dyn Fn(&str) -> WriterRuntime, lines: &mut [String]) {
    for line in lines.iter_mut() {
        if !(unstamped_container || line.contains("\"kind\":\"agent\"")) || super::origin_sig::is_stream_delta(line.as_bytes()) {
            continue;
        }
        let Ok(mut record) = serde_json::from_str::<Value>(line.trim_end_matches('\n')) else { continue };
        if !is_a_containers(&record, unstamped_container, sender) {
            continue;
        }
        let Some(map) = record.as_object_mut() else { continue };
        let mut amx = match map.remove("amx") {
            Some(Value::Object(amx)) if amx.get("verified") == Some(&Value::Bool(true)) => amx,
            _ => serde_json::Map::new(),
        };
        amx.insert("writer".to_string(), json!({ "runtime": "container", "attributed": true }));
        map.insert("amx".to_string(), Value::Object(amx));
        if let Ok(mut out) = serde_json::to_string(&record) {
            if line.ends_with('\n') {
                out.push('\n');
            }
            *line = out;
        }
    }
}

fn is_a_containers(record: &Value, unstamped_container: bool, sender: &dyn Fn(&str) -> WriterRuntime) -> bool {
    let amx = record.get("amx").and_then(Value::as_object);
    let verified = amx.is_some_and(|a| a.get("verified") == Some(&Value::Bool(true)));
    if !verified {
        return unstamped_container;
    }
    let writer = amx.and_then(|a| a.get("writer")).and_then(|w| w.get("runtime")).and_then(Value::as_str);
    if writer.is_some() {
        return false;
    }
    let origin = amx.and_then(|a| a.get("origin"));
    let from_an_agent = origin.and_then(|o| o.get("kind")).and_then(Value::as_str) == Some("agent");
    match origin.and_then(|o| o.get("from")).and_then(Value::as_str) {
        Some(from) if from_an_agent => sender(from) == WriterRuntime::Container,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host_but(container: &'static str) -> impl Fn(&str) -> WriterRuntime {
        move |from| if from == container { WriterRuntime::Container } else { WriterRuntime::Host }
    }

    fn run(unstamped_container: bool, sender: &dyn Fn(&str) -> WriterRuntime, line: Value) -> Value {
        let mut lines = [format!("{line}\n")];
        attribute_lines(unstamped_container, sender, &mut lines);
        serde_json::from_str(lines[0].trim_end()).unwrap()
    }

    fn user(from: &str) -> Value {
        json!({ "type": "user", "message": { "content": "see /x.png" }, "amx": { "v": 1, "verified": true, "origin": { "kind": "agent", "from": from } } })
    }

    fn runtime(v: &Value) -> Option<&str> {
        v.pointer("/amx/writer/runtime")?.as_str()
    }

    #[test]
    fn a_message_a_container_agent_sent_is_the_containers_and_one_from_a_host_agent_is_not_marked() {
        let sender = host_but("boxed");
        let sent = run(false, &sender, user("boxed"));
        assert_eq!(runtime(&sent), Some("container"));
        assert_eq!(sent.pointer("/amx/origin/from").unwrap(), "boxed", "the signed origin is kept");
        assert_eq!(sent.pointer("/amx/verified").unwrap(), true);
        assert_eq!(runtime(&run(false, &sender, user("hosted"))), None);
    }

    #[test]
    fn a_sender_that_is_not_on_record_counts_as_a_container_and_a_service_is_left_alone() {
        let unknown = |_: &str| WriterRuntime::Container;
        assert_eq!(runtime(&run(false, &unknown, user("elsewhere"))), Some("container"));
        let mut service = user("github-consumer");
        service["amx"]["origin"]["kind"] = json!("service");
        assert_eq!(runtime(&run(false, &unknown, service)), None);
    }

    #[test]
    fn a_line_with_no_verified_amx_in_a_fork_of_a_container_is_the_containers_and_a_verified_one_is_not_touched() {
        let sender = host_but("none");
        let bare = json!({ "type": "assistant", "message": { "content": "![](/home/me/secret.png)" } });
        assert_eq!(runtime(&run(true, &sender, bare.clone())), Some("container"));
        assert_eq!(runtime(&run(false, &sender, bare)), None, "outside such a fork nothing is added");
        // A forged stamp the vet dropped leaves no `amx`, so it reads as no stamp at all.
        let hosted = json!({ "type": "assistant", "amx": { "v": 1, "verified": true, "writer": { "runtime": "host", "agent": "a" } } });
        assert_eq!(runtime(&run(true, &sender, hosted)), Some("host"), "a verified stamp wins");
        let typed = json!({ "type": "user", "message": { "content": "hi" }, "amx": { "v": 1, "verified": true, "origin": { "kind": "user" } } });
        assert_eq!(runtime(&run(true, &sender, typed)), None, "the person's own line");
    }

    #[test]
    fn a_streaming_delta_is_never_rewritten_and_a_line_keeps_its_trailing_newline() {
        let delta = r#"{"type":"stream_event","event":{"x":1}}"#.to_string();
        let mut lines = [format!("{delta}\n"), "{\"type\":\"assistant\"}".to_string()];
        attribute_lines(true, &|_| WriterRuntime::Host, &mut lines);
        assert_eq!(lines[0], format!("{delta}\n"));
        assert!(lines[1].contains("\"container\"") && !lines[1].ends_with('\n'));
    }

    #[test]
    fn a_line_that_is_not_json_is_left_as_it_is() {
        let mut lines = ["not json".to_string()];
        attribute_lines(true, &|_| WriterRuntime::Container, &mut lines);
        assert_eq!(lines[0], "not json");
    }
}
