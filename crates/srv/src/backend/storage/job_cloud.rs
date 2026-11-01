// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! Cloud job runs in the run ledger (docs/specs/SPEC_JOBS_AND_TEMPLATES_2026_10_24.md §4.1).
//!
//! A cloud job reports a run with up to three events: `started`, `heartbeat`, `finished`. Each is one write here,
//! in one transaction, so two instances reading the same event off MuxBus (one ledger, many channels) cannot both
//! write it. A real cloud run's row has no `scheduled_for_ms` (its slot is in `links`): that column is the unique
//! slot lease, and the watchdog's `missed` row for a slot holds it. An event for a slot the watchdog already gave up
//! on takes that row over, so the run reads "late" instead of appearing twice.

use rusqlite::{params, OptionalExtension};

use super::job_runs::{reason, row_to_run, state, JobRun, COLS};
use super::store::Store;
use super::StoreError;

/// Event ids seen, so an event pulled twice (by two channels, or sent twice) is written once. An event older than
/// [`KEEP_EVENT_IDS_MS`] is refused on its age, so its id is not needed any more.
pub(crate) const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS db_job_events (
        job_id    TEXT NOT NULL,
        run_id    TEXT NOT NULL,
        event_id  TEXT NOT NULL,
        seen_ms   INTEGER NOT NULL,
        PRIMARY KEY (job_id, run_id, event_id)
    );
    CREATE INDEX IF NOT EXISTS idx_ids_job_events_seen ON db_job_events(seen_ms);";

/// Longer than the 7 days an event may be old (§4.2).
const KEEP_EVENT_IDS_MS: i64 = 8 * 24 * 3600 * 1000;

/// The event being written: its id is remembered in the same transaction as its effect, so an event that failed to
/// write is not taken for one already seen when it is sent again.
#[derive(Debug, Clone, Copy)]
pub struct EventMark<'a> {
    pub run_id: &'a str,
    pub event_id: &'a str,
    pub now_ms: i64,
}

/// What a `started` event did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloudStart {
    /// A new `running` row.
    Opened,
    /// The watchdog's `missed` row for its slot became this run, `running`.
    TookOverMissed,
    /// The run is already in the ledger (a `finished` event came first, or a repeat): nothing changed.
    Known,
}

/// What a `finished` event did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloudFinish {
    /// A `running` row closed.
    Closed,
    /// No `started` event had arrived: a closed row of its own.
    Created,
    /// The row was `missed` or `lost`: it closed and reads "late".
    Late,
    /// The run had finished already: nothing changed.
    Known,
}

/// The ledger id of a cloud run: the same for all of its events.
pub fn cloud_run_id(job_id: &str, run_id: &str) -> String {
    format!("cloud:{job_id}:{run_id}")
}

fn get(conn: &rusqlite::Connection, id: &str) -> Result<Option<JobRun>, StoreError> {
    Ok(conn.query_row(&format!("SELECT {COLS} FROM db_job_runs WHERE id = ?1"), params![id], row_to_run).optional()?)
}

/// The watchdog's `missed`/`lost` row for `job_id`'s `slot`, if there is one.
fn given_up_slot(conn: &rusqlite::Connection, job_id: &str, slot: Option<i64>) -> Result<Option<JobRun>, StoreError> {
    let Some(slot) = slot else {
        return Ok(None);
    };
    Ok(conn
        .query_row(
            &format!("SELECT {COLS} FROM db_job_runs WHERE job_id = ?1 AND scheduled_for_ms = ?2 AND state IN ('missed','lost')"),
            params![job_id, slot],
            row_to_run,
        )
        .optional()?)
}

fn merge_links(old: &serde_json::Value, new: &serde_json::Value) -> serde_json::Value {
    let mut merged = if old.is_object() { old.clone() } else { serde_json::json!({}) };
    if let (Some(m), Some(n)) = (merged.as_object_mut(), new.as_object()) {
        for (k, v) in n {
            m.insert(k.clone(), v.clone());
        }
    }
    merged
}

/// Remember the event in `tx`. `true` for the first time, `false` for a repeat. Ids past their keeping time are
/// dropped on the way.
fn record_event(tx: &rusqlite::Connection, mark: &EventMark, job_id: &str) -> Result<bool, StoreError> {
    tx.execute("DELETE FROM db_job_events WHERE seen_ms < ?1", params![mark.now_ms - KEEP_EVENT_IDS_MS])?;
    let n = tx.execute(
        "INSERT OR IGNORE INTO db_job_events (job_id, run_id, event_id, seen_ms) VALUES (?1, ?2, ?3, ?4)",
        params![job_id, mark.run_id, mark.event_id, mark.now_ms],
    )?;
    Ok(n > 0)
}

/// Whether the ledger has a run of cloud job `job_id` for the slot at `slot_ms`: one that names the slot, or one that
/// started in `[from_ms, until_ms)`, which is how a run that never named its slot is matched to it.
fn slot_covered(conn: &rusqlite::Connection, job_id: &str, slot_ms: i64, from_ms: i64, until_ms: i64) -> Result<bool, StoreError> {
    Ok(conn
        .query_row(
            "SELECT 1 FROM db_job_runs
              WHERE job_id = ?1 AND job_source = 'cloud'
                AND (scheduled_for_ms = ?2
                     OR (json_valid(links) AND json_extract(links, '$.scheduled_for_ms') = ?2)
                     OR (started_at_ms >= ?3 AND started_at_ms < ?4))
              LIMIT 1",
            params![job_id, slot_ms, from_ms, until_ms],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

impl Store {
    /// [`slot_covered`] for the watchdog's reading pass.
    pub fn job_cloud_slot_covered(&self, job_id: &str, slot_ms: i64, from_ms: i64, until_ms: i64) -> Result<bool, StoreError> {
        slot_covered(&self.conn().lock().unwrap(), job_id, slot_ms, from_ms, until_ms)
    }

    /// The watchdog gives up on a slot: `missed` is its row, written only if the slot still has no run, which is
    /// looked at again here in the same transaction the events are written in, so a report that arrived since the
    /// watchdog read the ledger is not made into a miss as well. `false` when the slot has a run or a miss already.
    pub fn job_cloud_give_up_slot(&self, missed: &JobRun, from_ms: i64, until_ms: i64) -> Result<bool, StoreError> {
        let mut conn = self.conn().lock().unwrap();
        let tx = conn.transaction()?;
        let Some(slot) = missed.scheduled_for_ms else {
            return Ok(false);
        };
        if slot_covered(&tx, &missed.job_id, slot, from_ms, until_ms)? {
            return Ok(false);
        }
        insert(&tx, missed)?;
        tx.commit()?;
        Ok(true)
    }

    /// Every cloud run that has started and not finished.
    pub fn job_cloud_running(&self) -> Result<Vec<JobRun>, StoreError> {
        let conn = self.conn().lock().unwrap();
        let mut stmt = conn.prepare(&format!("SELECT {COLS} FROM db_job_runs WHERE job_source = 'cloud' AND state = 'running'"))?;
        let rows = stmt.query_map([], row_to_run)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// The watchdog gives up on a running cloud run `id`: it becomes `lost` if it is still running and has not been
    /// heard from (its start, or its latest heartbeat) within `max_run_ms` of `now_ms`. Checked in the update itself,
    /// so a heartbeat or a finish that arrived since the watchdog read the run keeps it alive. `false` then.
    pub fn job_cloud_lose(&self, id: &str, now_ms: i64, max_run_ms: i64) -> Result<bool, StoreError> {
        let conn = self.conn().lock().unwrap();
        let n = conn.execute(
            "UPDATE db_job_runs SET state = 'lost', reason = ?2, ended_at_ms = ?3
              WHERE id = ?1 AND state = 'running'
                AND MAX(started_at_ms, CASE WHEN json_valid(links) THEN COALESCE(json_extract(links, '$.heartbeat_ms'), 0) ELSE 0 END) + ?4 < ?3",
            params![id, reason::NO_REPORT, now_ms, max_run_ms],
        )?;
        Ok(n > 0)
    }

    /// A cloud run's `started` event: `run` is its `running` row, whose `links.scheduled_for_ms` names the slot.
    /// `None` for an event seen before.
    pub fn job_cloud_start(&self, run: &JobRun, slot: Option<i64>, mark: &EventMark) -> Result<Option<CloudStart>, StoreError> {
        let mut conn = self.conn().lock().unwrap();
        let tx = conn.transaction()?;
        if !record_event(&tx, mark, &run.job_id)? {
            return Ok(None);
        }
        if get(&tx, &run.id)?.is_some() {
            tx.commit()?;
            return Ok(Some(CloudStart::Known));
        }
        let result = if let Some(given_up) = given_up_slot(&tx, &run.job_id, slot)? {
            tx.execute(
                "UPDATE db_job_runs SET id = ?2, state = 'running', reason = ?3, infra = '', ended_at_ms = NULL, started_at_ms = ?4,
                        trigger_kind = ?5, attempt = ?6, links = ?7, reporter = ?8, recorded_by = ?9
                  WHERE id = ?1",
                params![
                    given_up.id,
                    run.id,
                    reason::LATE,
                    run.started_at_ms,
                    run.trigger,
                    run.attempt,
                    merge_links(&given_up.links, &run.links).to_string(),
                    run.reporter,
                    run.recorded_by
                ],
            )?;
            CloudStart::TookOverMissed
        } else {
            insert(&tx, run)?;
            CloudStart::Opened
        };
        tx.commit()?;
        Ok(Some(result))
    }

    /// A `heartbeat`: the run is alive at `at_ms`, which pushes its lost deadline back (§4.4). `Some(false)` for a
    /// run that isn't `running`, `None` for an event seen before.
    pub fn job_cloud_heartbeat(&self, job_id: &str, id: &str, at_ms: i64, mark: &EventMark) -> Result<Option<bool>, StoreError> {
        let mut conn = self.conn().lock().unwrap();
        let tx = conn.transaction()?;
        if !record_event(&tx, mark, job_id)? {
            return Ok(None);
        }
        let n = tx.execute(
            "UPDATE db_job_runs SET links = json_set(CASE WHEN json_valid(links) THEN links ELSE '{}' END, '$.heartbeat_ms', MAX(?2, COALESCE(json_extract(links, '$.heartbeat_ms'), 0)))
              WHERE id = ?1 AND state = 'running'",
            params![id, at_ms],
        )?;
        tx.commit()?;
        Ok(Some(n > 0))
    }

    /// A cloud run's `finished` event: `run` is its closed row (`finished`, with infra, outcome, summary, cost,
    /// `links`). `None` for an event seen before.
    pub fn job_cloud_finish(&self, run: &JobRun, slot: Option<i64>, mark: &EventMark) -> Result<Option<CloudFinish>, StoreError> {
        let mut conn = self.conn().lock().unwrap();
        let tx = conn.transaction()?;
        if !record_event(&tx, mark, &run.job_id)? {
            return Ok(None);
        }
        let existing = match get(&tx, &run.id)? {
            Some(row) => Some(row),
            None => given_up_slot(&tx, &run.job_id, slot)?,
        };
        let result = match existing {
            None => {
                insert(&tx, run)?;
                CloudFinish::Created
            }
            Some(row) if row.state == state::FINISHED => {
                tx.commit()?;
                return Ok(Some(CloudFinish::Known));
            }
            Some(row) => {
                let late = matches!(row.state.as_str(), state::MISSED | state::LOST);
                // A running row keeps the start it was given; a given-up row takes the run's own.
                let started = if late { run.started_at_ms } else { row.started_at_ms };
                tx.execute(
                    "UPDATE db_job_runs SET id = ?2, state = 'finished', reason = ?3, infra = ?4, outcome = ?5, summary = ?6,
                            started_at_ms = ?7, ended_at_ms = ?8, cost_usd = ?9, cost_known = ?10, output_tokens = ?11,
                            links = ?12, reporter = ?13, recorded_by = ?14, trigger_kind = ?15, attempt = ?16
                      WHERE id = ?1",
                    params![
                        row.id,
                        run.id,
                        if late || row.reason == reason::LATE { reason::LATE } else { "" },
                        run.infra,
                        run.outcome,
                        run.summary,
                        started,
                        run.ended_at_ms,
                        run.cost_usd,
                        run.cost_known as i64,
                        run.output_tokens,
                        merge_links(&row.links, &run.links).to_string(),
                        run.reporter,
                        run.recorded_by,
                        run.trigger,
                        run.attempt
                    ],
                )?;
                if late {
                    // The incident keeper judged this slot as a miss: have it judge the run again.
                    if let Some(ended) = row.ended_at_ms {
                        tx.execute(
                            "UPDATE db_job_incident_cursors SET ended_at_ms = ?2, run_id = ''
                              WHERE job_id = ?1 AND (ended_at_ms > ?2 OR (ended_at_ms = ?2 AND run_id >= ?3))",
                            params![row.job_id, ended, row.id],
                        )?;
                    }
                    CloudFinish::Late
                } else {
                    CloudFinish::Closed
                }
            }
        };
        tx.commit()?;
        Ok(Some(result))
    }
}

fn insert(conn: &rusqlite::Connection, run: &JobRun) -> Result<(), StoreError> {
    conn.execute(
        &format!(
            "INSERT OR IGNORE INTO db_job_runs ({COLS}) VALUES \
             (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23,?24,?25,?26,?27,?28,?29,?30,?31,?32)"
        ),
        params![
            run.id,
            run.job_id,
            run.job_source,
            run.job_name,
            run.template_id,
            run.template_version,
            run.trigger,
            run.scheduled_for_ms,
            run.triggered_by,
            run.attempt,
            run.state,
            run.reason,
            run.infra,
            run.outcome,
            run.summary,
            run.target_agent_uid,
            run.block_id,
            run.turn_id,
            run.account_id,
            run.provider,
            run.started_at_ms,
            run.ended_at_ms,
            run.cost_usd,
            run.cost_known as i64,
            run.cost_partial as i64,
            run.output_tokens,
            run.steps,
            run.links.to_string(),
            run.skip_count,
            run.last_skip_at_ms,
            run.reporter,
            run.recorded_by,
        ],
    )?;
    Ok(())
}
