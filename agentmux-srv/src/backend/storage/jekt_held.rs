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
    pub sent_at_ms: i64,
    pub expires_at_ms: i64,
    pub attempts: i64,
    pub last_error: String,
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
        sent_at_ms: row.get("sent_at_ms")?,
        expires_at_ms: row.get("expires_at_ms")?,
        attempts: row.get("attempts")?,
        last_error: row.get("last_error")?,
    })
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
        let n = conn.execute(
            "INSERT INTO db_jekt_held
                (request_id, target_uid, target_agent, source_agent, audit_source_uid,
                 message, priority, jekt_tier, delivery_tier,
                 sig_verified, reagent_verified, lan_verified, channel_verified,
                 is_transcript_request, transcript_request_escalate_forced,
                 sent_at_ms, expires_at_ms, attempts, last_error)
             SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?18, ?19, ?14, ?15, 0, ''
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
            ],
        )?;
        Ok(if n > 0 { HoldOutcome::Held } else { HoldOutcome::Full })
    }

    /// Delete held messages past their expiry; returns how many.
    pub fn jekt_held_delete_expired(&self, now_ms: i64) -> Result<usize, StoreError> {
        let conn = self.conn().lock().unwrap();
        Ok(conn.execute(
            "DELETE FROM db_jekt_held WHERE expires_at_ms <= ?1",
            params![now_ms],
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
        let conn = self.conn().lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT * FROM db_jekt_held WHERE target_uid = ?1
              ORDER BY sent_at_ms ASC, request_id ASC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![target_uid, limit as i64], row_to_held)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
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
            sent_at_ms,
            expires_at_ms: sent_at_ms + HELD_TTL_MS,
            attempts: 0,
            last_error: String::new(),
        }
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
