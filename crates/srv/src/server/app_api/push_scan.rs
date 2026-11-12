// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `POST /api/v1/github/push-scan`: a container agent's push, judged by the host's publish guard.
//!
//! A container's `pre-push` hook (`agentmux-bashwrap git-hook pre-push`) collects what the push publishes with the
//! container's own git (per ref: the commits' messages, the added paths and the added lines) and posts it here as
//! data. srv gives it, on stdin, to the host's `gh-agent scan-push --repo OWNER/REPO` and answers with its verdict.
//!
//! The material is data: nothing here opens the container's clone or runs git in it, and the request carries no path.
//! Whether the repository is public is decided by `gh-agent` asking GitHub about the `owner/repo`, never by a flag in
//! the request.
//!
//! Callable with a container agent's own token only. Every field is checked and capped before anything runs; the
//! checks are the pure functions below.

use std::collections::{HashMap, VecDeque};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use axum::extract::{Extension, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::AppState;
use crate::identity::gh_agent_token::{self, ScanOutcome};
use crate::server::caller::Caller;

/// The most a request may carry, in bytes. The route's body limit is this.
pub(crate) const MAX_BODY_BYTES: usize = 17 * 1024 * 1024;
/// The most the pushed material may weigh once read; a bigger push is refused with what to do about it.
const MAX_MATERIAL_BYTES: usize = 16 * 1024 * 1024;
const MAX_PARTS: usize = 64;
const MAX_FIELD_BYTES: usize = 8 * 1024 * 1024;
const MAX_CHANGESETS: usize = 100;
const MAX_CHANGESET_BYTES: usize = 64 * 1024;
const MAX_REF_BYTES: usize = 512;
/// Pushes one agent may ask to have judged in a window, and the window.
const RATE_LIMIT: usize = 12;
const RATE_WINDOW: Duration = Duration::from_secs(60);
/// Judgements running at once on this machine.
const MAX_RUNNING: usize = 2;

/// What the container sends: one push, the same for every ref it updates.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PushScanRequest {
    /// `owner/repo`.
    repo: String,
    /// The agent set `GH_AGENT_ALLOW_NEW_DOCS=1`; the guard records the use like any other.
    allow_new_docs: bool,
    /// Empty asks only whether this machine has a publish guard to run.
    parts: Vec<Part>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Part {
    r#ref: String,
    count: u64,
    remote_named: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tag_message: Option<String>,
    #[serde(default)]
    messages: String,
    #[serde(default)]
    name_status: String,
    #[serde(default)]
    patch: String,
    #[serde(default)]
    changesets: Vec<Changeset>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Changeset {
    path: String,
    text: String,
}

/// A GitHub account or repository name: nothing that could be read as an option or a path.
fn is_name(s: &str) -> bool {
    !s.is_empty() && s.len() <= 100 && !s.starts_with(['-', '.']) && s.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

/// `owner/repo` in lowercase, or why the request names no repository.
fn repo_slug(repo: &str) -> Result<String, String> {
    match repo.split_once('/') {
        Some((owner, name)) if is_name(owner) && is_name(name) => Ok(format!("{owner}/{name}").to_ascii_lowercase()),
        _ => Err("repo is not owner/repo".to_string()),
    }
}

/// A ref name as git writes one: printable, no control characters, a bounded length.
fn is_ref(s: &str) -> bool {
    !s.is_empty() && s.len() <= MAX_REF_BYTES && s.bytes().all(|b| (0x20..0x7f).contains(&b) || b >= 0x80)
}

/// Every field of the request checked and capped, or the first thing wrong with it.
pub(crate) fn validate(req: &PushScanRequest) -> Result<String, String> {
    let slug = repo_slug(&req.repo)?;
    if req.parts.len() > MAX_PARTS {
        return Err(format!("more than {MAX_PARTS} refs in one push"));
    }
    let mut total = 0usize;
    for (i, p) in req.parts.iter().enumerate() {
        if !is_ref(&p.r#ref) {
            return Err(format!("part {i}: the ref is not a ref name"));
        }
        if p.count > 10_000_000 {
            return Err(format!("part {i}: count is out of range"));
        }
        let fields = [p.tag_message.as_deref().unwrap_or(""), &p.messages, &p.name_status, &p.patch];
        for f in fields {
            if f.len() > MAX_FIELD_BYTES {
                return Err(format!("part {i}: a field is over {MAX_FIELD_BYTES} bytes"));
            }
            total += f.len();
        }
        if p.changesets.len() > MAX_CHANGESETS {
            return Err(format!("part {i}: more than {MAX_CHANGESETS} changesets"));
        }
        for c in &p.changesets {
            if c.path.len() > 1024 || c.text.len() > MAX_CHANGESET_BYTES {
                return Err(format!("part {i}: a changeset is too large"));
            }
            total += c.path.len() + c.text.len();
        }
    }
    if total > MAX_MATERIAL_BYTES {
        return Err(format!("the push is over {MAX_MATERIAL_BYTES} bytes to check: split it into smaller pushes"));
    }
    Ok(slug)
}

/// A sliding window per agent.
struct Limiter {
    seen: Mutex<HashMap<String, VecDeque<Instant>>>,
}

impl Limiter {
    fn new() -> Self {
        Self { seen: Mutex::new(HashMap::new()) }
    }

    /// Whether `who` may ask now (and counts it if so).
    fn allow(&self, who: &str, now: Instant) -> bool {
        let mut seen = self.seen.lock().unwrap_or_else(|e| e.into_inner());
        let q = seen.entry(who.to_string()).or_default();
        while q.front().is_some_and(|t| now.duration_since(*t) >= RATE_WINDOW) {
            q.pop_front();
        }
        if q.len() >= RATE_LIMIT {
            return false;
        }
        q.push_back(now);
        true
    }
}

fn limiter() -> &'static Limiter {
    static LIMITER: OnceLock<Limiter> = OnceLock::new();
    LIMITER.get_or_init(Limiter::new)
}

fn running() -> &'static tokio::sync::Semaphore {
    static RUNNING: OnceLock<tokio::sync::Semaphore> = OnceLock::new();
    RUNNING.get_or_init(|| tokio::sync::Semaphore::new(MAX_RUNNING))
}

/// `POST /api/v1/github/push-scan`
pub(crate) async fn handle_push_scan(State(state): State<AppState>, caller: Option<Extension<Caller>>, Json(req): Json<PushScanRequest>) -> Response {
    let refuse = |status: StatusCode, message: String| (status, Json(json!({ "error": message }))).into_response();
    let Some(uid) = caller.as_ref().and_then(|Extension(c)| c.uid()).map(str::to_string) else {
        return refuse(StatusCode::UNAUTHORIZED, "no agent token: a push is judged for an agent's own commands".to_string());
    };
    let mstore = state.mstore.clone();
    let (slug_of_agent, in_container) = match tokio::task::spawn_blocking(move || mstore.agent_def_get(&uid)).await {
        Ok(Ok(Some(def))) => {
            let in_container = def.runtime().is_container();
            (def.slug, in_container)
        }
        Ok(Ok(None)) => return refuse(StatusCode::UNAUTHORIZED, "the agent token names no agent on this machine".to_string()),
        _ => return refuse(StatusCode::INTERNAL_SERVER_ERROR, "couldn't read the agent".to_string()),
    };
    // A native agent's own pre-push hook runs the guard in its clone; this route is for what runs in a container.
    if !in_container {
        return Json(json!({ "guard": "none", "code": 0, "lines": [] })).into_response();
    }
    let repo = match validate(&req) {
        Ok(repo) => repo,
        Err(why) => return refuse(StatusCode::BAD_REQUEST, why),
    };
    if !limiter().allow(&slug_of_agent, Instant::now()) {
        return refuse(StatusCode::TOO_MANY_REQUESTS, format!("too many pushes to check in a minute (at most {RATE_LIMIT}): wait a minute and push again"));
    }
    // A machine with no gh-agent has no publish guard to run: the hook does nothing and says nothing.
    if gh_agent_token::installed().is_none() {
        return Json(json!({ "guard": "none", "code": 0, "lines": [] })).into_response();
    }
    let Ok(_slot) = running().try_acquire() else {
        return refuse(StatusCode::TOO_MANY_REQUESTS, "other pushes are being checked on this machine: try again in a moment".to_string());
    };
    let material = match serde_json::to_vec(&req) {
        Ok(m) => m,
        Err(e) => return refuse(StatusCode::INTERNAL_SERVER_ERROR, format!("couldn't prepare the push: {e}")),
    };
    match gh_agent_token::scan_push(&slug_of_agent, &repo, &material).await {
        ScanOutcome::Judged { code, lines, public } => Json(json!({ "guard": "gh-agent", "code": code, "lines": lines, "public": public })).into_response(),
        ScanOutcome::Failed(why) => {
            tracing::warn!(target: "identity", agent = %slug_of_agent, repo = %repo, error = %why, "identity.github: the publish guard couldn't judge a container agent's push");
            refuse(StatusCode::SERVICE_UNAVAILABLE, why)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn part() -> Part {
        Part { r#ref: "refs/heads/x".into(), count: 1, remote_named: true, tag_message: None, messages: String::new(), name_status: String::new(), patch: String::new(), changesets: vec![] }
    }

    fn request(repo: &str, parts: Vec<Part>) -> PushScanRequest {
        PushScanRequest { repo: repo.into(), allow_new_docs: false, parts }
    }

    #[test]
    fn a_repository_is_owner_slash_name_in_a_safe_charset_and_comes_back_lowercase() {
        assert_eq!(validate(&request("AgentMuxAI/Agentmux", vec![part()])).unwrap(), "agentmuxai/agentmux");
        for bad in ["", "agentmuxai", "a/b/c", "a b/c", "a/b;rm -rf", "../x/y", "-x/y", "a/.hidden", "a/b\n", "a/b c", &format!("{}/y", "o".repeat(101))] {
            assert!(validate(&request(bad, vec![])).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn a_ref_is_printable_and_bounded() {
        for bad in ["", "refs/heads/a\nb", "refs/heads/a\0b", &"r".repeat(MAX_REF_BYTES + 1)] {
            let mut p = part();
            p.r#ref = bad.to_string();
            assert!(validate(&request("a/b", vec![p])).is_err(), "{bad:?}");
        }
        let mut p = part();
        p.r#ref = "refs/heads/feature/é-ü".into();
        assert!(validate(&request("a/b", vec![p])).is_ok());
    }

    #[test]
    fn counts_sizes_and_numbers_of_things_are_capped() {
        let mut p = part();
        p.count = 10_000_001;
        assert!(validate(&request("a/b", vec![p])).is_err());
        assert!(validate(&request("a/b", (0..=MAX_PARTS).map(|_| part()).collect())).is_err());
        let mut p = part();
        p.patch = "x".repeat(MAX_FIELD_BYTES + 1);
        assert!(validate(&request("a/b", vec![p])).is_err());
        let mut p = part();
        p.changesets = (0..=MAX_CHANGESETS).map(|_| Changeset { path: "p".into(), text: "t".into() }).collect();
        assert!(validate(&request("a/b", vec![p])).is_err());
        let mut p = part();
        p.changesets = vec![Changeset { path: "p".into(), text: "t".repeat(MAX_CHANGESET_BYTES + 1) }];
        assert!(validate(&request("a/b", vec![p])).is_err());
    }

    #[test]
    fn a_push_over_the_total_is_refused_with_what_to_do() {
        let big = |n: usize| {
            let mut p = part();
            p.patch = "x".repeat(n);
            p
        };
        let parts = vec![big(MAX_FIELD_BYTES), big(MAX_FIELD_BYTES), big(10)];
        let why = validate(&request("a/b", parts)).unwrap_err();
        assert!(why.contains("split it into smaller pushes"), "{why}");
    }

    #[test]
    fn a_field_the_request_does_not_know_is_refused_rather_than_passed_on() {
        let with_extra = r#"{"repo":"a/b","allow_new_docs":false,"parts":[],"cwd":"/workspace/x"}"#;
        assert!(serde_json::from_str::<PushScanRequest>(with_extra).is_err());
        let part_extra = r#"{"repo":"a/b","allow_new_docs":false,"parts":[{"ref":"r","count":1,"remote_named":true,"is_public":false}]}"#;
        assert!(serde_json::from_str::<PushScanRequest>(part_extra).is_err());
        let ok = r#"{"repo":"a/b","allow_new_docs":true,"parts":[{"ref":"refs/heads/x","count":1,"remote_named":true}]}"#;
        assert!(serde_json::from_str::<PushScanRequest>(ok).is_ok());
    }

    #[test]
    fn what_is_forwarded_is_what_was_read_and_nothing_else() {
        let ok = r#"{"repo":"a/b","allow_new_docs":true,"parts":[{"ref":"refs/heads/x","count":2,"remote_named":false,"patch":"p","changesets":[{"path":".changesets/a.md","text":"t"}]}]}"#;
        let req: PushScanRequest = serde_json::from_str(ok).unwrap();
        let again: serde_json::Value = serde_json::from_slice(&serde_json::to_vec(&req).unwrap()).unwrap();
        assert_eq!(again["parts"][0]["changesets"][0]["path"], ".changesets/a.md");
        assert_eq!(again["parts"][0]["count"], 2);
        assert!(again["parts"][0].get("tag_message").is_none());
        assert_eq!(again.as_object().unwrap().len(), 3);
    }

    #[test]
    fn an_agent_may_ask_a_dozen_times_a_minute_and_each_agent_has_its_own_count() {
        let l = Limiter::new();
        let t0 = Instant::now();
        for _ in 0..RATE_LIMIT {
            assert!(l.allow("aria", t0));
        }
        assert!(!l.allow("aria", t0));
        assert!(l.allow("other", t0), "another agent is not held back");
        assert!(l.allow("aria", t0 + RATE_WINDOW), "the window moves on");
    }
}
