// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Pure reducer for the Credential Broker's per-credential coordination
//! state — fresh / refreshing / failed / needs-reauth. Same shape as every
//! other reducer in this codebase (`agentmux-launcher/src/reducer/`,
//! `agentmux-srv/src/reducer.rs`, `agentmux-cef/src/reducer/mod.rs`):
//! `update(&mut State, Command) -> Vec<Event>`, no I/O, no async,
//! sub-millisecond lock hold when called from `scheduler.rs`'s orchestrator.
//!
//! Internal-only, like the CEF host reducer — this never crosses IPC, so it
//! does not reuse `agentmux_common::ipc::{Command, Event}` (the
//! srv<->launcher<->host wire protocol); it defines its own local types.
//!
//! This module never holds a credential's actual secret/value — only enough
//! coordination state to decide WHEN `scheduler.rs`'s caller-supplied
//! `is_fresh`/`refresh` closures should run, and to make that decision
//! observable. Events are mapped 1:1 to `auth.broker.*`-prefixed `tracing`
//! calls by the orchestrator — that prefix lands in `muxlog auth`'s existing
//! filter regex (`\bauth\.\w+|...`) with zero changes needed there.

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

/// After this many consecutive TRANSIENT refresh failures, a credential is
/// considered permanently broken rather than retried forever — the sweep
/// loop stops hammering a dead refresh_token on every tick. `ensure_fresh`
/// (on-demand, e.g. right before a reconnect attempt) still re-checks
/// freshness even from `NeedsReauth`: if an external login has since
/// succeeded, `is_fresh()` will report true and state naturally moves back
/// to `Fresh`. No explicit "clear" API needed.
pub const NEEDS_REAUTH_THRESHOLD: u32 = 5;

/// Coordination state for one registered credential.
#[derive(Debug, Clone, PartialEq)]
pub enum CredentialState {
    /// Registered; last check (if any) said fresh, or never checked yet.
    Fresh,
    /// A freshness check or refresh attempt is currently in flight — this
    /// IS the single-flight guard: a second `CheckRequested` while already
    /// `Refreshing` gets `Event::AlreadyInFlight`, never a second
    /// `RunFreshnessCheck`/`RunRefresh`. Carries forward the failure count
    /// from whatever `Failed` state (if any) preceded this attempt — the
    /// only place that count lives, since transitioning through
    /// `Refreshing` would otherwise lose it before `RefreshFailed` can read
    /// it back out to increment.
    Refreshing { prior_failures: u32 },
    /// Last refresh attempt failed with a transient error (network, parse,
    /// etc.) — will be retried on the next `ensure_fresh`/sweep tick.
    Failed {
        consecutive_failures: u32,
        last_error: String,
    },
    /// Either `consecutive_failures` crossed `NEEDS_REAUTH_THRESHOLD`, or a
    /// refresh closure explicitly reported
    /// `RefreshErrorKind::PermanentAuthFailure` (no need to accumulate
    /// failures for an error already known permanent). Retrying
    /// automatically is pointless; a human needs to log in again.
    NeedsReauth { since_unix: u64, reason: String },
}

/// What a registered refresh closure's failure means for scheduling.
#[derive(Debug, Clone, PartialEq)]
pub enum RefreshErrorKind {
    /// Network blip, parse failure, transient server error — worth retrying.
    Transient(String),
    /// The credential itself is rejected (e.g. an OAuth server's
    /// `invalid_grant` for a revoked/expired refresh_token) — retrying with
    /// the SAME credential will never succeed; only a fresh login fixes it.
    PermanentAuthFailure(String),
}

impl RefreshErrorKind {
    fn message(&self) -> &str {
        match self {
            RefreshErrorKind::Transient(m) | RefreshErrorKind::PermanentAuthFailure(m) => m,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    Register { id: String },
    Unregister { id: String },
    /// `ensure_fresh` or a sweep tick wants to know/ensure this credential
    /// is fresh.
    CheckRequested { id: String },
    /// The orchestrator ran `is_fresh()` and is reporting the result.
    FreshnessChecked { id: String, is_fresh: bool },
    RefreshSucceeded { id: String },
    RefreshFailed { id: String, error: RefreshErrorKind },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// Orchestrator should call the registered `is_fresh()` closure now.
    RunFreshnessCheck { id: String },
    /// Orchestrator should call the registered `refresh()` closure now.
    RunRefresh { id: String },
    /// Another caller is already checking/refreshing this id — the
    /// orchestrator should wait on the id's `Notify` and re-dispatch
    /// `CheckRequested` once woken.
    AlreadyInFlight { id: String },
    /// `id` was never registered (or was just unregistered).
    Unregistered { id: String },
    /// Diagnostics-only — mapped 1:1 to `tracing` by the orchestrator.
    BecameFresh { id: String },
    BecameFailed {
        id: String,
        consecutive_failures: u32,
        error: String,
    },
    BecameNeedsReauth { id: String, reason: String },
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// `states` is the orchestrator's whole coordination table — one reducer
/// call per dispatched command, sub-millisecond, never awaits.
pub fn update(states: &mut HashMap<String, CredentialState>, cmd: Command) -> Vec<Event> {
    match cmd {
        Command::Register { id } => {
            // Replacing an already-registered id is safe here specifically
            // because single-flight is guarded by THIS reducer's own state,
            // not by an external lock a caller could be left blocked on
            // (the pre-refactor design's latent race). A caller currently
            // waiting via AlreadyInFlight re-dispatches CheckRequested once
            // woken and just sees the fresh entry's is_fresh() result.
            states.insert(id, CredentialState::Fresh);
            Vec::new()
        }
        Command::Unregister { id } => {
            states.remove(&id);
            vec![Event::Unregistered { id }]
        }
        Command::CheckRequested { id } => match states.get(&id) {
            None => vec![Event::Unregistered { id }],
            Some(CredentialState::Refreshing { .. }) => vec![Event::AlreadyInFlight { id }],
            Some(state) => {
                // Carry the failure count forward through the Refreshing
                // transition — it's the only place it lives, so overwriting
                // it with a bare Refreshing here would otherwise strand
                // RefreshFailed with no way to read the prior count back
                // out and every failure would reset to 1.
                let prior_failures = match state {
                    CredentialState::Failed {
                        consecutive_failures,
                        ..
                    } => *consecutive_failures,
                    CredentialState::Fresh | CredentialState::NeedsReauth { .. } => 0,
                    CredentialState::Refreshing { .. } => unreachable!("matched above"),
                };
                states.insert(id.clone(), CredentialState::Refreshing { prior_failures });
                vec![Event::RunFreshnessCheck { id }]
            }
        },
        Command::FreshnessChecked { id, is_fresh } => {
            if is_fresh {
                let became_fresh = !matches!(states.get(&id), Some(CredentialState::Fresh));
                states.insert(id.clone(), CredentialState::Fresh);
                if became_fresh {
                    vec![Event::BecameFresh { id }]
                } else {
                    Vec::new()
                }
            } else {
                // Stays Refreshing — orchestrator now runs the actual refresh.
                vec![Event::RunRefresh { id }]
            }
        }
        Command::RefreshSucceeded { id } => {
            states.insert(id.clone(), CredentialState::Fresh);
            vec![Event::BecameFresh { id }]
        }
        Command::RefreshFailed { id, error } => {
            let reason = error.message().to_string();
            let permanent = matches!(error, RefreshErrorKind::PermanentAuthFailure(_));
            let prior_failures = match states.get(&id) {
                Some(CredentialState::Refreshing { prior_failures }) => *prior_failures,
                _ => 0,
            };
            let consecutive_failures = prior_failures + 1;
            if permanent || consecutive_failures >= NEEDS_REAUTH_THRESHOLD {
                states.insert(
                    id.clone(),
                    CredentialState::NeedsReauth {
                        since_unix: now_unix(),
                        reason: reason.clone(),
                    },
                );
                vec![Event::BecameNeedsReauth { id, reason }]
            } else {
                states.insert(
                    id.clone(),
                    CredentialState::Failed {
                        consecutive_failures,
                        last_error: reason.clone(),
                    },
                );
                vec![Event::BecameFailed {
                    id,
                    consecutive_failures,
                    error: reason,
                }]
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registered() -> HashMap<String, CredentialState> {
        let mut m = HashMap::new();
        update(&mut m, Command::Register { id: "cred".into() });
        m
    }

    #[test]
    fn register_starts_fresh() {
        let m = registered();
        assert_eq!(m.get("cred"), Some(&CredentialState::Fresh));
    }

    #[test]
    fn check_on_fresh_transitions_to_refreshing_and_requests_a_check() {
        let mut m = registered();
        let events = update(&mut m, Command::CheckRequested { id: "cred".into() });
        assert_eq!(events, vec![Event::RunFreshnessCheck { id: "cred".into() }]);
        assert_eq!(
            m.get("cred"),
            Some(&CredentialState::Refreshing { prior_failures: 0 })
        );
    }

    #[test]
    fn a_second_check_while_refreshing_gets_already_in_flight_not_a_second_run() {
        let mut m = registered();
        update(&mut m, Command::CheckRequested { id: "cred".into() });
        let events = update(&mut m, Command::CheckRequested { id: "cred".into() });
        assert_eq!(events, vec![Event::AlreadyInFlight { id: "cred".into() }]);
        // Still just one credential, still Refreshing — the second dispatch
        // did not disturb the in-flight state.
        assert_eq!(
            m.get("cred"),
            Some(&CredentialState::Refreshing { prior_failures: 0 })
        );
    }

    #[test]
    fn freshness_checked_true_returns_to_fresh_and_reports_became_fresh() {
        let mut m = registered();
        update(&mut m, Command::CheckRequested { id: "cred".into() });
        let events = update(
            &mut m,
            Command::FreshnessChecked {
                id: "cred".into(),
                is_fresh: true,
            },
        );
        assert_eq!(m.get("cred"), Some(&CredentialState::Fresh));
        assert_eq!(events, vec![Event::BecameFresh { id: "cred".into() }]);
    }

    #[test]
    fn freshness_checked_false_requests_a_refresh_and_stays_refreshing() {
        let mut m = registered();
        update(&mut m, Command::CheckRequested { id: "cred".into() });
        let events = update(
            &mut m,
            Command::FreshnessChecked {
                id: "cred".into(),
                is_fresh: false,
            },
        );
        assert_eq!(events, vec![Event::RunRefresh { id: "cred".into() }]);
        assert_eq!(
            m.get("cred"),
            Some(&CredentialState::Refreshing { prior_failures: 0 })
        );
    }

    #[test]
    fn refresh_succeeded_returns_to_fresh_and_resets_any_failure_count() {
        let mut m = registered();
        update(&mut m, Command::CheckRequested { id: "cred".into() });
        update(
            &mut m,
            Command::RefreshFailed {
                id: "cred".into(),
                error: RefreshErrorKind::Transient("boom".into()),
            },
        );
        update(&mut m, Command::CheckRequested { id: "cred".into() });
        let events = update(&mut m, Command::RefreshSucceeded { id: "cred".into() });
        assert_eq!(events, vec![Event::BecameFresh { id: "cred".into() }]);
        assert_eq!(m.get("cred"), Some(&CredentialState::Fresh));
    }

    #[test]
    fn refresh_failed_transient_moves_to_failed_with_count_one() {
        let mut m = registered();
        update(&mut m, Command::CheckRequested { id: "cred".into() });
        let events = update(
            &mut m,
            Command::RefreshFailed {
                id: "cred".into(),
                error: RefreshErrorKind::Transient("network blip".into()),
            },
        );
        assert_eq!(
            events,
            vec![Event::BecameFailed {
                id: "cred".into(),
                consecutive_failures: 1,
                error: "network blip".into(),
            }]
        );
        assert_eq!(
            m.get("cred"),
            Some(&CredentialState::Failed {
                consecutive_failures: 1,
                last_error: "network blip".into(),
            })
        );
    }

    #[test]
    fn a_failed_refresh_does_not_mark_the_credential_fresh() {
        // Port of the pre-refactor scheduler test — a second check after a
        // failure must retry (not treat the failed attempt as success).
        let mut m = registered();
        update(&mut m, Command::CheckRequested { id: "cred".into() });
        update(
            &mut m,
            Command::RefreshFailed {
                id: "cred".into(),
                error: RefreshErrorKind::Transient("nope".into()),
            },
        );
        assert!(!matches!(m.get("cred"), Some(CredentialState::Fresh)));
        let events = update(&mut m, Command::CheckRequested { id: "cred".into() });
        assert_eq!(events, vec![Event::RunFreshnessCheck { id: "cred".into() }]);
    }

    #[test]
    fn permanent_auth_failure_skips_straight_to_needs_reauth_on_the_first_failure() {
        let mut m = registered();
        update(&mut m, Command::CheckRequested { id: "cred".into() });
        let events = update(
            &mut m,
            Command::RefreshFailed {
                id: "cred".into(),
                error: RefreshErrorKind::PermanentAuthFailure("invalid_grant".into()),
            },
        );
        assert!(matches!(
            events.as_slice(),
            [Event::BecameNeedsReauth { .. }]
        ));
        assert!(matches!(
            m.get("cred"),
            Some(CredentialState::NeedsReauth { .. })
        ));
    }

    #[test]
    fn five_consecutive_transient_failures_escalate_to_needs_reauth() {
        let mut m = registered();
        for i in 1..=(NEEDS_REAUTH_THRESHOLD - 1) {
            update(&mut m, Command::CheckRequested { id: "cred".into() });
            let events = update(
                &mut m,
                Command::RefreshFailed {
                    id: "cred".into(),
                    error: RefreshErrorKind::Transient(format!("attempt {i}")),
                },
            );
            assert!(
                matches!(events.as_slice(), [Event::BecameFailed { .. }]),
                "attempt {i} should still be a transient Failed, not NeedsReauth yet"
            );
        }
        update(&mut m, Command::CheckRequested { id: "cred".into() });
        let events = update(
            &mut m,
            Command::RefreshFailed {
                id: "cred".into(),
                error: RefreshErrorKind::Transient("final straw".into()),
            },
        );
        assert!(matches!(
            events.as_slice(),
            [Event::BecameNeedsReauth { .. }]
        ));
        assert!(matches!(
            m.get("cred"),
            Some(CredentialState::NeedsReauth { .. })
        ));
    }

    #[test]
    fn check_requested_on_needs_reauth_still_runs_a_fresh_check_on_demand() {
        // A human may have logged in again out-of-band (e.g. muxbus.login)
        // since the credential was marked NeedsReauth — on-demand checks
        // (unlike the sweep loop) must not just give up permanently.
        let mut m = registered();
        update(&mut m, Command::CheckRequested { id: "cred".into() });
        update(
            &mut m,
            Command::RefreshFailed {
                id: "cred".into(),
                error: RefreshErrorKind::PermanentAuthFailure("dead".into()),
            },
        );
        let events = update(&mut m, Command::CheckRequested { id: "cred".into() });
        assert_eq!(events, vec![Event::RunFreshnessCheck { id: "cred".into() }]);
    }

    #[test]
    fn check_requested_on_an_unregistered_id_reports_unregistered() {
        let mut m: HashMap<String, CredentialState> = HashMap::new();
        let events = update(&mut m, Command::CheckRequested { id: "nope".into() });
        assert_eq!(events, vec![Event::Unregistered { id: "nope".into() }]);
    }

    #[test]
    fn unregister_then_check_reports_unregistered() {
        let mut m = registered();
        update(&mut m, Command::Unregister { id: "cred".into() });
        let events = update(&mut m, Command::CheckRequested { id: "cred".into() });
        assert_eq!(events, vec![Event::Unregistered { id: "cred".into() }]);
    }

    #[test]
    fn re_registering_an_id_that_is_mid_refresh_does_not_leave_anything_orphaned() {
        // The pre-refactor design's latent race: register() on an
        // already-registered id replaced the Entry (including its lock), so
        // a caller already blocked on the OLD entry's lock waited forever.
        // Here there's no separate lock to orphan — Register always resets
        // cleanly to Fresh, and the next CheckRequested from any caller
        // (old or new) just sees that fresh state.
        let mut m = registered();
        update(&mut m, Command::CheckRequested { id: "cred".into() });
        assert_eq!(
            m.get("cred"),
            Some(&CredentialState::Refreshing { prior_failures: 0 })
        );
        update(&mut m, Command::Register { id: "cred".into() });
        assert_eq!(m.get("cred"), Some(&CredentialState::Fresh));
        let events = update(&mut m, Command::CheckRequested { id: "cred".into() });
        assert_eq!(events, vec![Event::RunFreshnessCheck { id: "cred".into() }]);
    }
}
