// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Jekts held for an absent agent — `SPEC_DURABLE_JEKT_DELIVERY_2026_09_24.md`
//! Phase 1. A message to a known agent of this channel that no delivery tier
//! could take is kept here for up to 24 h and replayed when the agent
//! registers. The trust verdicts it was accepted with are explicit columns:
//! on `InjectionRequest` they are `skip_deserializing`, so they would not
//! survive a JSON round-trip.

use rusqlite::{params, OptionalExtension, Row};

use super::error::StoreError;
use super::store::Store;

/// Held messages per target UID, and per channel (§2.2).
pub const HELD_PER_TARGET: i64 = 64;
pub const HELD_PER_CHANNEL: i64 = 1000;
/// How long a message is held (§2.1).
pub const HELD_TTL_MS: i64 = 24 * 60 * 60 * 1000;
/// The most file bytes held for one agent for messages from other machines: the held rows keep the
/// files in the attachment store for the hold, so what a verified peer can park there is bounded.
pub const HELD_PEER_FILE_BYTES_PER_TARGET: u64 = 512 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq)]
pub struct HeldJekt {
    pub request_id: String,
    pub target_uid: String,
    pub target_agent: String,
    pub source_agent: String,
    pub audit_source_uid: String,
    pub message: String,
    pub priority: String,
    pub jekt_tier: String,
    pub delivery_tier: String,
    pub sig_verified: Option<bool>,
    pub reagent_verified: Option<bool>,
    pub lan_verified: Option<bool>,
    pub channel_verified: Option<bool>,
    /// The accept-time transcript-request fields, restored as they were: a
    /// replay addresses the target by UID, and recomputing them by slug
    /// would lose a forced escalation (review of #3632).
    pub is_transcript_request: bool,
    pub transcript_request_escalate_forced: bool,
    /// The files sent with it, as the request's manifest JSON; '' for a
    /// text-only jekt (SPEC_JEKT_FILE_TRANSFER_2026_10_05.md §4.5). The
    /// files stay in this instance's attachment store meanwhile: a held
    /// jekt always started here.
    pub files_json: String,
    pub sent_at_ms: i64,
    pub expires_at_ms: i64,
    pub attempts: i64,
    pub last_error: String,
    /// `idle` for a routine message held because the target's session went
    /// cold (`backend::cold_hold`); '' for the others, which wait for the
    /// agent to be there at all.
    pub hold_reason: String,
    /// The WAN verdict a message from another machine was accepted with, as JSON
    /// (`{"verified":true,"instance":{...}}`); '' for a host or LAN message. Restored on replay,
    /// so a held message keeps its tier and its proof (the cross-machine operations spec, §4.5).
    pub wan_json: String,
}

impl HeldJekt {
    /// Held because the session was cold, and delivered once it is warm.
    pub fn is_idle(&self) -> bool {
        self.hold_reason == crate::backend::cold_hold::REASON_IDLE
    }
}

/// What [`Store::jekt_held_insert`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HoldOutcome {
    /// Stored now.
    Held,
    /// This `request_id` was already held — nothing stored twice.
    AlreadyHeld,
    /// A cap was reached — nothing stored.
    Full,
}

fn verdict_to_sql(v: Option<bool>) -> Option<i64> {
    v.map(i64::from)
}

fn verdict_from_sql(v: Option<i64>) -> Option<bool> {
    v.map(|n| n != 0)
}

fn row_to_held(row: &Row) -> rusqlite::Result<HeldJekt> {
    Ok(HeldJekt {
        request_id: row.get("request_id")?,
        target_uid: row.get("target_uid")?,
        target_agent: row.get("target_agent")?,
        source_agent: row.get("source_agent")?,
        audit_source_uid: row.get("audit_source_uid")?,
        message: row.get("message")?,
        priority: row.get("priority")?,
        jekt_tier: row.get("jekt_tier")?,
        delivery_tier: row.get("delivery_tier")?,
        sig_verified: verdict_from_sql(row.get("sig_verified")?),
        reagent_verified: verdict_from_sql(row.get("reagent_verified")?),
        lan_verified: verdict_from_sql(row.get("lan_verified")?),
        channel_verified: verdict_from_sql(row.get("channel_verified")?),
        is_transcript_request: row.get::<_, i64>("is_transcript_request")? != 0,
        transcript_request_escalate_forced: row.get::<_, i64>("transcript_request_escalate_forced")? != 0,
        files_json: row.get("files_json")?,
        sent_at_ms: row.get("sent_at_ms")?,
        expires_at_ms: row.get("expires_at_ms")?,
        attempts: row.get("attempts")?,
        last_error: row.get("last_error")?,
        hold_reason: row.get("hold_reason")?,
        wan_json: row.get("wan_json")?,
    })
}

/// The bytes a row's file list names.
fn file_bytes(files_json: &str) -> u64 {
    serde_json::from_str::<Vec<agentmux_common::jekt_files::JektFile>>(files_json)
        .map(|files| files.iter().map(|f| f.size).sum())
        .unwrap_or(0)
}

impl Store {
    /// Hold `held`, unless its `request_id` is already held or a cap is
    /// reached — decided under the one connection lock, so two concurrent
    /// holds cannot both slip under a cap.
    pub fn jekt_held_insert(&self, held: &HeldJekt) -> Result<HoldOutcome, StoreError> {
        let conn = self.conn().lock().unwrap();
        let exists: Option<i64> = conn
            .query_row(
                "SELECT 1 FROM db_jekt_held WHERE request_id = ?1",
                params![held.request_id],
                |r| r.get(0),
            )
            .optional()?;
        if exists.is_some() {
            return Ok(HoldOutcome::AlreadyHeld);
        }
        if matches!(held.delivery_tier.as_str(), "lan" | "wan") && !held.files_json.is_empty() {
            let mut stmt = conn.prepare("SELECT files_json FROM db_jekt_held WHERE target_uid = ?1 AND files_json != '' AND delivery_tier IN ('lan', 'wan')")?;
            let kept: u64 = stmt
                .query_map(params![held.target_uid], |r| r.get::<_, String>(0))?
                .filter_map(Result::ok)
                .map(|json| file_bytes(&json))
                .sum();
            if kept.saturating_add(file_bytes(&held.files_json)) > HELD_PEER_FILE_BYTES_PER_TARGET {
                return Ok(HoldOutcome::Full);
            }
        }
        let n = conn.execute(
            "INSERT INTO db_jekt_held
                (request_id, target_uid, target_agent, source_agent, audit_source_uid,
                 message, priority, jekt_tier, delivery_tier,
                 sig_verified, reagent_verified, lan_verified, channel_verified,
                 is_transcript_request, transcript_request_escalate_forced, files_json,
                 sent_at_ms, expires_at_ms, attempts, last_error, hold_reason, wan_json)
             SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?18, ?19, ?20, ?14, ?15, 0, '', ?21, ?22
              WHERE (SELECT COUNT(*) FROM db_jekt_held WHERE target_uid = ?2) < ?16
                AND (SELECT COUNT(*) FROM db_jekt_held) < ?17",
            params![
                held.request_id,
                held.target_uid,
                held.target_agent,
                held.source_agent,
                held.audit_source_uid,
                held.message,
                held.priority,
                held.jekt_tier,
                held.delivery_tier,
                verdict_to_sql(held.sig_verified),
                verdict_to_sql(held.reagent_verified),
                verdict_to_sql(held.lan_verified),
                verdict_to_sql(held.channel_verified),
                held.sent_at_ms,
                held.expires_at_ms,
                HELD_PER_TARGET,
                HELD_PER_CHANNEL,
                i64::from(held.is_transcript_request),
                i64::from(held.transcript_request_escalate_forced),
                held.files_json,
                held.hold_reason,
                held.wan_json,
            ],
        )?;
        Ok(if n > 0 { HoldOutcome::Held } else { HoldOutcome::Full })
    }

    /// Delete held messages past their expiry; returns how many. A message
    /// held for a cold session is not dropped at its expiry: the replay
    /// delivers it then, so a quiet agent still gets it. It is dropped a
    /// whole TTL later, if its agent never came back to receive it.
    pub fn jekt_held_delete_expired(&self, now_ms: i64) -> Result<usize, StoreError> {
        let conn = self.conn().lock().unwrap();
        Ok(conn.execute(
            "DELETE FROM db_jekt_held
              WHERE (hold_reason != ?2 AND expires_at_ms <= ?1) OR expires_at_ms + ?3 <= ?1",
            params![now_ms, crate::backend::cold_hold::REASON_IDLE, HELD_TTL_MS],
        )?)
    }

    /// Every target UID with a held message, the one waiting longest first.
    pub fn jekt_held_targets(&self) -> Result<Vec<String>, StoreError> {
        let conn = self.conn().lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT target_uid FROM db_jekt_held GROUP BY target_uid ORDER BY MIN(sent_at_ms)",
        )?;
        let rows = stmt.query_map([], |r| r.get(0))?;
        Ok(rows.collect::<Result<Vec<String>, _>>()?)
    }

    /// One target's held messages, oldest first, at most `limit`.
    pub fn jekt_held_for_target(&self, target_uid: &str, limit: usize) -> Result<Vec<HeldJekt>, StoreError> {
        self.jekt_held_ready(target_uid, limit, true, 0)
    }

    /// One target's held messages that are ready to go, oldest first, at most
    /// `limit`: all of them when `warm`, otherwise (its session is still
    /// cold) only those not held for the cold session, and those that have
    /// waited out their expiry at `now_ms`.
    pub fn jekt_held_ready(&self, target_uid: &str, limit: usize, warm: bool, now_ms: i64) -> Result<Vec<HeldJekt>, StoreError> {
        let conn = self.conn().lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT * FROM db_jekt_held
              WHERE target_uid = ?1 AND (?3 OR hold_reason != ?5 OR expires_at_ms <= ?4)
              ORDER BY sent_at_ms ASC, request_id ASC LIMIT ?2",
        )?;
        let rows = stmt.query_map(
            params![target_uid, limit as i64, warm, now_ms, crate::backend::cold_hold::REASON_IDLE],
            row_to_held,
        )?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Drop the held-for-cold copies of a message the sender has now sent
    /// again to wake the agent (`urgent`), so it isn't delivered twice.
    /// Returns how many.
    pub fn jekt_held_delete_idle_copies(&self, target_uid: &str, source_agent: &str, message: &str) -> Result<usize, StoreError> {
        let conn = self.conn().lock().unwrap();
        Ok(conn.execute(
            "DELETE FROM db_jekt_held
              WHERE target_uid = ?1 AND source_agent = ?2 AND message = ?3 AND hold_reason = ?4",
            params![target_uid, source_agent, message, crate::backend::cold_hold::REASON_IDLE],
        )?)
    }

    /// Remove one held message (delivered, or dropped).
    pub fn jekt_held_delete(&self, request_id: &str) -> Result<(), StoreError> {
        let conn = self.conn().lock().unwrap();
        conn.execute("DELETE FROM db_jekt_held WHERE request_id = ?1", params![request_id])?;
        Ok(())
    }

    /// Count a failed replay; returns the new attempt count.
    pub fn jekt_held_record_failure(&self, request_id: &str, error: &str) -> Result<i64, StoreError> {
        let conn = self.conn().lock().unwrap();
        conn.execute(
            "UPDATE db_jekt_held SET attempts = attempts + 1, last_error = ?2 WHERE request_id = ?1",
            params![request_id, error],
        )?;
        Ok(conn
            .query_row(
                "SELECT attempts FROM db_jekt_held WHERE request_id = ?1",
                params![request_id],
                |r| r.get(0),
            )
            .optional()?
            .unwrap_or(0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn held(id: &str, target: &str, sent_at_ms: i64) -> HeldJekt {
        HeldJekt {
            request_id: id.into(),
            target_uid: target.into(),
            target_agent: "agenty".into(),
            source_agent: "sender".into(),
            audit_source_uid: "uid-sender".into(),
            message: "hello".into(),
            priority: "normal".into(),
            jekt_tier: String::new(),
            delivery_tier: "host".into(),
            sig_verified: Some(false),
            reagent_verified: None,
            lan_verified: None,
            channel_verified: Some(true),
            is_transcript_request: true,
            transcript_request_escalate_forced: true,
            files_json: String::new(),
            sent_at_ms,
            expires_at_ms: sent_at_ms + HELD_TTL_MS,
            attempts: 0,
            last_error: String::new(),
            hold_reason: String::new(),
            wan_json: String::new(),
        }
    }

    fn peer_with_files(id: &str, tier: &str, bytes: u64) -> HeldJekt {
        let files = vec![agentmux_common::jekt_files::JektFile { sha256: "ab".repeat(32), name: "a.bin".into(), size: bytes }];
        HeldJekt { delivery_tier: tier.into(), files_json: serde_json::to_string(&files).unwrap(), ..held(id, "uid-y", 1) }
    }

    /// What a verified peer can park in the attachment store for one agent is bounded; a local sender's
    /// files (already in the store, by hash) are not counted.
    #[test]
    fn a_peers_held_files_are_capped_per_agent() {
        const MIB: u64 = 1024 * 1024;
        let s = Store::open_in_memory().unwrap();
        assert_eq!(s.jekt_held_insert(&peer_with_files("p1", "lan", 100 * MIB)).unwrap(), HoldOutcome::Held);
        assert_eq!(s.jekt_held_insert(&peer_with_files("p2", "wan", 200 * MIB)).unwrap(), HoldOutcome::Held);
        assert_eq!(s.jekt_held_insert(&peer_with_files("p3", "lan", 212 * MIB)).unwrap(), HoldOutcome::Held, "512 MiB is the cap, met exactly");
        assert_eq!(s.jekt_held_insert(&peer_with_files("p4", "lan", MIB)).unwrap(), HoldOutcome::Full);
        assert_eq!(s.jekt_held_insert(&peer_with_files("p3", "lan", 212 * MIB)).unwrap(), HoldOutcome::AlreadyHeld);
        assert_eq!(s.jekt_held_insert(&peer_with_files("h1", "host", 200 * MIB)).unwrap(), HoldOutcome::Held, "host-tier files are the sender's own");
        assert_eq!(s.jekt_held_insert(&HeldJekt { delivery_tier: "lan".into(), ..held("t1", "uid-y", 1) }).unwrap(), HoldOutcome::Held, "no files, no weight");
        s.jekt_held_delete("p1").unwrap();
        assert_eq!(s.jekt_held_insert(&peer_with_files("p4", "lan", MIB)).unwrap(), HoldOutcome::Held, "room again once one is delivered");
    }

    #[test]
    fn holds_once_round_trips_verdicts_and_orders_oldest_first() {
        let s = Store::open_in_memory().unwrap();
        assert_eq!(s.jekt_held_insert(&held("r2", "uid-y", 200)).unwrap(), HoldOutcome::Held);
        assert_eq!(s.jekt_held_insert(&held("r1", "uid-y", 100)).unwrap(), HoldOutcome::Held);
        assert_eq!(s.jekt_held_insert(&held("r1", "uid-y", 100)).unwrap(), HoldOutcome::AlreadyHeld);
        let rows = s.jekt_held_for_target("uid-y", 10).unwrap();
        assert_eq!(rows.iter().map(|r| r.request_id.as_str()).collect::<Vec<_>>(), ["r1", "r2"]);
        assert_eq!(rows[0].sig_verified, Some(false), "a failed verdict survives");
        assert_eq!(rows[0].channel_verified, Some(true));
        assert_eq!(rows[0].reagent_verified, None);
        assert!(rows[0].is_transcript_request && rows[0].transcript_request_escalate_forced);
        assert_eq!(s.jekt_held_targets().unwrap(), ["uid-y"]);
    }

    /// Deleting the target agent purges what is held for it, through both
    /// deletion paths (Codex P1 on #3632); another agent's held rows stay.
    #[test]
    fn deleting_the_target_purges_its_held_messages() {
        use crate::backend::storage::agents::test_agent_def;
        for via_instance_delete in [false, true] {
            let s = Store::open_in_memory().unwrap();
            for id in ["uid-y", "uid-z"] {
                let mut def = test_agent_def(id, id, "claude", "agent", 1, "");
                s.agent_def_insert(&mut def).unwrap();
            }
            s.jekt_held_insert(&held("for-y", "uid-y", 1)).unwrap();
            s.jekt_held_insert(&held("for-z", "uid-z", 1)).unwrap();
            let deleted = if via_instance_delete {
                s.instance_delete("uid-y").unwrap()
            } else {
                s.agent_def_delete("uid-y").unwrap()
            };
            assert!(deleted);
            assert!(s.jekt_held_for_target("uid-y", 10).unwrap().is_empty(), "instance_delete={via_instance_delete}");
            assert_eq!(s.jekt_held_for_target("uid-z", 10).unwrap().len(), 1);
        }
    }

    #[test]
    fn caps_refuse_rather_than_overflow() {
        let s = Store::open_in_memory().unwrap();
        for i in 0..HELD_PER_TARGET {
            assert_eq!(s.jekt_held_insert(&held(&format!("r{i}"), "uid-y", i)).unwrap(), HoldOutcome::Held);
        }
        assert_eq!(s.jekt_held_insert(&held("one-more", "uid-y", 999)).unwrap(), HoldOutcome::Full);
        assert_eq!(s.jekt_held_insert(&held("other-target", "uid-z", 999)).unwrap(), HoldOutcome::Held);
    }

    #[test]
    fn expiry_failure_count_and_delete() {
        let s = Store::open_in_memory().unwrap();
        s.jekt_held_insert(&held("old", "uid-y", 0)).unwrap();
        s.jekt_held_insert(&held("new", "uid-y", HELD_TTL_MS)).unwrap();
        assert_eq!(s.jekt_held_delete_expired(HELD_TTL_MS).unwrap(), 1);
        assert_eq!(s.jekt_held_record_failure("new", "boom").unwrap(), 1);
        assert_eq!(s.jekt_held_record_failure("new", "boom").unwrap(), 2);
        assert_eq!(s.jekt_held_for_target("uid-y", 10).unwrap()[0].last_error, "boom");
        s.jekt_held_delete("new").unwrap();
        assert!(s.jekt_held_targets().unwrap().is_empty());
    }
}

#[cfg(test)]
mod idle_tests {
    use super::tests::held;
    use super::*;
    use crate::backend::cold_hold::REASON_IDLE;

    fn idle(id: &str, target: &str, sent_at_ms: i64) -> HeldJekt {
        HeldJekt { hold_reason: REASON_IDLE.into(), ..held(id, target, sent_at_ms) }
    }

    #[test]
    fn a_cold_hold_waits_for_a_warm_session_but_the_others_do_not() {
        let s = Store::open_in_memory().unwrap();
        s.jekt_held_insert(&idle("i1", "uid-y", 100)).unwrap();
        s.jekt_held_insert(&held("a1", "uid-y", 200)).unwrap();
        let ids = |warm: bool, now: i64| {
            s.jekt_held_ready("uid-y", 10, warm, now).unwrap().into_iter().map(|r| r.request_id).collect::<Vec<_>>()
        };
        assert_eq!(ids(true, 1000), ["i1", "a1"], "warm: everything, oldest first");
        assert_eq!(ids(false, 1000), ["a1"], "cold: only what was not held for the cold session");
        assert_eq!(ids(false, HELD_TTL_MS + 100), ["i1", "a1"], "waited out its 24 h: delivered anyway");
        let round_trip = s.jekt_held_for_target("uid-y", 10).unwrap();
        assert!(round_trip[0].is_idle() && !round_trip[1].is_idle());
    }

    #[test]
    fn a_cold_hold_is_delivered_at_its_expiry_not_dropped_until_a_ttl_later() {
        let s = Store::open_in_memory().unwrap();
        s.jekt_held_insert(&idle("i1", "uid-y", 0)).unwrap();
        s.jekt_held_insert(&held("a1", "uid-y", 0)).unwrap();
        assert_eq!(s.jekt_held_delete_expired(HELD_TTL_MS).unwrap(), 1, "only the other kind");
        assert_eq!(s.jekt_held_for_target("uid-y", 10).unwrap()[0].request_id, "i1");
        assert_eq!(s.jekt_held_delete_expired(2 * HELD_TTL_MS).unwrap(), 1, "dropped a whole TTL later");
        assert!(s.jekt_held_targets().unwrap().is_empty());
    }

    #[test]
    fn identical_cold_copies_from_one_sender_are_removed_by_a_resend() {
        let s = Store::open_in_memory().unwrap();
        s.jekt_held_insert(&idle("i1", "uid-y", 100)).unwrap();
        s.jekt_held_insert(&held("a1", "uid-y", 200)).unwrap();
        assert_eq!(s.jekt_held_delete_idle_copies("uid-y", "sender", "hello").unwrap(), 1);
        assert_eq!(s.jekt_held_delete_idle_copies("uid-y", "sender", "hello").unwrap(), 0);
        assert_eq!(s.jekt_held_for_target("uid-y", 10).unwrap()[0].request_id, "a1", "a not-running hold is left alone");
    }
}
