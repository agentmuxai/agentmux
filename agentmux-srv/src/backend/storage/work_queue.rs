// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! Muxqueue — the universal agent work queue.
//!
//! One row = one unit of work that ANY agent may claim, as opposed to jekt,
//! which is addressed push delivery to a named recipient right now. See
//! `docs/reports/REPORT_UNIVERSAL_AGENT_WORK_QUEUE_2026_09_01.md` for why this
//! is a new primitive rather than a feature of the Drone or Swarm panes.
//!
//! Shaped deliberately as `db_cron_jobs`' sibling: cron is this row with a
//! TIME trigger, this is the same row with a READINESS trigger. `not_before`
//! makes that relationship literal.
//!
//! **This lives in the always-global identity store, never the per-channel
//! one** (`db_work_queue` in `run_identity_store_schema`). A per-channel queue
//! would mean "any agent IN THIS CHANNEL can pick it up", which is close to
//! useless when every local/dev/portable build is its own channel.
//!
//! ## Claiming is a lease, not a lock
//!
//! A claim carries `claim_expires`. A claimant that dies without completing
//! loses the row back to the pool when [`Store::work_queue_reap`] runs. This is
//! deliberate, and copies `db_background_tasks`' `last_seen_ms` heartbeat
//! pattern rather than a bare `claimed` boolean — a bare flag reproduces the
//! stuck-`running`-forever incident class (agentmux issue #2518), where a row
//! whose owner vanished is indistinguishable from one being actively worked.
//!
//! `attempts`/`max_attempts` bound that recycling, so a poison item that
//! crashes every taker is eventually parked in `failed` instead of cycling
//! forever.

use rusqlite::{params, OptionalExtension, Row};
use serde::{Deserialize, Serialize};

use super::store::Store;
use super::StoreError;

/// Lifecycle state of a queue item. Stored as TEXT.
pub mod work_state {
    /// Claimable.
    pub const OPEN: &str = "open";
    /// Held under an unexpired lease by `claimed_by`.
    pub const CLAIMED: &str = "claimed";
    /// Finished successfully.
    pub const DONE: &str = "done";
    /// Gave up — either an explicit failure or `max_attempts` exhausted.
    pub const FAILED: &str = "failed";
    /// Withdrawn by a human/agent before completion.
    pub const CANCELLED: &str = "cancelled";
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkItem {
    pub id: String,
    pub title: String,
    /// The instruction injected into whichever agent claims this.
    pub payload: String,
    /// Free-form tag for filtering (`"review"`, `"repro"`, …). Empty = untyped.
    #[serde(default)]
    pub kind: String,
    /// Empty = any agent may claim. Non-empty = only this agent id.
    #[serde(default)]
    pub target_agent: String,
    /// Empty = no group restriction. Non-empty = a `db_agent_groups` id; any
    /// member may claim. Group membership is resolved by the CALLER (which
    /// holds the per-channel store where groups live), not here — this module
    /// stays a pure queue and does not reach across stores.
    #[serde(default)]
    pub target_group: String,
    /// Higher claims first. Ties broken by `created_at` ascending (FIFO).
    #[serde(default)]
    pub priority: i64,
    pub state: String,
    #[serde(default)]
    pub claimed_by: String,
    /// ms epoch; `None` unless `state == claimed`.
    #[serde(default)]
    pub claim_expires: Option<i64>,
    #[serde(default)]
    pub attempts: i64,
    #[serde(default)]
    pub max_attempts: i64,
    #[serde(default)]
    pub created_by: String,
    pub created_at: i64,
    pub updated_at: i64,
    /// ms epoch; not claimable before this. `None` = claimable immediately.
    #[serde(default)]
    pub not_before: Option<i64>,
    /// Completion note or failure reason.
    #[serde(default)]
    pub result: String,
}

/// What a caller may filter a claim by. All fields are AND-ed; `None` means
/// "don't constrain on this".
#[derive(Debug, Clone, Default)]
pub struct ClaimFilter {
    /// Only items with this `kind`.
    pub kind: Option<String>,
    /// The claiming agent's own id. Items targeted at a DIFFERENT agent are
    /// excluded; untargeted items stay eligible.
    pub agent_id: String,
    /// Group ids this agent belongs to. An item with a `target_group` is
    /// eligible only if its group is in this list. Resolved by the caller.
    pub groups: Vec<String>,
}

fn row_to_item(row: &Row) -> rusqlite::Result<WorkItem> {
    Ok(WorkItem {
        id: row.get("id")?,
        title: row.get("title")?,
        payload: row.get("payload")?,
        kind: row.get("kind")?,
        target_agent: row.get("target_agent")?,
        target_group: row.get("target_group")?,
        priority: row.get("priority")?,
        state: row.get("state")?,
        claimed_by: row.get("claimed_by")?,
        claim_expires: row.get("claim_expires")?,
        attempts: row.get("attempts")?,
        max_attempts: row.get("max_attempts")?,
        created_by: row.get("created_by")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
        not_before: row.get("not_before")?,
        result: row.get("result")?,
    })
}

const COLS: &str = "id, title, payload, kind, target_agent, target_group, priority, state, \
                    claimed_by, claim_expires, attempts, max_attempts, created_by, \
                    created_at, updated_at, not_before, result";

impl Store {
    /// Insert a new `open` item. `id` is caller-supplied so an enqueue can be
    /// made idempotent by the caller (re-enqueueing the same id is a conflict,
    /// not a duplicate).
    pub fn work_queue_enqueue(&self, item: &WorkItem) -> Result<(), StoreError> {
        let conn = self.conn().lock().unwrap();
        conn.execute(
            "INSERT INTO db_work_queue
                (id, title, payload, kind, target_agent, target_group, priority, state,
                 claimed_by, claim_expires, attempts, max_attempts, created_by,
                 created_at, updated_at, not_before, result)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17)",
            params![
                item.id, item.title, item.payload, item.kind, item.target_agent,
                item.target_group, item.priority, work_state::OPEN, "",
                None::<i64>, 0i64, if item.max_attempts > 0 { item.max_attempts } else { 3 },
                item.created_by, item.created_at, item.updated_at, item.not_before, "",
            ],
        )?;
        Ok(())
    }

    /// Atomically claim the highest-priority eligible item, or `None` when
    /// nothing is claimable.
    ///
    /// The entire claim is ONE conditional `UPDATE ... RETURNING`. That is what
    /// makes concurrent claims safe: SQLite serializes writers, so exactly one
    /// caller can observe a given row in `open` and transition it. Do NOT
    /// refactor this into a SELECT-then-UPDATE — that reintroduces the
    /// read-modify-write race this shape exists to avoid, and the race is
    /// invisible in single-threaded tests.
    ///
    /// `now_ms` and `lease_ms` are injected rather than read from the clock
    /// here so tests can drive expiry deterministically.
    pub fn work_queue_claim(
        &self,
        filter: &ClaimFilter,
        now_ms: i64,
        lease_ms: i64,
    ) -> Result<Option<WorkItem>, StoreError> {
        let conn = self.conn().lock().unwrap();

        // Group eligibility is an IN-list built from caller-resolved groups.
        // Built as a bound-parameter list (never string-interpolated) so a
        // group id can't inject SQL.
        let group_placeholders: String = if filter.groups.is_empty() {
            String::new()
        } else {
            let marks: Vec<String> = (0..filter.groups.len())
                .map(|i| format!("?{}", i + 5))
                .collect();
            format!(" OR target_group IN ({})", marks.join(","))
        };

        let sql = format!(
            "UPDATE db_work_queue
                SET state = '{claimed}',
                    claimed_by = ?1,
                    claim_expires = ?2,
                    attempts = attempts + 1,
                    updated_at = ?3
              WHERE id = (
                    SELECT id FROM db_work_queue
                     WHERE state = '{open}'
                       -- Defence in depth (Codex P2 on PR #2898): an item that
                       -- has burned its attempts must never be handed out
                       -- again, however it came to be `open`. release/reap
                       -- both park exhausted rows as `failed` themselves; this
                       -- makes a missed path fail closed rather than becoming
                       -- an infinite claim/release loop.
                       AND attempts < max_attempts
                       AND (not_before IS NULL OR not_before <= ?3)
                       AND (target_agent = '' OR target_agent = ?1)
                       AND (target_group = ''{groups})
                       AND (?4 = '' OR kind = ?4)
                     ORDER BY priority DESC, created_at ASC
                     LIMIT 1
              )
              RETURNING {cols}",
            claimed = work_state::CLAIMED,
            open = work_state::OPEN,
            groups = group_placeholders,
            cols = COLS,
        );

        let mut stmt = conn.prepare(&sql)?;
        // Positional binds: ?1 agent_id, ?2 lease expiry, ?3 now, ?4 kind,
        // ?5.. group ids. ?1 and ?3 are each referenced twice in the SQL above
        // (SET + WHERE); numbered parameters make that reuse safe.
        let expires = now_ms + lease_ms;
        let kind_filter = filter.kind.clone().unwrap_or_default();
        let mut binds: Vec<&dyn rusqlite::ToSql> =
            vec![&filter.agent_id, &expires, &now_ms, &kind_filter];
        for g in &filter.groups {
            binds.push(g);
        }

        let item = stmt
            .query_row(binds.as_slice(), row_to_item)
            .optional()?;
        Ok(item)
    }

    /// Extend a live lease.
    ///
    /// `attempt` is the **fence token** — the `attempts` value returned by the
    /// claim this call belongs to. Holder identity alone is NOT sufficient
    /// (Codex P1 on PR #2898): `agent_id` is stable across claims, so after
    /// `expire → reap → the SAME agent reclaims`, a delayed call left over
    /// from the FIRST claim still satisfies a holder-and-state predicate and
    /// would silently operate on the SECOND claim. That is a textbook ABA.
    /// `attempts` increments on every claim, so comparing it fences each claim
    /// instance apart with no extra column.
    pub fn work_queue_heartbeat(
        &self,
        id: &str,
        agent_id: &str,
        attempt: i64,
        now_ms: i64,
        lease_ms: i64,
    ) -> Result<bool, StoreError> {
        let conn = self.conn().lock().unwrap();
        let n = conn.execute(
            &format!(
                "UPDATE db_work_queue
                    SET claim_expires = ?4, updated_at = ?5
                  WHERE id = ?1 AND claimed_by = ?2 AND attempts = ?3
                    AND state = '{c}'",
                c = work_state::CLAIMED
            ),
            params![id, agent_id, attempt, now_ms + lease_ms, now_ms],
        )?;
        Ok(n > 0)
    }

    /// Mark a claimed item finished. Same holder + fence guard as heartbeat —
    /// a stale completion from a previous claim must not close out a newer
    /// one with an old result.
    pub fn work_queue_complete(
        &self,
        id: &str,
        agent_id: &str,
        attempt: i64,
        result: &str,
        now_ms: i64,
    ) -> Result<bool, StoreError> {
        let conn = self.conn().lock().unwrap();
        let n = conn.execute(
            &format!(
                "UPDATE db_work_queue
                    SET state = '{done}', result = ?4, claim_expires = NULL, updated_at = ?5
                  WHERE id = ?1 AND claimed_by = ?2 AND attempts = ?3
                    AND state = '{c}'",
                done = work_state::DONE,
                c = work_state::CLAIMED
            ),
            params![id, agent_id, attempt, result, now_ms],
        )?;
        Ok(n > 0)
    }

    /// Give a claim back voluntarily.
    ///
    /// `attempts` is NOT decremented — a release still consumed an attempt.
    /// And once those attempts are exhausted the item goes to `failed`, not
    /// back to `open` (Codex P2 on PR #2898): reopening unconditionally let a
    /// hot-potato item cycle claim→release→claim forever, because the reaper
    /// only parks rows that are still `claimed` and a released row isn't. That
    /// silently defeated the documented attempt bound for exactly the case the
    /// bound exists for.
    ///
    /// Fenced by `attempt` for the same ABA reason as `work_queue_heartbeat`.
    pub fn work_queue_release(
        &self,
        id: &str,
        agent_id: &str,
        attempt: i64,
        reason: &str,
        now_ms: i64,
    ) -> Result<bool, StoreError> {
        let conn = self.conn().lock().unwrap();
        let n = conn.execute(
            &format!(
                "UPDATE db_work_queue
                    SET state = CASE WHEN attempts >= max_attempts
                                     THEN '{failed}' ELSE '{open}' END,
                        claimed_by = '', claim_expires = NULL,
                        result = ?4, updated_at = ?5
                  WHERE id = ?1 AND claimed_by = ?2 AND attempts = ?3
                    AND state = '{c}'",
                failed = work_state::FAILED,
                open = work_state::OPEN,
                c = work_state::CLAIMED
            ),
            params![id, agent_id, attempt, reason, now_ms],
        )?;
        Ok(n > 0)
    }

    /// Reclaim every item whose lease expired: back to `open`, or to `failed`
    /// once `attempts >= max_attempts`.
    ///
    /// Returns `(reopened, failed)`. Safe to call on any interval; it is
    /// idempotent because an already-reaped row is no longer `claimed`.
    pub fn work_queue_reap(&self, now_ms: i64) -> Result<(usize, usize), StoreError> {
        let conn = self.conn().lock().unwrap();
        // Park the poison items FIRST — otherwise the reopen below would put a
        // row that has already exhausted its attempts back into the pool for
        // one more doomed round-trip.
        let failed = conn.execute(
            &format!(
                "UPDATE db_work_queue
                    SET state = '{failed}', claimed_by = '', claim_expires = NULL,
                        result = CASE WHEN result = '' THEN 'lease expired; max_attempts exhausted' ELSE result END,
                        updated_at = ?1
                  WHERE state = '{c}' AND claim_expires IS NOT NULL
                    AND claim_expires <= ?1 AND attempts >= max_attempts",
                failed = work_state::FAILED,
                c = work_state::CLAIMED
            ),
            params![now_ms],
        )?;
        let reopened = conn.execute(
            &format!(
                "UPDATE db_work_queue
                    SET state = '{open}', claimed_by = '', claim_expires = NULL, updated_at = ?1
                  WHERE state = '{c}' AND claim_expires IS NOT NULL
                    AND claim_expires <= ?1",
                open = work_state::OPEN,
                c = work_state::CLAIMED
            ),
            params![now_ms],
        )?;
        Ok((reopened, failed))
    }

    /// Explicitly abandon an item regardless of holder (operator action).
    pub fn work_queue_cancel(&self, id: &str, reason: &str, now_ms: i64) -> Result<bool, StoreError> {
        let conn = self.conn().lock().unwrap();
        let n = conn.execute(
            &format!(
                "UPDATE db_work_queue
                    SET state = '{cancelled}', claimed_by = '', claim_expires = NULL,
                        result = ?2, updated_at = ?3
                  WHERE id = ?1 AND state IN ('{open}','{c}')",
                cancelled = work_state::CANCELLED,
                open = work_state::OPEN,
                c = work_state::CLAIMED
            ),
            params![id, reason, now_ms],
        )?;
        Ok(n > 0)
    }

    pub fn work_queue_get(&self, id: &str) -> Result<Option<WorkItem>, StoreError> {
        let conn = self.conn().lock().unwrap();
        let item = conn
            .query_row(
                &format!("SELECT {COLS} FROM db_work_queue WHERE id = ?1"),
                params![id],
                row_to_item,
            )
            .optional()?;
        Ok(item)
    }

    /// List items, newest-updated first. `state` empty = all states.
    pub fn work_queue_list(&self, state: &str, limit: usize) -> Result<Vec<WorkItem>, StoreError> {
        let conn = self.conn().lock().unwrap();
        let mut stmt = conn.prepare(&format!(
            "SELECT {COLS} FROM db_work_queue
              WHERE (?1 = '' OR state = ?1)
              ORDER BY updated_at DESC, created_at DESC
              LIMIT ?2"
        ))?;
        let rows = stmt.query_map(params![state, limit as i64], row_to_item)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (Store, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let s = Store::open_identity_store(&dir.path().join("identity-store.db")).unwrap();
        (s, dir)
    }

    fn item(id: &str, title: &str) -> WorkItem {
        WorkItem {
            id: id.into(),
            title: title.into(),
            payload: "do the thing".into(),
            kind: String::new(),
            target_agent: String::new(),
            target_group: String::new(),
            priority: 0,
            state: work_state::OPEN.into(),
            claimed_by: String::new(),
            claim_expires: None,
            attempts: 0,
            max_attempts: 3,
            created_by: "tester".into(),
            created_at: 1000,
            updated_at: 1000,
            not_before: None,
            result: String::new(),
        }
    }

    fn any(agent: &str) -> ClaimFilter {
        ClaimFilter { kind: None, agent_id: agent.into(), groups: vec![] }
    }

    #[test]
    fn claim_returns_none_on_an_empty_queue() {
        let (s, _d) = store();
        assert!(s.work_queue_claim(&any("a1"), 2000, 60_000).unwrap().is_none());
    }

    /// The core guarantee: two agents racing for one item must not both get
    /// it. Single-threaded here, but it drives the same conditional-UPDATE
    /// path concurrency relies on — a SELECT-then-UPDATE refactor fails this.
    #[test]
    fn one_item_can_only_be_claimed_once() {
        let (s, _d) = store();
        s.work_queue_enqueue(&item("w1", "only one")).unwrap();

        let first = s.work_queue_claim(&any("a1"), 2000, 60_000).unwrap();
        let second = s.work_queue_claim(&any("a2"), 2000, 60_000).unwrap();

        assert_eq!(first.map(|i| i.id), Some("w1".to_string()));
        assert!(second.is_none(), "a second claimant must not get the same item");
    }

    #[test]
    fn claim_prefers_higher_priority_then_older_first() {
        let (s, _d) = store();
        let mut low = item("low", "low");
        low.priority = 0;
        low.created_at = 10;
        let mut high = item("high", "high");
        high.priority = 5;
        high.created_at = 99;
        let mut old = item("old", "old");
        old.priority = 5;
        old.created_at = 50;
        s.work_queue_enqueue(&low).unwrap();
        s.work_queue_enqueue(&high).unwrap();
        s.work_queue_enqueue(&old).unwrap();

        // priority 5 beats 0; within priority 5, created_at 50 beats 99.
        assert_eq!(s.work_queue_claim(&any("a"), 2000, 60_000).unwrap().unwrap().id, "old");
        assert_eq!(s.work_queue_claim(&any("a"), 2000, 60_000).unwrap().unwrap().id, "high");
        assert_eq!(s.work_queue_claim(&any("a"), 2000, 60_000).unwrap().unwrap().id, "low");
    }

    #[test]
    fn not_before_defers_an_item_until_its_time() {
        let (s, _d) = store();
        let mut later = item("later", "later");
        later.not_before = Some(5_000);
        s.work_queue_enqueue(&later).unwrap();

        assert!(
            s.work_queue_claim(&any("a"), 4_999, 60_000).unwrap().is_none(),
            "must not be claimable before not_before"
        );
        assert!(
            s.work_queue_claim(&any("a"), 5_000, 60_000).unwrap().is_some(),
            "claimable exactly at not_before"
        );
    }

    #[test]
    fn a_targeted_item_is_only_claimable_by_its_target() {
        let (s, _d) = store();
        let mut t = item("t", "for a2");
        t.target_agent = "a2".into();
        s.work_queue_enqueue(&t).unwrap();

        assert!(s.work_queue_claim(&any("a1"), 2000, 60_000).unwrap().is_none());
        assert_eq!(s.work_queue_claim(&any("a2"), 2000, 60_000).unwrap().unwrap().id, "t");
    }

    #[test]
    fn a_group_targeted_item_is_claimable_only_by_a_member() {
        let (s, _d) = store();
        let mut g = item("g", "for reviewers");
        g.target_group = "reviewers".into();
        s.work_queue_enqueue(&g).unwrap();

        let outsider =
            ClaimFilter { kind: None, agent_id: "a1".into(), groups: vec!["writers".into()] };
        let member =
            ClaimFilter { kind: None, agent_id: "a2".into(), groups: vec!["reviewers".into()] };

        assert!(s.work_queue_claim(&outsider, 2000, 60_000).unwrap().is_none());
        assert_eq!(s.work_queue_claim(&member, 2000, 60_000).unwrap().unwrap().id, "g");
    }

    /// An agent with NO groups must still claim untargeted work — the
    /// empty-group path builds a different SQL string, so it needs its own
    /// coverage rather than being assumed equivalent to the group path.
    #[test]
    fn an_agent_with_no_groups_can_still_claim_untargeted_work() {
        let (s, _d) = store();
        s.work_queue_enqueue(&item("free", "anyone")).unwrap();
        assert_eq!(s.work_queue_claim(&any("a"), 2000, 60_000).unwrap().unwrap().id, "free");
    }

    #[test]
    fn kind_filter_narrows_the_claim() {
        let (s, _d) = store();
        let mut review = item("r", "review");
        review.kind = "review".into();
        let mut repro = item("p", "repro");
        repro.kind = "repro".into();
        s.work_queue_enqueue(&review).unwrap();
        s.work_queue_enqueue(&repro).unwrap();

        let f = ClaimFilter { kind: Some("repro".into()), agent_id: "a".into(), groups: vec![] };
        assert_eq!(s.work_queue_claim(&f, 2000, 60_000).unwrap().unwrap().id, "p");
    }

    #[test]
    fn heartbeat_and_complete_are_holder_only() {
        let (s, _d) = store();
        s.work_queue_enqueue(&item("w", "held")).unwrap();
        let c = s.work_queue_claim(&any("owner"), 2000, 60_000).unwrap().unwrap();

        assert!(!s.work_queue_heartbeat("w", "impostor", c.attempts, 3000, 60_000).unwrap());
        assert!(s.work_queue_heartbeat("w", "owner", c.attempts, 3000, 60_000).unwrap());

        assert!(!s.work_queue_complete("w", "impostor", c.attempts, "nope", 4000).unwrap());
        assert!(s.work_queue_complete("w", "owner", c.attempts, "shipped", 4000).unwrap());

        let done = s.work_queue_get("w").unwrap().unwrap();
        assert_eq!(done.state, work_state::DONE);
        assert_eq!(done.result, "shipped");
        assert!(done.claim_expires.is_none());
    }

    /// Codex P1 on PR #2898 — the ABA case. `agent_id` is stable across
    /// claims, so after expire → reap → *the same agent* reclaims, a delayed
    /// call left over from the FIRST claim satisfies any holder-and-state
    /// predicate and would operate on the SECOND claim. The `attempts` fence
    /// is what separates the two claim instances.
    #[test]
    fn a_stale_call_from_a_previous_claim_cannot_touch_the_new_one() {
        let (s, _d) = store();
        s.work_queue_enqueue(&item("w", "reclaimed by the same agent")).unwrap();

        let first = s.work_queue_claim(&any("a1"), 1_000, 1_000).unwrap().unwrap();
        assert_eq!(first.attempts, 1);
        assert_eq!(s.work_queue_reap(2_000).unwrap(), (1, 0));

        // SAME agent id picks it up again — the whole point of the case.
        let second = s.work_queue_claim(&any("a1"), 3_000, 60_000).unwrap().unwrap();
        assert_eq!(second.attempts, 2, "the reclaim is a distinct claim instance");

        // Everything below is the FIRST claim's fence arriving late.
        assert!(
            !s.work_queue_complete("w", "a1", first.attempts, "stale result", 4_000).unwrap(),
            "a stale completion must not close out the newer claim"
        );
        assert!(
            !s.work_queue_heartbeat("w", "a1", first.attempts, 4_000, 60_000).unwrap(),
            "a stale heartbeat must not extend the newer claim"
        );
        assert!(
            !s.work_queue_release("w", "a1", first.attempts, "stale release", 4_000).unwrap(),
            "a stale release must not reopen the newer claim"
        );

        let live = s.work_queue_get("w").unwrap().unwrap();
        assert_eq!(live.state, work_state::CLAIMED, "the second claim is untouched");
        assert_eq!(live.result, "");

        // And the CURRENT fence still works.
        assert!(s.work_queue_complete("w", "a1", second.attempts, "real result", 5_000).unwrap());
        assert_eq!(s.work_queue_get("w").unwrap().unwrap().result, "real result");
    }

    /// The issue-#2518 lesson: a claimant that dies must not hold the row
    /// forever.
    #[test]
    fn reap_returns_an_expired_lease_to_the_pool() {
        let (s, _d) = store();
        s.work_queue_enqueue(&item("w", "abandoned")).unwrap();
        s.work_queue_claim(&any("ghost"), 1_000, 5_000).unwrap().unwrap(); // expires 6000

        assert_eq!(s.work_queue_reap(5_999).unwrap(), (0, 0), "not yet expired");
        assert_eq!(s.work_queue_reap(6_000).unwrap(), (1, 0), "expired at the boundary");

        let back = s.work_queue_get("w").unwrap().unwrap();
        assert_eq!(back.state, work_state::OPEN);
        assert_eq!(back.claimed_by, "");
        assert_eq!(back.attempts, 1, "the dead attempt is still counted");
        assert!(s.work_queue_claim(&any("someone-else"), 7_000, 60_000).unwrap().is_some());
    }

    /// A poison item that crashes every taker must eventually stop cycling.
    /// This also pins the ordering INSIDE `work_queue_reap`: if the reopen
    /// pass ran before the fail pass, an already-exhausted row would be handed
    /// out for one more doomed round-trip.
    #[test]
    fn a_poison_item_is_parked_as_failed_once_attempts_are_exhausted() {
        let (s, _d) = store();
        let mut poison = item("p", "kills takers");
        poison.max_attempts = 2;
        s.work_queue_enqueue(&poison).unwrap();

        s.work_queue_claim(&any("a1"), 1_000, 1_000).unwrap().unwrap();
        assert_eq!(s.work_queue_reap(2_000).unwrap(), (1, 0));

        s.work_queue_claim(&any("a2"), 3_000, 1_000).unwrap().unwrap();
        let (reopened, failed) = s.work_queue_reap(4_000).unwrap();
        assert_eq!((reopened, failed), (0, 1), "second expiry exhausts max_attempts");

        let dead = s.work_queue_get("p").unwrap().unwrap();
        assert_eq!(dead.state, work_state::FAILED);
        assert!(
            s.work_queue_claim(&any("a3"), 5_000, 60_000).unwrap().is_none(),
            "a failed item must never be handed out again"
        );
    }

    #[test]
    fn release_returns_the_item_but_keeps_the_attempt() {
        let (s, _d) = store();
        s.work_queue_enqueue(&item("w", "hot potato")).unwrap();
        let c = s.work_queue_claim(&any("a1"), 1_000, 60_000).unwrap().unwrap();

        assert!(!s.work_queue_release("w", "impostor", c.attempts, "not mine", 2_000).unwrap());
        assert!(s.work_queue_release("w", "a1", c.attempts, "cannot do this", 2_000).unwrap());

        let back = s.work_queue_get("w").unwrap().unwrap();
        assert_eq!(back.state, work_state::OPEN);
        assert_eq!(back.attempts, 1, "a voluntary release still consumed an attempt");
        assert_eq!(s.work_queue_claim(&any("a2"), 3_000, 60_000).unwrap().unwrap().id, "w");
    }

    /// Codex P2 on PR #2898: releasing on the FINAL allowed attempt used to
    /// return the row to `open` unconditionally, so a hot-potato item could
    /// cycle claim→release→claim forever — the reaper cannot park it, because
    /// a released row is no longer `claimed`. That silently defeated the
    /// attempt bound for precisely the case it exists to bound.
    #[test]
    fn releasing_on_the_final_attempt_fails_the_item_instead_of_reopening_it() {
        let (s, _d) = store();
        let mut hot = item("w", "nobody can do this");
        hot.max_attempts = 2;
        s.work_queue_enqueue(&hot).unwrap();

        let c1 = s.work_queue_claim(&any("a1"), 1_000, 60_000).unwrap().unwrap();
        assert!(s.work_queue_release("w", "a1", c1.attempts, "not for me", 1_500).unwrap());
        assert_eq!(s.work_queue_get("w").unwrap().unwrap().state, work_state::OPEN,
                   "attempt 1 of 2 — still has a life left");

        let c2 = s.work_queue_claim(&any("a2"), 2_000, 60_000).unwrap().unwrap();
        assert_eq!(c2.attempts, 2, "final allowed attempt");
        assert!(s.work_queue_release("w", "a2", c2.attempts, "nor me", 2_500).unwrap());

        let dead = s.work_queue_get("w").unwrap().unwrap();
        assert_eq!(dead.state, work_state::FAILED, "exhausted release must park, not reopen");
        assert!(
            s.work_queue_claim(&any("a3"), 3_000, 60_000).unwrap().is_none(),
            "the cycle must be broken, not merely slowed"
        );
    }

    /// Defence in depth for the same finding: even if a row somehow reaches
    /// `open` with its attempts already spent, the claim query itself must
    /// refuse it rather than trusting release/reap to have parked it.
    #[test]
    fn an_open_row_with_exhausted_attempts_is_not_claimable() {
        let (s, _d) = store();
        let mut spent = item("w", "already spent");
        spent.max_attempts = 1;
        s.work_queue_enqueue(&spent).unwrap();

        // Force the state release/reap would normally never leave behind.
        s.work_queue_claim(&any("a1"), 1_000, 60_000).unwrap().unwrap();
        {
            let conn = s.conn().lock().unwrap();
            conn.execute(
                "UPDATE db_work_queue SET state = 'open', claimed_by = '' WHERE id = 'w'",
                [],
            )
            .unwrap();
        }

        assert!(
            s.work_queue_claim(&any("a2"), 2_000, 60_000).unwrap().is_none(),
            "attempts >= max_attempts must fail closed at the claim query"
        );
    }

    #[test]
    fn cancel_removes_an_item_from_circulation_from_either_state() {
        let (s, _d) = store();
        s.work_queue_enqueue(&item("o", "open one")).unwrap();
        s.work_queue_enqueue(&item("c", "claimed one")).unwrap();
        // Cancel "o" out of the way so the claim below deterministically takes "c".
        assert!(s.work_queue_cancel("o", "not needed", 1_500).unwrap());
        s.work_queue_claim(&any("a1"), 2_000, 60_000).unwrap().unwrap();

        assert!(s.work_queue_cancel("c", "superseded", 2_500).unwrap());
        assert_eq!(s.work_queue_get("c").unwrap().unwrap().state, work_state::CANCELLED);
        assert!(s.work_queue_claim(&any("a2"), 3_000, 60_000).unwrap().is_none());
        // A second cancel is a no-op, not an error.
        assert!(!s.work_queue_cancel("c", "again", 3_000).unwrap());
    }

    #[test]
    fn a_completed_item_is_never_reaped_or_reclaimed() {
        let (s, _d) = store();
        s.work_queue_enqueue(&item("w", "finished")).unwrap();
        let c = s.work_queue_claim(&any("a1"), 1_000, 1_000).unwrap().unwrap();
        assert!(s.work_queue_complete("w", "a1", c.attempts, "ok", 1_500).unwrap());

        assert_eq!(s.work_queue_reap(9_999).unwrap(), (0, 0), "a done row has no live lease");
        assert!(s.work_queue_claim(&any("a2"), 9_999, 60_000).unwrap().is_none());
    }

    #[test]
    fn list_filters_by_state_and_respects_the_limit() {
        let (s, _d) = store();
        s.work_queue_enqueue(&item("a", "a")).unwrap();
        s.work_queue_enqueue(&item("b", "b")).unwrap();
        s.work_queue_claim(&any("x"), 2_000, 60_000).unwrap().unwrap();

        assert_eq!(s.work_queue_list("", 10).unwrap().len(), 2);
        assert_eq!(s.work_queue_list(work_state::CLAIMED, 10).unwrap().len(), 1);
        assert_eq!(s.work_queue_list(work_state::OPEN, 10).unwrap().len(), 1);
        assert_eq!(s.work_queue_list("", 1).unwrap().len(), 1);
    }

    #[test]
    fn enqueue_rejects_a_duplicate_id_rather_than_silently_overwriting() {
        let (s, _d) = store();
        s.work_queue_enqueue(&item("dup", "first")).unwrap();
        assert!(s.work_queue_enqueue(&item("dup", "second")).is_err());
        assert_eq!(s.work_queue_get("dup").unwrap().unwrap().title, "first");
    }
}
