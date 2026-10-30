// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What messages cost, counted from the jekt audit file: per agent per day, how many messages came in, how
//! many started a turn or joined one, how many waited in a hold, how many replies were refused as saying
//! nothing, and how many the agent sent (`SPEC_CHEAPER_FLEET_MESSAGING_2026_10_30.md` §7).

use std::collections::BTreeMap;
use std::path::Path;

use serde::Serialize;

use super::jekt_file;
use crate::backend::reactive::types::AuditLogEntry;

const DAY_MS: u64 = 24 * 60 * 60 * 1000;
/// The most days one ask covers; the file is read from its end back to the first of them.
pub const MAX_DAYS: u64 = 31;

/// One agent's messages on one UTC day.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct DayStat {
    /// `YYYY-MM-DD`, UTC.
    pub day: String,
    pub agent: String,
    /// Delivered to the agent.
    pub received: u32,
    /// Of those, ones that started a turn (the pane had none in flight).
    pub turns_started: u32,
    /// Of those, ones that joined a turn already in flight.
    pub joined: u32,
    /// Of those, ones that waited in a hold first (`held_delivered`).
    pub held: u32,
    /// Replies from this agent refused for saying nothing.
    pub acks_refused: u32,
    /// Messages this agent sent that were delivered.
    pub sent: u32,
}

/// `YYYY-MM-DD` of a Unix-millisecond time, UTC.
fn day_of(ms: u64) -> String {
    let days = (ms / DAY_MS) as i64;
    // Civil-from-days (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

/// The days' counts of `entries` newer than `since_ms`, oldest day first.
pub fn count(entries: impl IntoIterator<Item = AuditLogEntry>, since_ms: u64) -> Vec<DayStat> {
    let mut by: BTreeMap<(String, String), DayStat> = BTreeMap::new();
    let mut bump = |agent: &str, ms: u64, f: &mut dyn FnMut(&mut DayStat)| {
        let day = day_of(ms);
        let stat = by.entry((day.clone(), agent.to_lowercase())).or_insert_with(|| DayStat {
            day,
            agent: agent.to_lowercase(),
            ..Default::default()
        });
        f(stat);
    };
    for e in entries.into_iter().filter(|e| e.timestamp >= since_ms && e.event_kind == "delivery") {
        match e.outcome.as_deref() {
            Some("ack_refused") => {
                if let Some(from) = e.source_agent.as_deref() {
                    bump(from, e.timestamp, &mut |s| s.acks_refused += 1);
                }
            }
            None | Some("held_delivered") if e.success => {
                let held = e.outcome.is_some();
                let turn = e.turn.clone();
                bump(&e.target_agent, e.timestamp, &mut |s| {
                    s.received += 1;
                    s.held += u32::from(held);
                    match turn.as_deref() {
                        Some("started") => s.turns_started += 1,
                        Some("joined") => s.joined += 1,
                        _ => {}
                    }
                });
                if let Some(from) = e.source_agent.as_deref().filter(|f| !f.is_empty()) {
                    bump(from, e.timestamp, &mut |s| s.sent += 1);
                }
            }
            _ => {}
        }
    }
    by.into_values().collect()
}

/// The last `days` days (at most [`MAX_DAYS`]) counted from the audit file at `path`.
pub fn from_file(path: &Path, now_ms: u64, days: u64) -> Vec<DayStat> {
    let since = now_ms.saturating_sub(days.clamp(1, MAX_DAYS) * DAY_MS);
    // Newest first, so stop at the first entry older than the window.
    let recent = jekt_file::entries(path, None).map(|(_, _, e)| e).take_while(|e| e.timestamp >= since);
    count(recent, since)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(at: u64, from: &str, to: &str, outcome: Option<&str>, turn: Option<&str>, success: bool) -> AuditLogEntry {
        AuditLogEntry {
            timestamp: at,
            source_agent: Some(from.into()),
            target_agent: to.into(),
            block_id: "b".into(),
            message_hash: String::new(),
            message_length: 0,
            success,
            error_message: None,
            request_id: String::new(),
            outcome: outcome.map(str::to_string),
            reason: None,
            event_kind: "delivery".into(),
            evicted_block: None,
            evicted_agent: None,
            audit_source_uid: String::new(),
            lan_claimed_uid: String::new(),
            wan: None,
            cli_native: None,
            turn: turn.map(str::to_string),
        }
    }

    #[test]
    fn days_are_utc_dates() {
        assert_eq!(day_of(0), "1970-01-01");
        assert_eq!(day_of(1_793_348_000_000), "2026-10-30");
        assert_eq!(day_of(DAY_MS * 59 + 5), "1970-03-01");
    }

    #[test]
    fn counts_turns_started_joined_held_refused_and_sent_per_agent_per_day() {
        let d = 1_793_348_000_000;
        let stats = count(
            vec![
                entry(d, "AgentX", "Agent1", None, Some("started"), true),
                entry(d + 1, "AgentX", "Agent1", None, Some("joined"), true),
                entry(d + 2, "AgentY", "Agent1", Some("held_delivered"), Some("started"), true),
                entry(d + 3, "Agent1", "AgentX", Some("ack_refused"), None, false),
                entry(d + 4, "AgentX", "Agent1", None, None, false),
                entry(d - 2 * DAY_MS, "AgentX", "Agent1", None, Some("started"), true),
            ],
            d - DAY_MS,
        );
        let agent1 = stats.iter().find(|s| s.agent == "agent1").unwrap();
        assert_eq!((agent1.received, agent1.turns_started, agent1.joined, agent1.held), (3, 2, 1, 1));
        assert_eq!(agent1.acks_refused, 1, "a refused reply counts for the one who tried to send it");
        assert_eq!(stats.iter().find(|s| s.agent == "agentx").unwrap().sent, 2);
        assert_eq!(stats.iter().find(|s| s.agent == "agenty").unwrap().sent, 1);
        assert!(stats.iter().all(|s| s.day == "2026-10-30"), "the entry from two days ago is outside the window");
    }
}
