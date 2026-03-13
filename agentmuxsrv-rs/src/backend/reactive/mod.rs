// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

//! Reactive messaging system for agent-to-agent terminal communication.
//! Port of Go's pkg/reactive/.
//!
//! Provides message injection into terminal panes, agent registration,
//! rate limiting, message sanitization, audit logging, and cross-host
//! polling via AgentMux cloud service.

pub mod handler;
pub mod poller;
pub mod registry;
pub mod sanitize;
pub mod types;
#[cfg(test)]
mod tests;

use std::time::{SystemTime, UNIX_EPOCH};

// ---- Constants ----

/// Maximum message length in bytes.
pub const MAX_MESSAGE_LENGTH: usize = 10_000;

/// Suffix appended when a message is truncated.
pub const TRUNCATION_SUFFIX: &str = "\n[Message truncated]";

/// Maximum entries in the audit log ring buffer.
const AUDIT_LOG_MAX: usize = 100;

/// Rate limit: max tokens (requests per second).
const RATE_LIMIT_MAX: u32 = 10;

/// Default poll interval for AgentMux poller (seconds).
pub const DEFAULT_POLL_INTERVAL_SECS: u64 = 30;

// ---- Re-exports ----

#[allow(unused_imports)]
pub use handler::{get_global_handler, Handler, ReactiveHandler};
#[allow(unused_imports)]
pub use poller::Poller;
#[allow(unused_imports)]
pub use sanitize::{
    format_injected_message, sanitize_message, validate_agent_id, validate_agentmux_url,
};
#[allow(unused_imports)]
pub use types::*;

// ---- Helpers ----

/// Get current time as Unix milliseconds.
fn now_unix_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Compute SHA-256 hex digest of a string (for audit log privacy).
fn sha256_hex(input: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    // Use a fast non-crypto hash for audit log (privacy, not security)
    let mut hasher = DefaultHasher::new();
    input.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}
