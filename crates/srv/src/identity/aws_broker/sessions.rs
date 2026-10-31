// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The sessions srv has minted for container agents, and how often it may mint (spec §8).
//!
//! The AWS CLI calls `credential_process` on every command, and botocore treats credentials with under ten minutes
//! left as expired, so a session is handed out again only while at least [`REUSE_MARGIN`] remains, and a request that
//! finds none mints. One mint at a time per (agent, grant, reason): concurrent requests wait for the same result.

use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Duration, Utc};

use super::mint::{Minted, MintError};

/// Credentials with less than this left are not handed out again.
pub const REUSE_MARGIN: Duration = Duration::minutes(10);
const WINDOW: Duration = Duration::hours(1);
/// Past this many slots, the ones with nothing live in them are dropped when a new one is made.
const PRUNE_ABOVE: usize = 32;

#[derive(Clone, PartialEq, Eq, Hash)]
struct Key {
    agent: String,
    grant: String,
    reason: String,
}

struct Slot {
    session: Option<Minted>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum SessionError {
    /// More mints than the grant allows in the last hour.
    TooMany { cap: u32 },
    Mint(MintError),
}

/// Whether the answer is a session that was already there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Fresh,
    /// Minted before, still more than the margin from expiring.
    Reused,
}

#[derive(Default)]
pub struct Sessions {
    slots: Mutex<HashMap<Key, Arc<tokio::sync::Mutex<Slot>>>>,
    attempts: Mutex<HashMap<(String, String), VecDeque<DateTime<Utc>>>>,
}

impl Sessions {
    /// The slot for `key`, made if need be. Slots that hold nothing usable (expired, or never filled) are dropped first,
    /// so the map holds at most the sessions that are live: an agent that varies its reason cannot grow it.
    fn slot(&self, key: &Key, now: DateTime<Utc>) -> Arc<tokio::sync::Mutex<Slot>> {
        let mut slots = self.slots.lock().unwrap_or_else(|e| e.into_inner());
        if slots.len() >= PRUNE_ABOVE {
            // A slot being minted into is locked: it stays.
            slots.retain(|_, s| s.try_lock().map_or(true, |g| g.session.as_ref().is_some_and(|m| m.expiration > now)));
        }
        slots.entry(key.clone()).or_insert_with(|| Arc::new(tokio::sync::Mutex::new(Slot { session: None }))).clone()
    }

    /// Drop the slot for `key` if it holds no session: a request that could not mint leaves nothing behind.
    fn forget_if_empty(&self, key: &Key) {
        let mut slots = self.slots.lock().unwrap_or_else(|e| e.into_inner());
        if slots.get(key).is_some_and(|s| s.try_lock().is_ok_and(|g| g.session.is_none())) {
            slots.remove(key);
        }
    }

    /// Count an attempt at minting for (agent, grant) unless the last hour already has `cap`.
    fn allow_attempt(&self, agent: &str, grant: &str, cap: u32, now: DateTime<Utc>) -> bool {
        let mut all = self.attempts.lock().unwrap_or_else(|e| e.into_inner());
        let times = all.entry((agent.to_string(), grant.to_string())).or_default();
        while times.front().is_some_and(|t| now - *t >= WINDOW) {
            times.pop_front();
        }
        if times.len() >= cap as usize {
            return false;
        }
        times.push_back(now);
        true
    }

    /// The session for (agent, grant, reason): the one minted before while it has more than the margin left, else the
    /// result of `mint`. Attempts count against `cap` per hour whether or not they succeed, so a failing loop is
    /// bounded too.
    pub async fn get_or_mint<F, Fut>(
        &self,
        agent: &str,
        grant: &str,
        reason: &str,
        cap: u32,
        now: DateTime<Utc>,
        mint: F,
    ) -> Result<(Minted, Source), SessionError>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<Minted, MintError>>,
    {
        let key = Key { agent: agent.to_string(), grant: grant.to_string(), reason: reason.to_string() };
        let slot = self.slot(&key, now);
        let outcome = {
            let mut held = slot.lock().await;
            if let Some(session) = held.session.as_ref().filter(|s| s.expiration - now >= REUSE_MARGIN) {
                return Ok((session.clone(), Source::Reused));
            }
            held.session = None;
            if !self.allow_attempt(agent, grant, cap, now) {
                Err(SessionError::TooMany { cap })
            } else {
                match mint().await {
                    Ok(minted) => {
                        held.session = Some(minted.clone());
                        Ok((minted, Source::Fresh))
                    }
                    Err(e) => Err(SessionError::Mint(e)),
                }
            }
        };
        if outcome.is_err() {
            drop(slot);
            self.forget_if_empty(&key);
        }
        outcome
    }

    /// Forget everything held for an agent (its container stopped).
    pub fn revoke_agent(&self, agent: &str) {
        self.slots.lock().unwrap_or_else(|e| e.into_inner()).retain(|k, _| k.agent != agent);
        // The attempts stay: a loop that restarts its container must not reset the hourly cap.
    }

    #[cfg(test)]
    pub fn held(&self, agent: &str) -> usize {
        self.slots.lock().unwrap().keys().filter(|k| k.agent == agent).count()
    }
}

/// srv's sessions: one cache for the process.
pub fn global() -> &'static Sessions {
    static SESSIONS: std::sync::OnceLock<Sessions> = std::sync::OnceLock::new();
    SESSIONS.get_or_init(Sessions::default)
}

#[cfg(test)]
#[path = "sessions_tests.rs"]
mod tests;
