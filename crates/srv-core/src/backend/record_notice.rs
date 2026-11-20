// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What is said of a flag of the record's rules
//! (docs/specs/SPEC_LONG_RUNNING_ZERO_DEGRADATION_2026_11_16.md §7, task #47820): a quiet note in the status bar,
//! and a note for the machine's steward agent. Nothing is stopped and the person is not interrupted.
//!
//! - The status bar follows the `record-flags` event: the flags still said (`record_flags::still_said`), shaped
//!   like the host's `health-signal` payloads (`kind` is `record`; a `normal` level clears it). It is persisted,
//!   so a window that opens later gets it. It is published when the rules look (once an hour), while there is
//!   something to say and once more when there no longer is.
//! - Each steward running here (an agent whose seat says so, `agent_seat::is_steward`) gets one held note for
//!   the flags a look wrote: it waits for the agent's next turn and starts none, and a later one replaces one
//!   not read yet. With no steward here nobody is told; the flags are in the record all the same.
//!
//! The decisions are pure ([`event`], [`note_text`]), so they are tested without a broker or an agent.

use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::Value;

use super::record_flags::{Flag, Looked};

/// The key of the held note: a later note replaces an earlier one that its agent has not read yet.
pub const NOTE_KEY: &str = "record-flags";

/// One flag that is still said, as the record holds it: its rule, what it is about and its numbers.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct RecordFlagLine {
    /// `stalls`, `long_stall` or `growth`.
    pub rule: String,
    /// What it is about: `ui_stall` or `srv_stall`, or a series (`core`, `core:srv`, `page:<window>:heap`,
    /// `page:<window>:dom`).
    pub key: String,
    /// The end of the hour it was found in (ms since the epoch).
    #[ts(type = "number")]
    pub t: u64,
    /// `stalls`: how many of 5 s or more in the hour.
    #[ts(type = "number | null")]
    pub n: Option<u64>,
    /// `stalls`, `long_stall`: the longest, in ms.
    #[ts(type = "number | null")]
    pub worst_ms: Option<u64>,
    /// `growth`: what `level` and `rise` count: `bytes`, `mb` or `elements`.
    pub unit: Option<String>,
    /// `growth`: the size's level over the last day.
    #[ts(type = "number | null")]
    pub level: Option<u64>,
    /// `growth`: what its trend gained in the last day.
    #[ts(type = "number | null")]
    pub rise: Option<u64>,
}

/// The `record-flags` event's payload (`mps_events::RecordFlags`), shaped like the host's `health-signal`
/// payloads so the status bar's store takes it: `kind` is `record`, and a `normal` level clears the signal.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct RecordFlagsEvent {
    /// Always `record`.
    #[ts(type = "\"record\"")]
    pub kind: String,
    /// `warn` while a flag is said, else `normal`. A flag is never critical: it is a note, not an alarm.
    #[ts(type = "\"normal\" | \"warn\"")]
    pub level: String,
    /// The time of the oldest flag still said; absent while normal.
    #[ts(type = "number | null")]
    pub since_ms: Option<u64>,
    /// The flags still said, oldest first.
    pub flags: Vec<RecordFlagLine>,
}

fn line(e: &Value) -> Option<RecordFlagLine> {
    let n = |field: &str| e[field].as_u64();
    Some(RecordFlagLine {
        rule: e["rule"].as_str()?.to_string(),
        key: e["key"].as_str()?.to_string(),
        t: n("t")?,
        n: n("n"),
        worst_ms: n("worst_ms"),
        unit: e["unit"].as_str().map(str::to_string),
        level: n("level"),
        rise: n("rise"),
    })
}

/// The event for the flags still said (`Looked::said`).
pub fn event(said: &[Value]) -> RecordFlagsEvent {
    let flags: Vec<RecordFlagLine> = said.iter().filter_map(line).collect();
    RecordFlagsEvent {
        kind: "record".to_string(),
        level: if flags.is_empty() { "normal" } else { "warn" }.to_string(),
        since_ms: flags.iter().map(|f| f.t).min(),
        flags,
    }
}

/// What a flag is about, in a few words, for the steward's note.
fn about(flag: &Flag) -> String {
    let stalled = if flag.key == "ui_stall" { "the window host's UI thread" } else { "srv's runtime" };
    let n = |field: &str| flag.fields[field].as_u64().unwrap_or(0);
    match flag.rule {
        "stalls" => format!("{stalled} stalled {} times for 5 s or more in one hour (the longest {} s)", n("n"), n("worst_ms") / 1000),
        "long_stall" => format!("{stalled} stalled for {} s", n("worst_ms") / 1000),
        "growth" => format!("{} has grown two days running, by more than a fifth of its size each day", flag.key),
        other => format!("{other}: {}", flag.key),
    }
}

/// What the stewards are told about the flags a look wrote; nothing when it wrote none.
pub fn note_text(new: &[Flag]) -> Option<String> {
    if new.is_empty() {
        return None;
    }
    let what: Vec<String> = new.iter().map(about).collect();
    Some(format!(
        "[AgentMux] The record of this AgentMux flagged {}: {}. \
         Nothing was stopped and the person was not interrupted; the status bar shows a note. \
         `App record` (view \"summary\") has the numbers and the days behind them. \
         If it is real, file it for the seat that owns it, with the export of those days.",
        if new.len() == 1 { "one thing".to_string() } else { format!("{} things", new.len()) },
        what.join("; ")
    ))
}

/// The stewards running here, as a note is addressed to them (uid, else id), each once.
fn steward_targets(mstore: &crate::backend::storage::store::Store) -> Vec<String> {
    let defs = mstore.agent_def_list().unwrap_or_default();
    let mut out: Vec<String> = Vec::new();
    for rec in crate::backend::reactive::get_global_handler().list_agents() {
        let def = defs.iter().find(|d| rec.uid.as_deref() == Some(d.id.as_str()) || d.slug == rec.agent_id || d.id == rec.agent_id);
        if !def.and_then(|d| crate::backend::agent_seat::effective_seat(mstore, d)).is_some_and(|seat| crate::backend::agent_seat::is_steward(&seat)) {
            continue;
        }
        let target = rec.uid.clone().filter(|u| !u.is_empty()).unwrap_or(rec.agent_id.clone());
        if !out.contains(&target) {
            out.push(target);
        }
    }
    out
}

/// Whether the last event published said anything: one more is published when it no longer does.
static SAID: AtomicBool = AtomicBool::new(false);

/// Say what a look found: the event for the status bar, then the note for the stewards. BLOCKING (the note takes
/// the message handler's lock); call it where the look ran, off the async workers. The event goes first, so a
/// handler that cannot be reached costs the note and not the status bar.
pub fn tell(broker: &crate::backend::mps::Broker, mstore: &crate::backend::storage::store::Store, looked: &Looked) {
    let event = event(&looked.said);
    let says = !event.flags.is_empty();
    let said_before = SAID.swap(says, Ordering::Relaxed);
    if says || said_before {
        broker.publish_event::<crate::backend::mps_events::RecordFlags>(&event, vec![], 1);
    }
    let Some(text) = note_text(&looked.new) else { return };
    let handler = crate::backend::reactive::get_global_handler();
    let targets = steward_targets(mstore);
    if targets.is_empty() {
        tracing::info!("[record] flags were written and no steward is here to be told");
    }
    for target in targets {
        match handler.hold_system_note(&target, NOTE_KEY, &text) {
            Ok(()) => tracing::info!(agent = %target, "[record] the steward has a note about the flags"),
            Err(e) => tracing::debug!(agent = %target, error = %e, "[record] the steward was not told"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 2026-11-16 12:00 UTC.
    const END: u64 = 1_794_830_400_000;

    #[test]
    fn the_event_carries_each_flag_still_said_with_its_numbers_and_is_normal_with_none() {
        let said = vec![
            json!({ "t": END - 3_600_000, "kind": "flag", "rule": "growth", "key": "page:main:heap", "unit": "mb", "level": 306, "rise": 72, "rise_before": 70, "content_pct": 2 }),
            json!({ "t": END, "kind": "flag", "rule": "stalls", "key": "srv_stall", "n": 4, "worst_ms": 6_200 }),
            // A line that is not a whole flag is left out.
            json!({ "t": END, "kind": "flag", "rule": "growth" }),
        ];
        let e = event(&said);
        assert_eq!((e.kind.as_str(), e.level.as_str(), e.since_ms), ("record", "warn", Some(END - 3_600_000)));
        assert_eq!(e.flags.len(), 2);
        assert_eq!(
            e.flags[0],
            RecordFlagLine { rule: "growth".into(), key: "page:main:heap".into(), t: END - 3_600_000, n: None, worst_ms: None, unit: Some("mb".into()), level: Some(306), rise: Some(72) }
        );
        assert_eq!((e.flags[1].rule.as_str(), e.flags[1].n, e.flags[1].worst_ms), ("stalls", Some(4), Some(6_200)));

        let none = event(&[]);
        assert_eq!((none.level.as_str(), none.since_ms, none.flags.len()), ("normal", None, 0));
        // As the page reads it: the shape of a health signal.
        let wire = serde_json::to_value(&e).unwrap();
        assert_eq!(wire["kind"], "record");
        assert_eq!(wire["flags"][1]["worst_ms"], 6_200);
    }

    #[test]
    fn the_steward_is_told_once_for_what_a_look_wrote_and_nothing_for_a_look_that_wrote_nothing() {
        assert_eq!(note_text(&[]), None);
        let one = note_text(&[Flag { rule: "long_stall", key: "ui_stall".into(), fields: json!({ "n": 1, "worst_ms": 41_000 }) }]).unwrap();
        assert!(one.starts_with("[AgentMux] The record of this AgentMux flagged one thing: the window host's UI thread stalled for 41 s."), "{one}");
        let two = note_text(&[
            Flag { rule: "stalls", key: "srv_stall".into(), fields: json!({ "n": 4, "worst_ms": 6_200 }) },
            Flag { rule: "growth", key: "core:srv".into(), fields: json!({ "unit": "bytes", "level": 300, "rise": 90 }) },
        ])
        .unwrap();
        assert!(two.contains("flagged 2 things: srv's runtime stalled 4 times for 5 s or more in one hour (the longest 6 s); core:srv has grown two days running"), "{two}");
        assert!(two.contains("`App record`"));
        assert!(two.contains("Nothing was stopped"));
    }
}
