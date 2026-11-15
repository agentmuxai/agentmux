// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! A job on the `webhook_local` source starts a run when a script on this computer POSTs to
//! `http://127.0.0.1:<port>/agentmux/hook/<job id>` with the job's secret
//! (docs/specs/SPEC_DRONE_REAL_WORLD_JOBS_2026_11_11.md §3.2; the route is `server::hook_handlers`).
//!
//! The job's filter holds one thing: `secret_sha256`, the SHA-256 of the secret, as 64 lowercase hex characters
//! (`printf %s "$SECRET" | shasum -a 256`). The secret itself is never stored: whoever writes the hook's caller
//! (a git hook, a CI script) keeps it, and a job definition, which agents and the Jobs list can read, holds only the hash.
//! A secret should be random (`openssl rand -hex 32`); a guessable one makes its hash guessable.
//!
//! The run reads the request as `{{trigger.body}}` (the JSON object the script sent) and `{{trigger.received_ms}}`.
//! The body sits under its own key so a script's own field names can never be read as the event's.

use ring::constant_time::verify_slices_are_equal;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use super::definition::{event_source, trigger_kind};
use crate::backend::storage::cron::CronJob;

/// The header that carries the secret: never the URL, a query or the body, so a cross-site form cannot send it.
pub const SECRET_HEADER: &str = "x-agentmux-hook-secret";
/// An optional header naming the call, so a script that retries a call it did not hear back from starts one run.
pub const ID_HEADER: &str = "x-agentmux-hook-id";
/// The most a request body may be.
pub const MAX_BODY: usize = 64 * 1024;
/// The longest a call id may be.
pub const MAX_ID_CHARS: usize = 200;

const KEY: &str = "secret_sha256";

/// A `webhook_local` trigger's filter: the secret's hash, and nothing else.
pub fn validate_filter(filter: &Map<String, Value>) -> Result<(), String> {
    if let Some(key) = filter.keys().find(|k| k.as_str() != KEY) {
        return Err(format!("trigger: webhook_local has no filter key {key:?}; it has only {KEY}"));
    }
    let hash = filter.get(KEY).and_then(Value::as_str).unwrap_or("");
    if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
        return Err(format!("trigger: a webhook_local filter has {KEY}, the SHA-256 of the secret as 64 lowercase hex characters"));
    }
    Ok(())
}

/// The hash a job keeps for a secret.
pub fn hash_of(secret: &str) -> String {
    hex::encode(Sha256::digest(secret.as_bytes()))
}

/// Whether `job` is an enabled event job on `webhook_local`, and `secret` is its secret. The hashes are compared in
/// constant time. A job that is not one, or is off, never matches: the caller answers all of those the same way.
pub fn accepts(job: &CronJob, secret: &str) -> bool {
    if !job.enabled || job.trigger_kind != trigger_kind::EVENT {
        return false;
    }
    let t = &job.trigger_event;
    if t.get("source").and_then(Value::as_str) != Some(event_source::WEBHOOK_LOCAL) {
        return false;
    }
    let Some(want) = t.get("filter").and_then(|f| f.get(KEY)).and_then(Value::as_str) else { return false };
    verify_slices_are_equal(hash_of(secret).as_bytes(), want.as_bytes()).is_ok()
}
