// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `update()` over every state and every event, for the current generation and for a
//! stale one: the state it moves to and the effects it asks for, one line per case in
//! `persistent_resume_transitions.txt`.
//!
//! Written before `update` was split into one function per event, to hold the whole
//! transition table still. After an intended change to the machine, regenerate the
//! file with `UPDATE_TRANSITIONS=1 cargo test -p agentmux-srv --bins -- persistent_resume`
//! and read the diff.

use super::*;

const CURRENT: u64 = 1;
const STALE: u64 = 2;

fn config() -> PersistentSpawnConfig {
    PersistentSpawnConfig {
        cli_command: "not-a-real-binary".to_string(),
        cli_args: vec![],
        working_dir: String::new(),
        env_vars: std::collections::HashMap::new(),
        session_id_field: "session_id".to_string(),
        resume_flag: "--resume".to_string(),
        session_id: "dead-sid".to_string(),
        message_id: None,
    }
}

fn entry(seq: u64, json: &str) -> QueuedRetryEntry {
    QueuedRetryEntry { seq, json: json.to_string() }
}

fn retry() -> RetryPayload {
    RetryPayload { config: config(), messages: vec![entry(1, "seed")] }
}

fn awaiting(held: Option<&str>, stop: bool) -> ResumeState {
    ResumeState::AwaitingOutcome { generation: CURRENT, attempted_sid: "old".into(), retry: retry(), held_error_line: held.map(String::from), stop_requested: stop }
}

fn confirmed(held: Option<&str>, stop: bool) -> ResumeState {
    ResumeState::ConfirmedRetry { generation: CURRENT, attempted_sid: "old".into(), retry: retry(), held_error_line: held.map(String::from), stop_requested: stop }
}

fn states() -> Vec<ResumeState> {
    let mut all = vec![ResumeState::NotTracking { current_generation: CURRENT }];
    for held in [None, Some("HELD")] {
        for stop in [false, true] {
            all.push(awaiting(held, stop));
            all.push(confirmed(held, stop));
        }
    }
    all
}

/// `(label, event)`: each event kind for a generation that matches and one that doesn't.
fn events() -> Vec<(String, ResumeEvent)> {
    let mut all: Vec<(String, ResumeEvent)> = vec![
        ("SpawnedWithResume(gen 2)".into(), ResumeEvent::SpawnedWithResume { generation: STALE, attempted_sid: "new".into(), retry: retry() }),
        ("SpawnedFresh(gen 2)".into(), ResumeEvent::SpawnedFresh { generation: STALE }),
    ];
    for (which, generation) in [("current", CURRENT), ("stale", STALE)] {
        for (sid, success) in [("old", false), ("old", true), ("other", false), ("other", true)] {
            all.push((
                format!("SessionCaptured({which}, sid {sid}, success {success})"),
                ResumeEvent::SessionCaptured { generation, sid: sid.into(), is_confirmed_success: success },
            ));
        }
        for (name, seq) in [("after", 2), ("before", 0)] {
            all.push((
                format!("MessageAppended({which}, seq {seq} {name} the seed)"),
                ResumeEvent::MessageAppendedToRetryBatch { generation, entry: entry(seq, "later") },
            ));
        }
        for sid in ["old", "other"] {
            all.push((format!("ResumeUnreachable({which}, sid {sid})"), ResumeEvent::ResumeUnreachable { generation, sid: sid.into() }));
        }
        all.push((format!("ErrorResultLine({which})"), ResumeEvent::ErrorResultLine { generation, line: "NEW".into() }));
        all.push((format!("ProcessExited({which})"), ResumeEvent::ProcessExited { generation }));
        all.push((format!("StopRequested({which})"), ResumeEvent::StopRequested { generation }));
    }
    all
}

fn retry_text(r: &RetryPayload) -> String {
    r.messages.iter().map(|m| format!("{}:{}", m.seq, m.json)).collect::<Vec<_>>().join(",")
}

fn opt(line: &Option<String>) -> String {
    line.as_deref().unwrap_or("-").to_string()
}

fn state_text(s: &ResumeState) -> String {
    match s {
        ResumeState::NotTracking { current_generation } => format!("NotTracking(gen {current_generation})"),
        ResumeState::AwaitingOutcome { generation, attempted_sid, retry, held_error_line, stop_requested } => {
            format!("Awaiting(gen {generation} sid {attempted_sid} retry [{}] held {} stop {stop_requested})", retry_text(retry), opt(held_error_line))
        }
        ResumeState::ConfirmedRetry { generation, attempted_sid, retry, held_error_line, stop_requested } => {
            format!("Confirmed(gen {generation} sid {attempted_sid} retry [{}] held {} stop {stop_requested})", retry_text(retry), opt(held_error_line))
        }
    }
}

fn effect_text(e: &ResumeEffect) -> String {
    match e {
        ResumeEffect::PersistImmediately(l) => format!("Persist({l})"),
        ResumeEffect::FlushErrorLine(l) => format!("Flush({l})"),
        ResumeEffect::FireRetry { retry, held_error_line, attempted_sid } => format!("FireRetry([{}] held {} sid {attempted_sid})", retry_text(retry), opt(held_error_line)),
        ResumeEffect::PublishDone => "PublishDone".to_string(),
        ResumeEffect::EmitSessionOutcome { outcome, attempted_sid, actual_sid } => {
            format!("Outcome({outcome:?} attempted {attempted_sid} actual {})", actual_sid.as_deref().unwrap_or("-"))
        }
    }
}

fn table() -> String {
    let mut out = String::new();
    for state in states() {
        for (label, event) in events() {
            let before = state_text(&state);
            let (next, effects) = update(state.clone(), event);
            let effects = effects.iter().map(effect_text).collect::<Vec<_>>().join(" ; ");
            out.push_str(&format!("{before} + {label} => {} | {}\n", state_text(&next), if effects.is_empty() { "-" } else { &effects }));
        }
    }
    out
}

#[test]
fn the_transition_table_is_unchanged() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/backend/blockcontroller/persistent_resume_transitions.txt");
    let now = table();
    if std::env::var_os("UPDATE_TRANSITIONS").is_some() {
        std::fs::write(&path, &now).unwrap();
        return;
    }
    let want = std::fs::read_to_string(&path).expect("persistent_resume_transitions.txt");
    for (i, (a, b)) in now.lines().zip(want.lines()).enumerate() {
        assert_eq!(a, b, "line {} of persistent_resume_transitions.txt", i + 1);
    }
    assert_eq!(now.lines().count(), want.lines().count(), "the table has as many cases as the file");
}
