// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! In-memory manager for pre-launch OAuth sessions.
//!
//! See `docs/specs/SPEC_PRE_LAUNCH_OAUTH_FLOW_2026_05_14.md` §7.
//!
//! Each session represents one user-initiated "Connect with OAuth"
//! attempt. The frontend creates a session via `start_session`,
//! polls via `poll_session`, optionally pastes a callback URL via
//! `submit_callback_url`, and cancels via `cancel_session`.
//!
//! PR A scope: the session-state machine + per-line stdout
//! interpretation + lifecycle (timeout, cancel, cleanup). The
//! actual CLI spawn lives in the handler (so it can use AppState's
//! CLI resolver) but emits frames into this module via
//! `record_line` / `record_exit`. That keeps this module pure and
//! testable.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use super::auth_patterns::{match_line, AuthPatternMatch};

/// Wall-clock cap on a single auth session. Past this, the session
/// transitions to `Failed { reason: "timeout" }` and the spawned
/// CLI (if any) is killed by the handler that owns the join handle.
const SESSION_TIMEOUT_SECS: u64 = 600;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(
    tag = "status",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum AuthSessionStatus {
    /// CLI is spawned, we're waiting for it to emit either a URL,
    /// a device code, or a success line.
    Pending,
    /// CLI emitted an OAuth URL. Frontend surfaces this to the user
    /// for paste-into-browser if auto-open failed.
    UrlAvailable { auth_url: String },
    /// CLI emitted a device code. Frontend renders it prominently.
    CodeEmitted {
        device_code: String,
        verification_url: String,
    },
    /// CLI authenticated successfully. The handler captured the
    /// credentials and created the bundle.
    Success {
        bundle_id: String,
        /// Best-effort — extracted from the CLI's "logged in as..."
        /// line. Used by the frontend to name the bundle.
        email: Option<String>,
        /// Set only for a direct-account session (issue #1624 PR-C
        /// Part B) — the newly-minted or reused `IdentityAccount` id,
        /// with no bundle involved. `bundle_id` is an empty string in
        /// that mode; existing bundle-mode callers never populate
        /// this field, so it's skipped on the wire when absent.
        #[serde(skip_serializing_if = "Option::is_none")]
        account_id: Option<String>,
    },
    /// Auth attempt failed. `error` is a short human-readable phrase
    /// suitable to render inline.
    Failed { error: String },
}

impl AuthSessionStatus {
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Success { .. } | Self::Failed { .. })
    }
}

#[derive(Debug)]
struct Session {
    provider_id: String,
    status: AuthSessionStatus,
    /// Last URL or code we surfaced — kept across polls so the
    /// frontend can repaint without re-receiving on every tick.
    captured_url: Option<String>,
    captured_device_code: Option<(String, String)>,
    captured_email: Option<String>,
    started_at: Instant,
    /// All stdout/stderr lines we've seen, in order. Used for the
    /// diagnostic "show me what the CLI said" panel and the
    /// integration tests.
    transcript: Vec<String>,
}

impl Session {
    fn new(provider_id: String) -> Self {
        Self {
            provider_id,
            status: AuthSessionStatus::Pending,
            captured_url: None,
            captured_device_code: None,
            captured_email: None,
            started_at: Instant::now(),
            transcript: Vec::new(),
        }
    }

    fn timed_out(&self) -> bool {
        self.started_at.elapsed() > Duration::from_secs(SESSION_TIMEOUT_SECS)
    }
}

/// Public-facing handle returned by `start_session`. The handler
/// owns the spawned CLI's child handle (so it can stdin-inject the
/// pasted callback URL and kill on cancel); this manager owns the
/// session state.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartSessionResult {
    pub session_id: String,
    /// If the CLI emitted the URL synchronously (before this call
    /// returns) — rare but happens for fast providers. Usually
    /// `None`; the frontend polls and picks it up on the first tick.
    pub auth_url: Option<String>,
}

/// Snapshot returned by `poll_session`. Mirrors `AuthSessionStatus`
/// plus the provider id for renderer dispatch.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PollSessionResult {
    pub provider_id: String,
    #[serde(flatten)]
    pub status: AuthSessionStatus,
}

/// Per-session "process refs" — the join handle for the drain task
/// and the stdin sender for the callback-paste-back path. Held
/// outside the `sessions` Mutex so cancellation can `.abort()` /
/// drop without holding the state lock across an await.
#[derive(Default)]
struct ProcessRefs {
    drain_tasks: HashMap<String, tokio::task::JoinHandle<()>>,
    stdin_senders: HashMap<String, tokio::sync::mpsc::Sender<String>>,
    /// PID of a PTY-backed auth subprocess. Aborting `drain_tasks`
    /// can't reach into a `spawn_blocking` wait task, so cancel_session
    /// kills the child by PID for PTY transports.
    pty_pids: HashMap<String, u32>,
}

#[derive(Default)]
pub struct AuthSessionManager {
    sessions: Arc<Mutex<HashMap<String, Session>>>,
    process_refs: Arc<Mutex<ProcessRefs>>,
}

impl AuthSessionManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Allocate a new session id and store the initial Pending state.
    /// The caller (handler) is responsible for spawning the CLI and
    /// feeding stdout lines into `record_line`.
    pub fn start_session(
        &self,
        provider_id: String,
        _into_bundle_id: Option<String>,
    ) -> StartSessionResult {
        let session_id = format!("auth-{}", uuid::Uuid::new_v4());
        let session = Session::new(provider_id);
        self.sessions
            .lock()
            .unwrap()
            .insert(session_id.clone(), session);
        StartSessionResult {
            session_id,
            auth_url: None,
        }
    }

    /// Feed a single line of CLI stdout/stderr into the session.
    /// Returns the pattern match (if any) so the handler can decide
    /// to forward the URL elsewhere (e.g. broker event).
    pub fn record_line(&self, session_id: &str, line: &str) -> Option<AuthPatternMatch> {
        let mut sessions = self.sessions.lock().unwrap();
        let session = sessions.get_mut(session_id)?;
        session.transcript.push(line.to_string());
        let m = match_line(&session.provider_id, line)?;
        match &m {
            AuthPatternMatch::OAuthUrl(url) => {
                if session.captured_url.is_none() {
                    session.captured_url = Some(url.clone());
                    if matches!(session.status, AuthSessionStatus::Pending) {
                        session.status = AuthSessionStatus::UrlAvailable {
                            auth_url: url.clone(),
                        };
                    }
                }
            }
            AuthPatternMatch::DeviceCode {
                code,
                verification_url,
            } => {
                if session.captured_device_code.is_none() {
                    session.captured_device_code =
                        Some((code.clone(), verification_url.clone()));
                    if matches!(
                        session.status,
                        AuthSessionStatus::Pending | AuthSessionStatus::UrlAvailable { .. }
                    ) {
                        session.status = AuthSessionStatus::CodeEmitted {
                            device_code: code.clone(),
                            verification_url: verification_url.clone(),
                        };
                    }
                }
            }
            AuthPatternMatch::LoginSuccess { email } => {
                // Don't transition to Success here — the handler is
                // responsible for confirming with authCheckCommand
                // AND persisting the bundle row before declaring
                // success. We just record the email for later.
                if session.captured_email.is_none() {
                    session.captured_email = email.clone();
                }
            }
            AuthPatternMatch::LoginFailure { message: _ } => {
                // Same — handler decides Failed status (it has more
                // context: CLI exit code, auth check result, etc.).
            }
        }
        Some(m)
    }

    /// Handler-side completion hook. Called when the CLI exits or
    /// auth check confirms success. Transitions the session to a
    /// terminal state. `account_id` is `None` for the legacy
    /// bundle-mode path; `Some(id)` for a direct-account session
    /// (issue #1624 PR-C Part B) — see `AuthSessionStatus::Success`.
    pub fn finish_success(
        &self,
        session_id: &str,
        bundle_id: String,
        account_id: Option<String>,
    ) -> bool {
        let mut sessions = self.sessions.lock().unwrap();
        let Some(session) = sessions.get_mut(session_id) else {
            return false;
        };
        if session.status.is_terminal() {
            return false;
        }
        session.status = AuthSessionStatus::Success {
            bundle_id,
            email: session.captured_email.clone(),
            account_id,
        };
        true
    }

    pub fn finish_failure(&self, session_id: &str, error: String) -> bool {
        let mut sessions = self.sessions.lock().unwrap();
        let Some(session) = sessions.get_mut(session_id) else {
            return false;
        };
        if session.status.is_terminal() {
            return false;
        }
        session.status = AuthSessionStatus::Failed { error };
        true
    }

    /// Poll a session's current status. Also sweeps timed-out
    /// sessions: if the session has been Pending past
    /// SESSION_TIMEOUT_SECS, transition to Failed and return that.
    pub fn poll_session(&self, session_id: &str) -> Option<PollSessionResult> {
        let mut sessions = self.sessions.lock().unwrap();
        let session = sessions.get_mut(session_id)?;
        if !session.status.is_terminal() && session.timed_out() {
            session.status = AuthSessionStatus::Failed {
                error: format!(
                    "auth session timed out after {SESSION_TIMEOUT_SECS}s"
                ),
            };
        }
        Some(PollSessionResult {
            provider_id: session.provider_id.clone(),
            status: session.status.clone(),
        })
    }

    /// Cancel a session: transition state to Failed + abort the
    /// spawned drain task (which kills the child via kill_on_drop).
    /// Returns true if the state transition happened (false on
    /// unknown / already-terminal session — the process refs are
    /// torn down either way so a re-fired cancel is a no-op).
    pub fn cancel_session(&self, session_id: &str) -> bool {
        let transitioned =
            self.finish_failure(session_id, "cancelled by user".to_string());
        let mut refs = self.process_refs.lock().unwrap();
        if let Some(handle) = refs.drain_tasks.remove(session_id) {
            handle.abort();
        }
        refs.stdin_senders.remove(session_id);
        // PTY transport: abort() doesn't reach the spawn_blocking
        // wait task, so kill the child by PID. Pipes path doesn't
        // need this — its tokio Child uses kill_on_drop.
        if let Some(pid) = refs.pty_pids.remove(session_id) {
            if let Err(e) = kill_pid(pid) {
                tracing::warn!(pid, session_id, error = %e, "cancel_session: kill_pid failed");
            } else {
                tracing::info!(pid, session_id, "cancel_session: PTY child killed");
            }
        }
        transitioned
    }

    /// Register the PID of a PTY-backed auth subprocess so
    /// `cancel_session` can terminate it. Called by `auth.start`
    /// after spawning a PTY login (in addition to `attach_process`).
    pub fn attach_pty_pid(&self, session_id: &str, pid: u32) {
        let mut refs = self.process_refs.lock().unwrap();
        refs.pty_pids.insert(session_id.to_string(), pid);
    }

    /// Register the drain task + stdin sender for a session. Called
    /// by the handler immediately after spawning the CLI.
    pub fn attach_process(
        &self,
        session_id: &str,
        drain_task: tokio::task::JoinHandle<()>,
        stdin_sender: tokio::sync::mpsc::Sender<String>,
    ) {
        let mut refs = self.process_refs.lock().unwrap();
        refs.drain_tasks.insert(session_id.to_string(), drain_task);
        refs.stdin_senders.insert(session_id.to_string(), stdin_sender);
    }

    /// Forward a pasted callback URL to the spawned CLI's stdin.
    /// Returns true if the session has an attached stdin sender;
    /// false if the session never spawned a CLI or the sender's
    /// receiver was dropped.
    pub async fn send_to_stdin(&self, session_id: &str, line: String) -> bool {
        let sender = {
            let refs = self.process_refs.lock().unwrap();
            refs.stdin_senders.get(session_id).cloned()
        };
        match sender {
            Some(s) => s.send(line).await.is_ok(),
            None => false,
        }
    }

    /// Drop a session's process refs without aborting them. Called
    /// by the drain task itself when it exits normally so terminal
    /// state lookups still work but the resources are reclaimed.
    pub fn detach_process(&self, session_id: &str) {
        let mut refs = self.process_refs.lock().unwrap();
        refs.drain_tasks.remove(session_id);
        refs.stdin_senders.remove(session_id);
        refs.pty_pids.remove(session_id);
    }

    /// Read the full transcript of captured stdout/stderr lines.
    /// Used by integration tests; exposed for completeness even
    /// though no production caller currently reads it.
    #[allow(dead_code)]
    pub fn transcript(&self, session_id: &str) -> Option<Vec<String>> {
        self.sessions
            .lock()
            .unwrap()
            .get(session_id)
            .map(|s| s.transcript.clone())
    }

    /// Inject a force-elapsed `started_at` for the given session —
    /// test helper so we can exercise the timeout transition without
    /// actually waiting SESSION_TIMEOUT_SECS.
    #[cfg(test)]
    fn force_age(&self, session_id: &str) {
        if let Some(s) = self.sessions.lock().unwrap().get_mut(session_id) {
            s.started_at = Instant::now() - Duration::from_secs(SESSION_TIMEOUT_SECS + 1);
        }
    }
}

/// Platform-specific best-effort kill of a child process by PID.
/// Mirror of the cef-side helper in `agentmux-cef::commands::platform`.
#[cfg(windows)]
fn kill_pid(pid: u32) -> std::io::Result<()> {
    let status = std::process::Command::new("taskkill")
        .args(["/F", "/T", "/PID", &pid.to_string()])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(std::io::Error::other(format!("taskkill exit {:?}", status.code())))
    }
}

#[cfg(unix)]
fn kill_pid(pid: u32) -> std::io::Result<()> {
    let ret = unsafe { libc::kill(pid as i32, libc::SIGTERM) };
    if ret != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mgr() -> AuthSessionManager {
        AuthSessionManager::new()
    }

    #[test]
    fn start_creates_pending_session() {
        let m = mgr();
        let r = m.start_session("claude".to_string(), None);
        assert!(!r.session_id.is_empty());
        assert!(r.auth_url.is_none());
        let p = m.poll_session(&r.session_id).expect("session exists");
        assert_eq!(p.provider_id, "claude");
        assert!(matches!(p.status, AuthSessionStatus::Pending));
    }

    #[test]
    fn url_line_transitions_to_url_available() {
        let m = mgr();
        let r = m.start_session("claude".to_string(), None);
        let _ = m.record_line(
            &r.session_id,
            "Open https://console.anthropic.com/oauth/authorize?state=xyz",
        );
        let p = m.poll_session(&r.session_id).unwrap();
        match p.status {
            AuthSessionStatus::UrlAvailable { auth_url } => {
                assert!(auth_url.contains("anthropic.com/oauth"));
            }
            other => panic!("expected UrlAvailable, got {other:?}"),
        }
    }

    #[test]
    fn device_code_line_transitions_to_code_emitted() {
        let m = mgr();
        let r = m.start_session("copilot".to_string(), None);
        let _ = m.record_line(&r.session_id, "! Copy your one-time code: ABCD-1234");
        let p = m.poll_session(&r.session_id).unwrap();
        match p.status {
            AuthSessionStatus::CodeEmitted {
                device_code,
                verification_url,
            } => {
                assert_eq!(device_code, "ABCD-1234");
                assert_eq!(verification_url, "https://github.com/login/device");
            }
            other => panic!("expected CodeEmitted, got {other:?}"),
        }
    }

    #[test]
    fn login_success_line_does_not_transition_state_alone() {
        // The CLI saying "logged in" isn't enough — the handler
        // confirms via authCheck before declaring Success.
        let m = mgr();
        let r = m.start_session("claude".to_string(), None);
        let _ = m.record_line(&r.session_id, "Successfully logged in as asaf@example.com");
        let p = m.poll_session(&r.session_id).unwrap();
        // Still pending — handler hasn't called finish_success yet.
        assert!(matches!(p.status, AuthSessionStatus::Pending));
    }

    #[test]
    fn finish_success_carries_email_from_transcript() {
        let m = mgr();
        let r = m.start_session("claude".to_string(), None);
        let _ = m.record_line(&r.session_id, "Successfully logged in as asaf@example.com");
        assert!(m.finish_success(&r.session_id, "bundle-1".to_string(), None));
        let p = m.poll_session(&r.session_id).unwrap();
        match p.status {
            AuthSessionStatus::Success { bundle_id, email, account_id } => {
                assert_eq!(bundle_id, "bundle-1");
                assert_eq!(email.as_deref(), Some("asaf@example.com"));
                assert_eq!(account_id, None);
            }
            other => panic!("expected Success, got {other:?}"),
        }
    }

    #[test]
    fn cancel_transitions_to_failed() {
        let m = mgr();
        let r = m.start_session("claude".to_string(), None);
        assert!(m.cancel_session(&r.session_id));
        let p = m.poll_session(&r.session_id).unwrap();
        match p.status {
            AuthSessionStatus::Failed { error } => assert!(error.contains("cancelled")),
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[test]
    // FLAKY on low-uptime machines (CI runners): force_age does
    // `Instant::now() - Duration::from_secs(SESSION_TIMEOUT_SECS + 1)`, which
    // PANICS when monotonic uptime is less than the timeout (can't construct an
    // Instant that far in the past). Passes locally (high uptime), fails on a
    // freshly-booted CI runner. Production is unaffected — it never subtracts from
    // Instant. Ignored to unblock CI; fix = make the timeout mockable (deadline
    // model / injectable clock) so force_age doesn't underflow.
    // SPEC_CI_TEST_RUNNER_2026_06_22.md §6.4.
    #[ignore = "force_age underflows Instant on low-uptime CI runners; make the timeout mockable, then un-ignore"]
    fn timeout_transitions_pending_to_failed_on_poll() {
        let m = mgr();
        let r = m.start_session("claude".to_string(), None);
        m.force_age(&r.session_id);
        let p = m.poll_session(&r.session_id).unwrap();
        match p.status {
            AuthSessionStatus::Failed { error } => assert!(error.contains("timed out")),
            other => panic!("expected timeout Failed, got {other:?}"),
        }
    }

    #[test]
    fn terminal_states_cannot_be_re_transitioned() {
        let m = mgr();
        let r = m.start_session("claude".to_string(), None);
        assert!(m.finish_success(&r.session_id, "bundle-1".to_string(), None));
        // Second finish_failure is a no-op.
        assert!(!m.finish_failure(&r.session_id, "should be ignored".to_string()));
        let p = m.poll_session(&r.session_id).unwrap();
        assert!(matches!(p.status, AuthSessionStatus::Success { .. }));
    }

    #[test]
    fn multiple_url_lines_keep_the_first_url() {
        // If the CLI emits the URL again later (some do), we don't
        // overwrite — the first URL is what the user saw + pasted.
        let m = mgr();
        let r = m.start_session("claude".to_string(), None);
        let _ = m.record_line(
            &r.session_id,
            "Open https://console.anthropic.com/oauth/authorize?state=first",
        );
        let _ = m.record_line(
            &r.session_id,
            "Open https://console.anthropic.com/oauth/authorize?state=second",
        );
        let p = m.poll_session(&r.session_id).unwrap();
        match p.status {
            AuthSessionStatus::UrlAvailable { auth_url } => {
                assert!(auth_url.contains("state=first"));
            }
            _ => panic!("expected UrlAvailable"),
        }
    }

    #[test]
    fn status_serializes_with_camelcase_field_names() {
        // Codex P2 on PR #840: the per-variant fields used to
        // serialize as auth_url / device_code / etc. (snake_case)
        // while the rest of the wire is camelCase. Now uniformly
        // camelCase via rename_all_fields.
        let s = AuthSessionStatus::UrlAvailable {
            auth_url: "https://example.com/oauth".to_string(),
        };
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(
            v,
            serde_json::json!({
                "status": "url-available",
                "authUrl": "https://example.com/oauth"
            })
        );

        let s = AuthSessionStatus::CodeEmitted {
            device_code: "ABCD-1234".to_string(),
            verification_url: "https://github.com/login/device".to_string(),
        };
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(
            v,
            serde_json::json!({
                "status": "code-emitted",
                "deviceCode": "ABCD-1234",
                "verificationUrl": "https://github.com/login/device"
            })
        );

        let s = AuthSessionStatus::Success {
            bundle_id: "bundle-1".to_string(),
            email: Some("asaf@example.com".to_string()),
            account_id: None,
        };
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(
            v,
            serde_json::json!({
                "status": "success",
                "bundleId": "bundle-1",
                "email": "asaf@example.com"
            })
        );

        // Direct-account mode (issue #1624 PR-C Part B) — accountId
        // appears on the wire; bundleId is present but empty.
        let s = AuthSessionStatus::Success {
            bundle_id: String::new(),
            email: Some("asaf@example.com".to_string()),
            account_id: Some("acc-1".to_string()),
        };
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(
            v,
            serde_json::json!({
                "status": "success",
                "bundleId": "",
                "email": "asaf@example.com",
                "accountId": "acc-1"
            })
        );
    }

    #[test]
    fn unknown_session_polls_to_none() {
        let m = mgr();
        assert!(m.poll_session("does-not-exist").is_none());
    }

    #[test]
    fn transcript_records_all_lines_including_non_matching() {
        let m = mgr();
        let r = m.start_session("claude".to_string(), None);
        let _ = m.record_line(&r.session_id, "Starting auth flow...");
        let _ = m.record_line(&r.session_id, "Open https://console.anthropic.com/oauth/authorize");
        let _ = m.record_line(&r.session_id, "Waiting for callback...");
        let t = m.transcript(&r.session_id).unwrap();
        assert_eq!(t.len(), 3);
        assert_eq!(t[0], "Starting auth flow...");
        assert!(t[1].contains("anthropic.com/oauth"));
        assert_eq!(t[2], "Waiting for callback...");
    }
}
