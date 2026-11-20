// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Telling an agent how the upgrade it asked another machine for ended.
//!
//! An agent that files `app.upgrade` on another machine gets the request's id and no more: the person
//! there may veto the countdown, the restart may not be confirmed, the release may not verify, or the
//! machine may restart into the new version, and until now the agent learned that only by asking again.
//! This module reads how the request stands for it and tells it once, as an `[AgentMux]` system note,
//! never as a message from an agent.
//!
//! It reads, it is not told. The other machine has no way to speak as one of this machine's agents, and
//! nothing here gives it one: this srv holds the requesting agent's LAN key and asks `app.upgrade` with
//! `status` and `app.version` for it, the same two reads the agent can make itself. So the other machine
//! can say nothing the requester could not have asked, and the note is built here from words this file
//! knows, with the machine's own text left out.
//!
//! The note goes only to the agent that asked, from the machine it asked, once. An agent that is not
//! running is tried again at each read until it can take it, and the watch ends after a day with nothing
//! said. It is kept in memory: a restart of this srv forgets it, and the agent can still read the status.

use std::collections::HashSet;
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

use serde_json::{json, Value};

use super::op::{OpStatus, RemoteOpResult};
use super::send;
use crate::server::AppState;

/// Between reads while the request is young: a veto or a restart is a matter of minutes.
const POLL_YOUNG: Duration = Duration::from_secs(20);
/// Between reads after the first quarter hour, to the end of the day: each read is an audit row on both machines.
const POLL_OLD: Duration = Duration::from_secs(300);
const YOUNG_FOR: Duration = Duration::from_secs(15 * 60);
/// After this the agent is not told at all.
const GIVE_UP_AFTER: Duration = Duration::from_secs(24 * 60 * 60);
/// Watches at once, so a flood of requests cannot become a flood of reads.
const MAX_WATCHES: usize = 32;

const READ_REASON: &str = "read how my upgrade request ended, to tell the agent that asked";

/// How a request on the other machine ended, as far as an agent is told.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Ended {
    /// The super agent said no.
    Denied,
    /// The person stopped it in the countdown.
    Vetoed,
    /// Withdrawn.
    Cancelled,
    /// Nobody decided in time.
    Expired,
    /// The operation could not go ahead.
    Failed,
    /// The release did not download or did not verify.
    NotDownloaded,
    /// The restart was asked for and did not go ahead; `why` is one of the status's words, `at_ms` its time.
    NotConfirmed { why: String, at_ms: u64 },
    /// The machine came back on the release.
    Restarted,
}

/// Where a request stands while it has not ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Phase {
    /// Asked, counting down, or the operation is starting.
    Open,
    /// The operation ran: the release downloads, then the machine restarts into it.
    Ran,
}

/// What one read of the request's status says to do next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Step {
    Wait(Phase),
    /// The request is gone from the machine, or the machine does not answer, after the operation ran: it may
    /// have restarted, which its version says.
    CheckVersion,
    End(Ended),
}

/// One read of `app.upgrade` with `status`, in `phase`, for the release `version`.
pub(super) fn read_status(phase: Phase, answer: &RemoteOpResult, version: &str) -> Step {
    if answer.status != OpStatus::Done {
        return if phase == Phase::Ran { Step::CheckVersion } else { Step::Wait(phase) };
    }
    let r = &answer.result;
    match r.get("status").and_then(Value::as_str).unwrap_or_default() {
        "asked" | "pending" | "running" => Step::Wait(Phase::Open),
        "denied" => Step::End(Ended::Denied),
        "vetoed" => Step::End(Ended::Vetoed),
        "cancelled" => Step::End(Ended::Cancelled),
        "expired" => Step::End(Ended::Expired),
        "failed" => Step::End(Ended::Failed),
        "done" => {
            let restart = &r["restart"];
            let this_release = restart["version"].as_str() == Some(version);
            match (restart["state"].as_str(), restart["not_confirmed"].as_str(), restart["not_confirmed_at"].as_u64()) {
                (Some("ready"), Some(why), Some(at_ms)) if this_release => Step::End(Ended::NotConfirmed { why: why.to_string(), at_ms }),
                (Some("failed"), _, _) if this_release => Step::End(Ended::NotDownloaded),
                _ => Step::Wait(Phase::Ran),
            }
        }
        _ => Step::Wait(phase),
    }
}

/// One read of `app.version`: whether the machine now runs `version`.
pub(super) fn read_version(answer: &RemoteOpResult, version: &str) -> bool {
    answer.status == OpStatus::Done && answer.result["version"].as_str() == Some(version)
}

/// `ms` (Unix) as "16:33 UTC".
fn clock(ms: u64) -> String {
    chrono::DateTime::from_timestamp_millis(ms as i64).map_or_else(|| "an unknown time".into(), |t| t.format("%H:%M UTC").to_string())
}

fn why_words(why: &str) -> &'static str {
    match why {
        "no" => "the person answered No",
        "timed_out" => "nobody answered the prompt before it closed itself",
        "too_late" => "an answer came after the prompt had lapsed",
        "unanswered" => "the launcher did not restart AgentMux in time",
        _ => "the restart was declined",
    }
}

/// The note: which machine, which release, how it ended and when. Every word is this file's own, but for the
/// machine's name and the release, which are quoted like any text a note carries that srv did not write.
pub(super) fn note_for(host: &str, version: &str, ended: &Ended, seen_ms: u64) -> String {
    use crate::backend::reactive::sanitize::quote_in_system_note as q;
    let (host, version) = (q(host), q(version));
    let at = clock(seen_ms);
    match ended {
        Ended::Denied => format!("[AgentMux] Your request to upgrade {host} to {version} was denied (seen at {at}). Nothing was installed."),
        Ended::Vetoed => format!("[AgentMux] The person at {host} stopped your request to upgrade it to {version} (seen at {at}). Nothing was installed."),
        Ended::Cancelled => format!("[AgentMux] Your request to upgrade {host} to {version} was withdrawn (seen at {at})."),
        Ended::Expired => format!("[AgentMux] Nobody decided your request to upgrade {host} to {version} in time (seen at {at}). Nothing was installed."),
        Ended::Failed => format!("[AgentMux] Your request to upgrade {host} to {version} could not go ahead (seen at {at}). Read it with App upgrade, host {host}, for the reason."),
        Ended::NotDownloaded => format!("[AgentMux] {host} could not download or verify {version} (seen at {at}), so it was not installed. Ask again if it is still wanted."),
        Ended::NotConfirmed { why, at_ms } => format!(
            "[AgentMux] The upgrade of {host} to {version} did not happen: the restart did not go ahead at {}: {}. The release is downloaded and waiting there. Ask again if it is still wanted.",
            clock(*at_ms),
            why_words(why)
        ),
        Ended::Restarted => format!("[AgentMux] {host} restarted and now runs {version} (seen at {at})."),
    }
}

/// One request being followed.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) struct Watch {
    /// The requesting agent, by name: the key that signs the reads.
    pub slug: String,
    /// Its uid on this machine: whom the note goes to.
    pub uid: String,
    pub host: String,
    pub id: String,
    pub version: String,
}

static WATCHING: LazyLock<Mutex<HashSet<(String, String, String)>>> = LazyLock::new(Default::default);

fn key(w: &Watch) -> (String, String, String) {
    (w.uid.clone(), w.host.to_ascii_lowercase(), w.id.clone())
}

/// Start following `w` in `set`: false when it already is, or when `max` are.
fn register_in(set: &mut HashSet<(String, String, String)>, w: &Watch, max: usize) -> bool {
    if set.len() >= max {
        return false;
    }
    set.insert(key(w))
}

fn register(w: &Watch) -> bool {
    register_in(&mut WATCHING.lock().unwrap_or_else(|e| e.into_inner()), w, MAX_WATCHES)
}

fn unregister(w: &Watch) {
    WATCHING.lock().unwrap_or_else(|e| e.into_inner()).remove(&key(w));
}

/// The watch an answer to `app.upgrade` starts, if it is the answer to filing a request: not a read of one
/// (`status`), and `requested` with the request's id.
pub(super) fn watch_for(slug: &str, uid: &str, host: &str, params: &Value, answer: &RemoteOpResult) -> Option<Watch> {
    if params.get("status").is_some() || answer.status != OpStatus::Done {
        return None;
    }
    let r = &answer.result;
    if r["status"].as_str() != Some("requested") {
        return None;
    }
    let (id, version) = (r["id"].as_str()?.trim(), r["version"].as_str()?.trim());
    if id.is_empty() || version.is_empty() {
        return None;
    }
    Some(Watch { slug: slug.to_string(), uid: uid.to_string(), host: host.trim().to_string(), id: id.to_string(), version: version.to_string() })
}

/// Called with every answer to a remote `app.upgrade` this machine's agents get: follows a request just filed.
pub(super) fn note_asked(state: &AppState, slug: &str, uid: &str, host: &str, params: &Value, answer: &RemoteOpResult) {
    let Some(w) = watch_for(slug, uid, host, params, answer) else { return };
    if !register(&w) {
        return;
    }
    tracing::info!(agent = %w.slug, host = %w.host, request = %w.id, "upgrade: following a request on another machine to tell its agent how it ends");
    let state = state.clone();
    tokio::spawn(async move {
        run(&state, &w).await;
        unregister(&w);
    });
}

/// Tell `w`'s agent: whether it was delivered (an agent that is not running is not an error: it is tried again).
fn tell(w: &Watch, note: &str) -> bool {
    crate::sagas::sensitive_window::tell_one(&w.uid, note)
}

async fn run(state: &AppState, w: &Watch) {
    let began = tokio::time::Instant::now();
    let mut phase = Phase::Open;
    let mut ended: Option<Ended> = None;
    loop {
        let waited = began.elapsed();
        if waited >= GIVE_UP_AFTER {
            tracing::info!(agent = %w.slug, host = %w.host, request = %w.id, "upgrade: gave up telling the agent how its request on another machine ended");
            return;
        }
        tokio::time::sleep(if waited < YOUNG_FOR { POLL_YOUNG } else { POLL_OLD }).await;
        if ended.is_none() {
            let answer = send::call(state, &w.slug, &w.host, "app.upgrade", json!({ "status": w.id }), READ_REASON).await;
            match read_status(phase, &answer, &w.version) {
                Step::Wait(next) => phase = next,
                Step::End(e) => ended = Some(e),
                Step::CheckVersion => {
                    let version = send::call(state, &w.slug, &w.host, "app.version", json!({}), READ_REASON).await;
                    if read_version(&version, &w.version) {
                        ended = Some(Ended::Restarted);
                    } else {
                        phase = Phase::Ran;
                    }
                }
            }
        }
        if let Some(e) = &ended {
            if tell(w, &note_for(&w.host, &w.version, e, agentmux_common::time::now_ms_u64())) {
                return;
            }
        }
    }
}

#[cfg(test)]
#[path = "upgrade_watch_tests.rs"]
mod tests;
