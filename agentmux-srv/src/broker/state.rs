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
    /// `generation` is the value from the `RunFreshnessCheck` event that
    /// triggered this call — see `Event::RunFreshnessCheck`'s doc comment.
    FreshnessChecked {
        id: String,
        is_fresh: bool,
        generation: u64,
    },
    /// `generation` carried forward from the `RunRefresh` event that
    /// triggered this call.
    RefreshSucceeded { id: String, generation: u64 },
    RefreshFailed {
        id: String,
        error: RefreshErrorKind,
        generation: u64,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// Orchestrator should call the registered `is_fresh()` closure now.
    /// `generation` is this id's registration epoch at dispatch time — the
    /// orchestrator must echo it back unchanged in the resulting
    /// `Command::FreshnessChecked` (and, if a refresh follows, in
    /// `RefreshSucceeded`/`RefreshFailed` too). This lets the reducer detect
    /// a stale completion: if `register()` replaces this id's closures
    /// (bumping its generation) while `is_fresh()`/`refresh()` is still
    /// awaiting, the eventual result would otherwise silently clobber the
    /// freshly re-registered credential's state with a verdict about
    /// closures that no longer apply (reagent re-review on #2275).
    RunFreshnessCheck { id: String, generation: u64 },
    /// Orchestrator should call the registered `refresh()` closure now.
    /// `generation` carries the same epoch forward from `RunFreshnessCheck`.
    RunRefresh { id: String, generation: u64 },
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

/// `states` is the orchestrator's whole coordination table; `generations`
/// tracks each id's CURRENT registration epoch so a completion from a
/// superseded registration can be told apart from a current one (see
/// `Event::RunFreshnessCheck`'s doc comment); `next_generation` is a single
/// counter shared across ALL ids, strictly increasing, never reused —
/// deliberately NOT a per-id counter restarted at 0 on each `Register`,
/// because an unregister-then-re-register sequence would then hand the new
/// registration the SAME generation number the old one had, and a stale
/// completion from the old registration (still in flight when the
/// unregister/re-register raced in) would wrongly pass the staleness check
/// instead of being discarded (reagent re-review on #2275). One reducer
/// call per dispatched command, sub-millisecond, never awaits.
pub fn update(
    states: &mut HashMap<String, CredentialState>,
    generations: &mut HashMap<String, u64>,
    next_generation: &mut u64,
    cmd: Command,
) -> Vec<Event> {
    match cmd {
        Command::Register { id } => {
            // Replacing an already-registered id is safe here specifically
            // because single-flight is guarded by THIS reducer's own state,
            // not by an external lock a caller could be left blocked on
            // (the pre-refactor design's latent race). A caller currently
            // waiting via AlreadyInFlight re-dispatches CheckRequested once
            // woken and just sees the fresh entry's is_fresh() result.
            let generation = *next_generation;
            *next_generation += 1;
            generations.insert(id.clone(), generation);
            states.insert(id, CredentialState::Fresh);
            Vec::new()
        }
        Command::Unregister { id } => {
            // Removing the entry here is just memory hygiene, not a
            // correctness requirement — next_generation never reuses a
            // number, so a future re-registration can't collide with a
            // stale completion's captured generation either way.
            generations.remove(&id);
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
                let generation = *generations.get(&id).unwrap_or(&0);
                vec![Event::RunFreshnessCheck { id, generation }]
            }
        },
        Command::FreshnessChecked {
            id,
            is_fresh,
            generation,
        } => {
            if generations.get(&id).copied() != Some(generation) {
                // Stale completion: id was unregistered (generation gone)
                // or re-registered (generation bumped) while is_fresh() was
                // still awaiting. Either way this result is about closures
                // that no longer apply — discard it rather than clobber the
                // current epoch's state (reagent re-review on #2275).
                return Vec::new();
            }
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
                vec![Event::RunRefresh { id, generation }]
            }
        }
        Command::RefreshSucceeded { id, generation } => {
            if generations.get(&id).copied() != Some(generation) {
                // Same stale-epoch hazard as FreshnessChecked above.
                return Vec::new();
            }
            states.insert(id.clone(), CredentialState::Fresh);
            vec![Event::BecameFresh { id }]
        }
        Command::RefreshFailed {
            id,
            error,
            generation,
        } => {
            if generations.get(&id).copied() != Some(generation) {
                // Same stale-epoch hazard as FreshnessChecked above.
                return Vec::new();
            }
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

    /// Bundles `states` + `generations` + `next_generation` so tests can
    /// dispatch commands without threading three pieces of state through
    /// every call site by hand.
    #[derive(Default)]
    struct Harness {
        states: HashMap<String, CredentialState>,
        generations: HashMap<String, u64>,
        next_generation: u64,
    }

    impl Harness {
        fn dispatch(&mut self, cmd: Command) -> Vec<Event> {
            update(
                &mut self.states,
                &mut self.generations,
                &mut self.next_generation,
                cmd,
            )
        }

        fn get(&self, id: &str) -> Option<&CredentialState> {
            self.states.get(id)
        }

        fn contains_key(&self, id: &str) -> bool {
            self.states.contains_key(id)
        }

        fn generation_of(&self, id: &str) -> u64 {
            *self.generations.get(id).expect("id should be registered")
        }
    }

    fn registered() -> Harness {
        let mut h = Harness::default();
        h.dispatch(Command::Register { id: "cred".into() });
        h
    }

    #[test]
    fn register_starts_fresh() {
        let m = registered();
        assert_eq!(m.get("cred"), Some(&CredentialState::Fresh));
    }

    #[test]
    fn check_on_fresh_transitions_to_refreshing_and_requests_a_check() {
        let mut m = registered();
        let gen = m.generation_of("cred");
        let events = m.dispatch(Command::CheckRequested { id: "cred".into() });
        assert_eq!(
            events,
            vec![Event::RunFreshnessCheck {
                id: "cred".into(),
                generation: gen
            }]
        );
        assert_eq!(
            m.get("cred"),
            Some(&CredentialState::Refreshing { prior_failures: 0 })
        );
    }

    #[test]
    fn a_second_check_while_refreshing_gets_already_in_flight_not_a_second_run() {
        let mut m = registered();
        m.dispatch(Command::CheckRequested { id: "cred".into() });
        let events = m.dispatch(Command::CheckRequested { id: "cred".into() });
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
        let gen = m.generation_of("cred");
        m.dispatch(Command::CheckRequested { id: "cred".into() });
        let events = m.dispatch(Command::FreshnessChecked {
            id: "cred".into(),
            is_fresh: true,
            generation: gen,
        });
        assert_eq!(m.get("cred"), Some(&CredentialState::Fresh));
        assert_eq!(events, vec![Event::BecameFresh { id: "cred".into() }]);
    }

    #[test]
    fn freshness_checked_false_requests_a_refresh_and_stays_refreshing() {
        let mut m = registered();
        let gen = m.generation_of("cred");
        m.dispatch(Command::CheckRequested { id: "cred".into() });
        let events = m.dispatch(Command::FreshnessChecked {
            id: "cred".into(),
            is_fresh: false,
            generation: gen,
        });
        assert_eq!(
            events,
            vec![Event::RunRefresh {
                id: "cred".into(),
                generation: gen
            }]
        );
        assert_eq!(
            m.get("cred"),
            Some(&CredentialState::Refreshing { prior_failures: 0 })
        );
    }

    #[test]
    fn refresh_succeeded_returns_to_fresh_and_resets_any_failure_count() {
        let mut m = registered();
        let gen = m.generation_of("cred");
        m.dispatch(Command::CheckRequested { id: "cred".into() });
        m.dispatch(Command::RefreshFailed {
            id: "cred".into(),
            error: RefreshErrorKind::Transient("boom".into()),
            generation: gen,
        });
        m.dispatch(Command::CheckRequested { id: "cred".into() });
        let events = m.dispatch(Command::RefreshSucceeded {
            id: "cred".into(),
            generation: gen,
        });
        assert_eq!(events, vec![Event::BecameFresh { id: "cred".into() }]);
        assert_eq!(m.get("cred"), Some(&CredentialState::Fresh));
    }

    #[test]
    fn refresh_failed_transient_moves_to_failed_with_count_one() {
        let mut m = registered();
        let gen = m.generation_of("cred");
        m.dispatch(Command::CheckRequested { id: "cred".into() });
        let events = m.dispatch(Command::RefreshFailed {
            id: "cred".into(),
            error: RefreshErrorKind::Transient("network blip".into()),
            generation: gen,
        });
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
        let gen = m.generation_of("cred");
        m.dispatch(Command::CheckRequested { id: "cred".into() });
        m.dispatch(Command::RefreshFailed {
            id: "cred".into(),
            error: RefreshErrorKind::Transient("nope".into()),
            generation: gen,
        });
        assert!(!matches!(m.get("cred"), Some(CredentialState::Fresh)));
        let events = m.dispatch(Command::CheckRequested { id: "cred".into() });
        assert_eq!(
            events,
            vec![Event::RunFreshnessCheck {
                id: "cred".into(),
                generation: gen
            }]
        );
    }

    #[test]
    fn permanent_auth_failure_skips_straight_to_needs_reauth_on_the_first_failure() {
        let mut m = registered();
        let gen = m.generation_of("cred");
        m.dispatch(Command::CheckRequested { id: "cred".into() });
        let events = m.dispatch(Command::RefreshFailed {
            id: "cred".into(),
            error: RefreshErrorKind::PermanentAuthFailure("invalid_grant".into()),
            generation: gen,
        });
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
        let gen = m.generation_of("cred");
        for i in 1..=(NEEDS_REAUTH_THRESHOLD - 1) {
            m.dispatch(Command::CheckRequested { id: "cred".into() });
            let events = m.dispatch(Command::RefreshFailed {
                id: "cred".into(),
                error: RefreshErrorKind::Transient(format!("attempt {i}")),
                generation: gen,
            });
            assert!(
                matches!(events.as_slice(), [Event::BecameFailed { .. }]),
                "attempt {i} should still be a transient Failed, not NeedsReauth yet"
            );
        }
        m.dispatch(Command::CheckRequested { id: "cred".into() });
        let events = m.dispatch(Command::RefreshFailed {
            id: "cred".into(),
            error: RefreshErrorKind::Transient("final straw".into()),
            generation: gen,
        });
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
        let gen = m.generation_of("cred");
        m.dispatch(Command::CheckRequested { id: "cred".into() });
        m.dispatch(Command::RefreshFailed {
            id: "cred".into(),
            error: RefreshErrorKind::PermanentAuthFailure("dead".into()),
            generation: gen,
        });
        let events = m.dispatch(Command::CheckRequested { id: "cred".into() });
        assert_eq!(
            events,
            vec![Event::RunFreshnessCheck {
                id: "cred".into(),
                generation: gen
            }]
        );
    }

    #[test]
    fn check_requested_on_an_unregistered_id_reports_unregistered() {
        let mut m = Harness::default();
        let events = m.dispatch(Command::CheckRequested { id: "nope".into() });
        assert_eq!(events, vec![Event::Unregistered { id: "nope".into() }]);
    }

    #[test]
    fn unregister_then_check_reports_unregistered() {
        let mut m = registered();
        m.dispatch(Command::Unregister { id: "cred".into() });
        let events = m.dispatch(Command::CheckRequested { id: "cred".into() });
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
        m.dispatch(Command::CheckRequested { id: "cred".into() });
        assert_eq!(
            m.get("cred"),
            Some(&CredentialState::Refreshing { prior_failures: 0 })
        );
        m.dispatch(Command::Register { id: "cred".into() });
        assert_eq!(m.get("cred"), Some(&CredentialState::Fresh));
        let new_gen = m.generation_of("cred");
        let events = m.dispatch(Command::CheckRequested { id: "cred".into() });
        assert_eq!(
            events,
            vec![Event::RunFreshnessCheck {
                id: "cred".into(),
                generation: new_gen
            }]
        );
    }

    #[test]
    fn a_stale_completion_after_unregister_does_not_resurrect_a_phantom_entry() {
        // reagent re-review on #2275: an in-flight check/refresh that loses
        // a race with a concurrent unregister() must not reinsert an entry
        // for an id nothing owns anymore — the sweep loop only walks
        // registered closures, so a resurrected entry would leak forever
        // and scheduler.state(id) would misreport it as still registered.
        fn mid_check_then_unregistered() -> (Harness, u64) {
            let mut m = registered();
            let gen = m.generation_of("cred");
            m.dispatch(Command::CheckRequested { id: "cred".into() });
            m.dispatch(Command::Unregister { id: "cred".into() });
            assert!(!m.contains_key("cred"));
            (m, gen)
        }

        let stale_completion_builders: [fn(u64) -> Command; 4] = [
            |gen| Command::FreshnessChecked {
                id: "cred".into(),
                is_fresh: true,
                generation: gen,
            },
            |gen| Command::FreshnessChecked {
                id: "cred".into(),
                is_fresh: false,
                generation: gen,
            },
            |gen| Command::RefreshSucceeded {
                id: "cred".into(),
                generation: gen,
            },
            |gen| Command::RefreshFailed {
                id: "cred".into(),
                error: RefreshErrorKind::Transient("late".into()),
                generation: gen,
            },
        ];
        for build_stale_completion in stale_completion_builders {
            let (mut m, gen) = mid_check_then_unregistered();
            let events = m.dispatch(build_stale_completion(gen));
            assert_eq!(events, Vec::new());
            assert!(
                !m.contains_key("cred"),
                "a stale completion must not resurrect the entry"
            );
        }
    }

    #[test]
    fn a_stale_completion_after_re_register_does_not_clobber_the_new_epoch() {
        // reagent re-review on #2275: if register() is called again for an
        // id while an OLD in-flight check/refresh for that same id is still
        // awaiting, the stale completion (belonging to the pre-registration
        // closures) must not overwrite the freshly re-registered
        // credential's Fresh state — it belongs to a superseded epoch and
        // has nothing meaningful to say about the current one.
        let mut m = registered();
        let old_gen = m.generation_of("cred");
        m.dispatch(Command::CheckRequested { id: "cred".into() });

        // Re-register while the "old" check/refresh is still in flight —
        // bumps the generation and resets to Fresh.
        m.dispatch(Command::Register { id: "cred".into() });
        assert_eq!(m.get("cred"), Some(&CredentialState::Fresh));
        let new_gen = m.generation_of("cred");
        assert_ne!(old_gen, new_gen);

        // The old attempt's belated failure must not land.
        let events = m.dispatch(Command::RefreshFailed {
            id: "cred".into(),
            error: RefreshErrorKind::PermanentAuthFailure("stale invalid_grant".into()),
            generation: old_gen,
        });
        assert_eq!(events, Vec::new());
        assert_eq!(
            m.get("cred"),
            Some(&CredentialState::Fresh),
            "a stale completion from a superseded registration must not mask the new one"
        );
    }

    #[test]
    fn a_stale_completion_survives_an_unregister_then_re_register_of_the_same_id() {
        // reagent re-review on #2275, second round: a PER-ID counter that
        // restarts at 0 on each Register would hand an unregister-then-
        // re-register sequence the SAME generation number the original
        // registration had, so a stale completion from before the
        // unregister would wrongly pass the staleness check after the
        // re-register instead of being discarded. next_generation is a
        // single counter shared across every id specifically so this can't
        // happen — assert the two registrations never collide even across
        // an intervening unregister.
        let mut m = registered();
        let old_gen = m.generation_of("cred");
        m.dispatch(Command::CheckRequested { id: "cred".into() });

        m.dispatch(Command::Unregister { id: "cred".into() });
        m.dispatch(Command::Register { id: "cred".into() });
        let new_gen = m.generation_of("cred");
        assert_ne!(
            old_gen, new_gen,
            "unregister-then-register must never reissue a prior generation number"
        );

        let events = m.dispatch(Command::RefreshFailed {
            id: "cred".into(),
            error: RefreshErrorKind::PermanentAuthFailure("stale invalid_grant".into()),
            generation: old_gen,
        });
        assert_eq!(events, Vec::new());
        assert_eq!(
            m.get("cred"),
            Some(&CredentialState::Fresh),
            "a stale completion from before the unregister must not mask the re-registered credential"
        );
    }
}
