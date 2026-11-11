// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What each agent has used through the model gateway, per route and UTC day (`db_agent_spend_day`), so an
//! srv restart does not start the day again (`docs/specs/SPEC_ROUTE_AGENT_BRAKES_2026_11_10.md` section 3.1).
//! `backend::model_gateway::spend` keeps the live table and writes each changed row here.
//!
//! The table is created on first use (`CREATE TABLE IF NOT EXISTS`), as `container_limits` does: no schema bump,
//! and an older build ignores a table it does not know.

use rusqlite::params;

use super::error::StoreError;
use super::store::Store;

const CREATE_TABLE: &str = "CREATE TABLE IF NOT EXISTS db_agent_spend_day (
    agent_id       TEXT NOT NULL,
    route          TEXT NOT NULL,
    day            INTEGER NOT NULL,
    requests       INTEGER NOT NULL DEFAULT 0,
    input          INTEGER NOT NULL DEFAULT 0,
    output         INTEGER NOT NULL DEFAULT 0,
    cache_creation INTEGER NOT NULL DEFAULT 0,
    cache_read     INTEGER NOT NULL DEFAULT 0,
    cost_usd       REAL NOT NULL DEFAULT 0,
    alerted        INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (agent_id, route, day)
)";

/// How many days of rows are kept.
pub const KEEP_DAYS: i64 = 14;

/// One agent's use on one route for one UTC day (`day` is days since the epoch).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SpendDayRow {
    pub agent_id: String,
    pub route: String,
    pub day: i64,
    pub requests: u64,
    pub input: u64,
    pub output: u64,
    pub cache_creation: u64,
    pub cache_read: u64,
    pub cost_usd: f64,
    /// The highest alert level sent for this agent's day so far (0, 50, 80 or 100).
    pub alerted: u8,
}

impl Store {
    /// Writes `row`, replacing the stored one for its agent, route and day.
    pub fn spend_day_save(&self, row: &SpendDayRow) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(CREATE_TABLE)?;
        conn.execute(
            "INSERT OR REPLACE INTO db_agent_spend_day
             (agent_id, route, day, requests, input, output, cache_creation, cache_read, cost_usd, alerted)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                row.agent_id, row.route, row.day, row.requests as i64, row.input as i64, row.output as i64,
                row.cache_creation as i64, row.cache_read as i64, row.cost_usd, row.alerted
            ],
        )?;
        Ok(())
    }

    /// Every stored row from `since_day` on; the older ones are dropped, the table keeping [`KEEP_DAYS`] days.
    pub fn spend_day_load(&self, since_day: i64) -> Result<Vec<SpendDayRow>, StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(CREATE_TABLE)?;
        conn.execute("DELETE FROM db_agent_spend_day WHERE day < ?1", params![since_day - KEEP_DAYS])?;
        let mut stmt = conn.prepare(
            "SELECT agent_id, route, day, requests, input, output, cache_creation, cache_read, cost_usd, alerted
             FROM db_agent_spend_day WHERE day >= ?1",
        )?;
        let rows = stmt.query_map(params![since_day], |r| {
            Ok(SpendDayRow {
                agent_id: r.get(0)?,
                route: r.get(1)?,
                day: r.get(2)?,
                requests: r.get::<_, i64>(3)? as u64,
                input: r.get::<_, i64>(4)? as u64,
                output: r.get::<_, i64>(5)? as u64,
                cache_creation: r.get::<_, i64>(6)? as u64,
                cache_read: r.get::<_, i64>(7)? as u64,
                cost_usd: r.get(8)?,
                alerted: r.get::<_, i64>(9)? as u8,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Deletes `agent_id`'s rows for `day` (the "Reset today" button). The number of rows removed.
    pub fn spend_day_clear(&self, agent_id: &str, day: i64) -> Result<usize, StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(CREATE_TABLE)?;
        Ok(conn.execute("DELETE FROM db_agent_spend_day WHERE agent_id = ?1 AND day = ?2", params![agent_id, day])?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(agent: &str, route: &str, day: i64, cost: f64) -> SpendDayRow {
        SpendDayRow { agent_id: agent.into(), route: route.into(), day, requests: 3, output: 40, cost_usd: cost, alerted: 50, ..Default::default() }
    }

    #[test]
    fn a_saved_row_comes_back_and_a_second_save_replaces_it() {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        assert!(store.spend_day_load(0).unwrap().is_empty(), "no table yet: nothing, not an error");
        store.spend_day_save(&row("glim", "openrouter", 20_000, 4.5)).unwrap();
        store.spend_day_save(&row("glim", "openrouter", 20_000, 9.25)).unwrap();
        store.spend_day_save(&row("glim", "other", 20_000, 1.0)).unwrap();
        let mut rows = store.spend_day_load(20_000).unwrap();
        rows.sort_by(|a, b| b.route.cmp(&a.route));
        assert_eq!(rows.len(), 2);
        assert_eq!((rows[1].route.as_str(), rows[1].cost_usd, rows[1].alerted, rows[1].requests), ("openrouter", 9.25, 50, 3));
    }

    #[test]
    fn loading_drops_days_past_the_keep_window_and_clear_removes_one_agents_day() {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        store.spend_day_save(&row("glim", "openrouter", 20_000 - KEEP_DAYS - 1, 1.0)).unwrap();
        store.spend_day_save(&row("glim", "openrouter", 20_000, 2.0)).unwrap();
        store.spend_day_save(&row("aria", "openrouter", 20_000, 3.0)).unwrap();
        assert_eq!(store.spend_day_load(0).unwrap().len(), 3);
        assert_eq!(store.spend_day_load(20_000).unwrap().len(), 2, "the old row was dropped");
        assert_eq!(store.spend_day_clear("glim", 20_000).unwrap(), 1);
        let left = store.spend_day_load(0).unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].agent_id, "aria");
    }
}
