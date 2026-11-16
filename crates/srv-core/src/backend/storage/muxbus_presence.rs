// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Whether a channel has a MuxBus sign-in of its own stored, asked of the store alone.
//!
//! A channel's sign-in is a row in `db_muxbus_credentials` (`channel:<ch>`, or `global` in a
//! store that is already the channel's own) plus its tokens in the OS keychain under
//! `muxbus:channel:<ch>`. The row is written when the sign-in completes and deleted when it
//! ends. Both are keyed by the channel, not the version, so an in-app upgrade (the same channel,
//! a new version) finds its predecessor's sign-in. Reading the row touches no keychain, so
//! boot can ask this before deciding whether a reconnect would read one.

use super::muxbus::current_row_id;
use super::store::Store;

impl Store {
    /// Whether this process's channel has a MuxBus sign-in of its own stored. A SQL read only.
    pub fn muxbus_has_own_sign_in(&self) -> bool {
        self.muxbus_has_row(&current_row_id())
    }

    fn muxbus_has_row(&self, row_id: &str) -> bool {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.query_row("SELECT 1 FROM db_muxbus_credentials WHERE id = ?1", [row_id], |_| Ok(()))
            .is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::super::muxbus::row_id_for;
    use super::*;

    const CHANNEL: &str = "local-HEAD-7138a5-8dbe7897";

    fn sign_in(store: &Store, row_id: &str) {
        let conn = store.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO db_muxbus_credentials (id, user_email, user_sub) VALUES (?1, 'a@b.c', 'sub-1')",
            [row_id],
        )
        .unwrap();
    }

    #[test]
    fn a_channel_with_no_sign_in_of_its_own_has_none() {
        let store = Store::open_in_memory().unwrap();
        assert!(!store.muxbus_has_row(&row_id_for(Some(CHANNEL), false)));
    }

    /// The host-wide `stable` sign-in is not this channel's: a local channel on a machine where
    /// the installed release is signed in still has none of its own.
    #[test]
    fn the_stable_sign_in_is_not_the_channels() {
        let store = Store::open_in_memory().unwrap();
        sign_in(&store, &row_id_for(Some("stable"), false));
        assert!(!store.muxbus_has_row(&row_id_for(Some(CHANNEL), false)));
    }

    #[test]
    fn another_channels_sign_in_is_not_this_channels() {
        let store = Store::open_in_memory().unwrap();
        sign_in(&store, &row_id_for(Some("local-main-b28b7a-651ced27"), false));
        assert!(!store.muxbus_has_row(&row_id_for(Some(CHANNEL), false)));
    }

    /// Sign in on version N; the in-app upgrade restarts srv as N+1 on the same store file, in
    /// the same channel: the row is found, so boot reconnects (before: it was skipped and the
    /// window showed signed out until someone signed in again).
    #[test]
    fn an_upgrade_in_the_same_channel_finds_the_previous_versions_sign_in() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("store.db");
        let row = row_id_for(Some(CHANNEL), false);
        {
            let version_n = Store::open(&path).unwrap();
            assert!(!version_n.muxbus_has_row(&row));
            sign_in(&version_n, &row);
        }
        let version_n_plus_1 = Store::open(&path).unwrap();
        assert!(version_n_plus_1.muxbus_has_row(&row));
    }
}
