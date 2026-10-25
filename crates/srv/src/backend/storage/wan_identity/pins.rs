// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Which install may speak for an agent name over WAN.
//!
//! A verified WAN message proves that some install of the account certified
//! the agent key that signed it. Nothing in that says which install is meant
//! to be `lazo`: any install of the account can publish a key under any
//! agent name. The first install seen speaking for a `(name, channel)` is
//! pinned here, the way the first key seen for a LAN name is pinned, and a
//! later message for the same name from another install fails verification
//! until a person moves the pin.
//!
//! The pin is set only for an install the account's presence list shows, and
//! is moved only by [`WanIdentityStore::agent_instance_pin_replace`], which
//! the operator's own window calls. A mismatch is remembered on the pin's row
//! so that window can show the pinned install beside the one that was refused.

use rusqlite::{params, OptionalExtension};

use super::{StoreError, WanIdentityStore};

/// Who is pinned to an agent name on one channel, and the last other install
/// that spoke for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentInstancePin {
    pub agent_id: String,
    pub channel: String,
    pub instance_id: String,
    pub host_hint: String,
    pub pinned_at: i64,
    /// The last other install whose message for this name was refused.
    pub conflict: Option<PinConflict>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinConflict {
    pub instance_id: String,
    pub host_hint: String,
    pub last_seen_at: i64,
    pub count: i64,
}

const COLUMNS: &str = "agent_id, channel, instance_id, host_hint, pinned_at, \
                       conflict_instance_id, conflict_host_hint, conflict_at, conflict_count";

fn row_to_pin(r: &rusqlite::Row<'_>) -> rusqlite::Result<AgentInstancePin> {
    let conflict_instance: Option<String> = r.get(5)?;
    Ok(AgentInstancePin {
        agent_id: r.get(0)?,
        channel: r.get(1)?,
        instance_id: r.get(2)?,
        host_hint: r.get(3)?,
        pinned_at: r.get(4)?,
        conflict: match conflict_instance {
            Some(instance_id) => Some(PinConflict {
                instance_id,
                host_hint: r.get::<_, Option<String>>(6)?.unwrap_or_default(),
                last_seen_at: r.get::<_, Option<i64>>(7)?.unwrap_or_default(),
                count: r.get::<_, Option<i64>>(8)?.unwrap_or_default(),
            }),
            None => None,
        },
    })
}

impl WanIdentityStore {
    /// The install pinned to `agent_id` on `channel`, if any.
    pub fn agent_instance_pin_get(&self, agent_id: &str, channel: &str) -> Result<Option<AgentInstancePin>, StoreError> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        Ok(conn
            .query_row(
                &format!("SELECT {COLUMNS} FROM wan_agent_instance_pins WHERE agent_id = ?1 AND channel = ?2"),
                params![agent_id.to_lowercase(), channel],
                row_to_pin,
            )
            .optional()?)
    }

    /// Pin `instance_id` to `(agent_id, channel)` unless a pin already exists,
    /// and return the pin that stands: the caller compares its instance with
    /// its own, so two first messages racing cannot both win.
    pub fn agent_instance_pin_get_or_set(
        &self,
        agent_id: &str,
        channel: &str,
        instance_id: &str,
        host_hint: &str,
        now: i64,
    ) -> Result<AgentInstancePin, StoreError> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "INSERT OR IGNORE INTO wan_agent_instance_pins (agent_id, channel, instance_id, host_hint, pinned_at) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![agent_id.to_lowercase(), channel, instance_id, host_hint, now],
        )?;
        Ok(conn.query_row(
            &format!("SELECT {COLUMNS} FROM wan_agent_instance_pins WHERE agent_id = ?1 AND channel = ?2"),
            params![agent_id.to_lowercase(), channel],
            row_to_pin,
        )?)
    }

    /// Remember that `instance_id` spoke for a name pinned to another install.
    pub fn agent_instance_pin_note_conflict(
        &self,
        agent_id: &str,
        channel: &str,
        instance_id: &str,
        host_hint: &str,
        now: i64,
    ) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "UPDATE wan_agent_instance_pins SET \
               conflict_count = CASE WHEN conflict_instance_id = ?3 THEN COALESCE(conflict_count, 0) + 1 ELSE 1 END, \
               conflict_instance_id = ?3, conflict_host_hint = ?4, conflict_at = ?5 \
             WHERE agent_id = ?1 AND channel = ?2",
            params![agent_id.to_lowercase(), channel, instance_id, host_hint, now],
        )?;
        Ok(())
    }

    /// Move the pin to `instance_id`, clearing the remembered conflict. The
    /// operator's window is the only caller. Returns the pin it replaced.
    pub fn agent_instance_pin_replace(
        &self,
        agent_id: &str,
        channel: &str,
        instance_id: &str,
        host_hint: &str,
        now: i64,
    ) -> Result<Option<AgentInstancePin>, StoreError> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let previous = conn
            .query_row(
                &format!("SELECT {COLUMNS} FROM wan_agent_instance_pins WHERE agent_id = ?1 AND channel = ?2"),
                params![agent_id.to_lowercase(), channel],
                row_to_pin,
            )
            .optional()?;
        conn.execute(
            "INSERT OR REPLACE INTO wan_agent_instance_pins (agent_id, channel, instance_id, host_hint, pinned_at) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![agent_id.to_lowercase(), channel, instance_id, host_hint, now],
        )?;
        Ok(previous)
    }

    /// Every pin, for the operator's window.
    pub fn agent_instance_pins_list(&self) -> Result<Vec<AgentInstancePin>, StoreError> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare(&format!("SELECT {COLUMNS} FROM wan_agent_instance_pins ORDER BY agent_id, channel"))?;
        let rows = stmt.query_map([], row_to_pin)?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, WanIdentityStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = WanIdentityStore::open(&dir.path().join("wan.db")).unwrap();
        (dir, store)
    }

    #[test]
    fn the_first_install_is_pinned_and_a_later_one_does_not_replace_it() {
        let (_dir, s) = store();
        assert!(s.agent_instance_pin_get("Lazo", "stable").unwrap().is_none());
        let first = s.agent_instance_pin_get_or_set("Lazo", "stable", "inst-a", "narko", 10).unwrap();
        assert_eq!((first.instance_id.as_str(), first.agent_id.as_str()), ("inst-a", "lazo"));
        let again = s.agent_instance_pin_get_or_set("lazo", "stable", "inst-b", "area54", 20).unwrap();
        assert_eq!(again.instance_id, "inst-a", "the pin holds the first install");
        assert_eq!(again.pinned_at, 10);
    }

    #[test]
    fn a_pin_is_per_channel() {
        let (_dir, s) = store();
        s.agent_instance_pin_get_or_set("lazo", "stable", "inst-a", "narko", 10).unwrap();
        let dev = s.agent_instance_pin_get_or_set("lazo", "dev-x", "inst-b", "area54", 11).unwrap();
        assert_eq!(dev.instance_id, "inst-b");
        assert_eq!(s.agent_instance_pins_list().unwrap().len(), 2);
    }

    #[test]
    fn a_conflict_is_remembered_and_counted_per_install() {
        let (_dir, s) = store();
        s.agent_instance_pin_get_or_set("lazo", "stable", "inst-a", "narko", 10).unwrap();
        s.agent_instance_pin_note_conflict("lazo", "stable", "inst-b", "area54", 20).unwrap();
        s.agent_instance_pin_note_conflict("lazo", "stable", "inst-b", "area54", 30).unwrap();
        let c = s.agent_instance_pin_get("lazo", "stable").unwrap().unwrap().conflict.unwrap();
        assert_eq!((c.instance_id.as_str(), c.host_hint.as_str(), c.last_seen_at, c.count), ("inst-b", "area54", 30, 2));
        s.agent_instance_pin_note_conflict("lazo", "stable", "inst-c", "box", 40).unwrap();
        let c = s.agent_instance_pin_get("lazo", "stable").unwrap().unwrap().conflict.unwrap();
        assert_eq!((c.instance_id.as_str(), c.count), ("inst-c", 1), "a different install starts its own count");
    }

    #[test]
    fn replacing_a_pin_returns_the_old_one_and_clears_the_conflict() {
        let (_dir, s) = store();
        s.agent_instance_pin_get_or_set("lazo", "stable", "inst-a", "narko", 10).unwrap();
        s.agent_instance_pin_note_conflict("lazo", "stable", "inst-b", "area54", 20).unwrap();
        let old = s.agent_instance_pin_replace("lazo", "stable", "inst-b", "area54", 50).unwrap().unwrap();
        assert_eq!(old.instance_id, "inst-a");
        let now = s.agent_instance_pin_get("lazo", "stable").unwrap().unwrap();
        assert_eq!((now.instance_id.as_str(), now.pinned_at, now.conflict), ("inst-b", 50, None));
        assert!(s.agent_instance_pin_replace("kim", "stable", "inst-z", "box", 60).unwrap().is_none(), "replace also pins a new name");
    }

    #[test]
    fn signing_out_or_switching_account_forgets_every_pin() {
        let (_dir, s) = store();
        s.agent_instance_pin_get_or_set("lazo", "stable", "inst-a", "narko", 10).unwrap();
        s.peer_cache_clear().unwrap();
        assert!(s.agent_instance_pins_list().unwrap().is_empty());
    }
}
