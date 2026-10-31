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
    fn slot(&self, key: &Key) -> Arc<tokio::sync::Mutex<Slot>> {
        let mut slots = self.slots.lock().unwrap_or_else(|e| e.into_inner());
        slots.entry(key.clone()).or_insert_with(|| Arc::new(tokio::sync::Mutex::new(Slot { session: None }))).clone()
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
        let slot = self.slot(&Key { agent: agent.to_string(), grant: grant.to_string(), reason: reason.to_string() });
        let mut slot = slot.lock().await;
        if let Some(session) = slot.session.as_ref().filter(|s| s.expiration - now >= REUSE_MARGIN) {
            return Ok((session.clone(), Source::Reused));
        }
        slot.session = None;
        if !self.allow_attempt(agent, grant, cap, now) {
            return Err(SessionError::TooMany { cap });
        }
        let minted = mint().await.map_err(SessionError::Mint)?;
        slot.session = Some(minted.clone());
        Ok((minted, Source::Fresh))
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
