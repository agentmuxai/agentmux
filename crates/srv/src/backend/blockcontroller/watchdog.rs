// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

//! Agent process watchdog: kills agent panes that exceed max-runtime or idle-output limits.
//!
//! Runs every 60 seconds and inspects every running controller that reports
//! `is_agent_pane = true`: PTY agent panes and the persistent, ACP, MSP and
//! App Server controllers. Two independent kill conditions:
//!
//!   A) `term:agentmaxruntimehours` — wall-clock runtime since spawn (0 = disabled)
//!   B) `term:agentidletimeoutmins` — minutes without output (0 = disabled).
//!      A PTY pane is idle whenever its PTY is quiet. Any other agent is idle
//!      only while a turn is in flight and no question waits for the user:
//!      between turns, or on a question, the agent is waiting for the user,
//!      not stuck.
//!
//! Both limits default to 0 (disabled) so the watchdog is opt-in.

use std::sync::Arc;
use std::time::Duration;
use tokio::time::interval;

use crate::backend::storage::store::Store;
use crate::backend::wconfig::ConfigState;
use super::{get_all_controllers, STATUS_RUNNING};

/// Check interval for the watchdog loop.
const WATCHDOG_INTERVAL_SECS: u64 = 60;

/// The two limits, from settings. 0 (or less) disables one.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Limits {
    pub max_runtime_hours: f64,
    pub idle_timeout_mins: f64,
}

/// What the watchdog knows about one running agent pane (times in unix ms).
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct AgentClock {
    pub spawn_ms: Option<i64>,
    pub last_output_ms: Option<i64>,
    /// When the pass in flight started, if one is.
    pub pass_started_ms: Option<i64>,
    pub turn_active: bool,
    /// A question in the pane waits for the user (`term:awaiting_user`).
    pub awaiting_user: bool,
    /// A PTY pane (`ShellController`), idle whenever its PTY is quiet.
    pub is_shell: bool,
}

/// Why the watchdog stops an agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StopReason {
    MaxRuntime { elapsed_ms: i64 },
    IdleOutput { idle_ms: i64 },
}

/// Whether to stop this agent now, and why.
pub(crate) fn verdict(limits: Limits, now_ms: i64, agent: &AgentClock) -> Option<StopReason> {
    if limits.max_runtime_hours > 0.0 {
        if let Some(spawn_ms) = agent.spawn_ms {
            let elapsed_ms = (now_ms - spawn_ms).max(0);
            if elapsed_ms >= (limits.max_runtime_hours * 3_600_000.0) as i64 {
                return Some(StopReason::MaxRuntime { elapsed_ms });
            }
        }
    }
    if limits.idle_timeout_mins > 0.0 {
        let quiet_since = if agent.is_shell {
            agent.last_output_ms
        } else if agent.turn_active && !agent.awaiting_user {
            // Counted from the pass's start at the earliest: the time the agent
            // spent waiting for the user before it is not idleness.
            [agent.last_output_ms, agent.pass_started_ms, agent.spawn_ms].into_iter().flatten().max()
        } else {
            None
        };
        if let Some(since) = quiet_since {
            let idle_ms = (now_ms - since).max(0);
            if idle_ms >= (limits.idle_timeout_mins * 60_000.0) as i64 {
                return Some(StopReason::IdleOutput { idle_ms });
            }
        }
    }
    None
}

/// Whether a question in the pane waits for the user.
fn awaiting_user(store: &Store, block_id: &str) -> bool {
    store
        .get::<crate::backend::obj::Block>(block_id)
        .ok()
        .flatten()
        .is_some_and(|b| {
            crate::backend::obj::meta_get_bool(&b.meta, crate::backend::agent_state::META_AWAITING_USER, false)
        })
}

/// Run the agent watchdog loop. Never returns.
pub async fn run_watchdog_loop(config: Arc<ConfigState>, store: Arc<Store>) {
    let mut ticker = interval(Duration::from_secs(WATCHDOG_INTERVAL_SECS));
    loop {
        ticker.tick().await;
        let settings = config.get_settings();
        let limits = Limits {
            max_runtime_hours: settings.term_agent_max_runtime_hours,
            idle_timeout_mins: settings.term_agent_idle_timeout_mins,
        };

        // Skip entire scan if both limits are disabled.
        if limits.max_runtime_hours <= 0.0 && limits.idle_timeout_mins <= 0.0 {
            continue;
        }

        let now_ms = agentmux_common::time::now_ms();

        for (block_id, ctrl) in get_all_controllers() {
            let status = ctrl.get_runtime_status();
            if status.shellprocstatus != STATUS_RUNNING {
                continue;
            }
            if !status.is_agent_pane {
                continue;
            }
            let shell = ctrl.as_any().downcast_ref::<super::shell::ShellController>();
            let last_output_ms = match shell {
                Some(shell) => shell.last_output_secs_ago().map(|secs| now_ms - secs as i64 * 1000),
                None => ctrl.last_output_ms(),
            };
            let check_question = shell.is_none() && status.turn_active && limits.idle_timeout_mins > 0.0;
            let agent = AgentClock {
                spawn_ms: status.spawn_ts_ms,
                last_output_ms,
                pass_started_ms: ctrl.turn_tracker().and_then(|t| t.pass_started_ms()),
                turn_active: status.turn_active,
                awaiting_user: check_question && awaiting_user(&store, &block_id),
                is_shell: shell.is_some(),
            };

            match verdict(limits, now_ms, &agent) {
                Some(StopReason::MaxRuntime { elapsed_ms }) => {
                    tracing::warn!(
                        block_id = %block_id,
                        elapsed_hours = elapsed_ms / 3_600_000,
                        limit_hours = %limits.max_runtime_hours,
                        "watchdog: agent pane exceeded max-runtime, stopping"
                    );
                }
                Some(StopReason::IdleOutput { idle_ms }) => {
                    tracing::warn!(
                        block_id = %block_id,
                        idle_mins = idle_ms / 60_000,
                        limit_mins = %limits.idle_timeout_mins,
                        "watchdog: agent pane exceeded idle-output timeout, stopping"
                    );
                }
                None => continue,
            }
            let _ = crate::sagas::agent_teardown::stop_now(&block_id, true);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOUR: i64 = 3_600_000;
    const MIN: i64 = 60_000;
    const NOW: i64 = 1_800_000_000_000;

    fn limits(hours: f64, mins: f64) -> Limits {
        Limits { max_runtime_hours: hours, idle_timeout_mins: mins }
    }

    /// A non-shell agent spawned `age_ms` ago, mid-turn, silent for `quiet_ms`
    /// since a pass that started before it.
    fn mid_turn(age_ms: i64, quiet_ms: i64) -> AgentClock {
        AgentClock {
            spawn_ms: Some(NOW - age_ms),
            last_output_ms: Some(NOW - quiet_ms),
            pass_started_ms: Some(NOW - age_ms),
            turn_active: true,
            ..AgentClock::default()
        }
    }

    #[test]
    fn disabled_limits_never_stop_anything() {
        assert_eq!(verdict(limits(0.0, 0.0), NOW, &mid_turn(100 * HOUR, 100 * HOUR)), None);
    }

    #[test]
    fn max_runtime_stops_any_agent_with_a_spawn_time() {
        let over = AgentClock { spawn_ms: Some(NOW - 2 * HOUR), ..AgentClock::default() };
        assert_eq!(verdict(limits(2.0, 0.0), NOW, &over), Some(StopReason::MaxRuntime { elapsed_ms: 2 * HOUR }));
        let under = AgentClock { spawn_ms: Some(NOW - 2 * HOUR + 1), ..AgentClock::default() };
        assert_eq!(verdict(limits(2.0, 0.0), NOW, &under), None);
        assert_eq!(verdict(limits(2.0, 0.0), NOW, &AgentClock::default()), None, "no spawn time, no runtime");
    }

    #[test]
    fn a_silent_turn_in_flight_is_idle() {
        let agent = mid_turn(HOUR, 30 * MIN);
        assert_eq!(verdict(limits(0.0, 30.0), NOW, &agent), Some(StopReason::IdleOutput { idle_ms: 30 * MIN }));
        assert_eq!(verdict(limits(0.0, 31.0), NOW, &agent), None);
    }

    #[test]
    fn an_agent_between_turns_waits_for_the_user_and_is_never_idle() {
        let agent = AgentClock { turn_active: false, pass_started_ms: None, ..mid_turn(10 * HOUR, 10 * HOUR) };
        assert_eq!(verdict(limits(0.0, 1.0), NOW, &agent), None);
    }

    #[test]
    fn a_question_waiting_for_the_user_is_not_idle() {
        let agent = AgentClock { awaiting_user: true, ..mid_turn(HOUR, HOUR) };
        assert_eq!(verdict(limits(0.0, 1.0), NOW, &agent), None);
    }

    #[test]
    fn a_new_turn_after_a_long_wait_starts_its_idle_clock_at_the_turn() {
        // Last output was the previous turn, an hour ago; this pass began 2 min ago.
        let agent = AgentClock { pass_started_ms: Some(NOW - 2 * MIN), ..mid_turn(5 * HOUR, HOUR) };
        assert_eq!(verdict(limits(0.0, 30.0), NOW, &agent), None);
        assert_eq!(verdict(limits(0.0, 2.0), NOW, &agent), Some(StopReason::IdleOutput { idle_ms: 2 * MIN }));
    }

    #[test]
    fn a_turn_with_no_output_yet_counts_from_its_start() {
        let agent = AgentClock { last_output_ms: None, pass_started_ms: Some(NOW - 45 * MIN), ..mid_turn(HOUR, 0) };
        assert_eq!(verdict(limits(0.0, 30.0), NOW, &agent), Some(StopReason::IdleOutput { idle_ms: 45 * MIN }));
    }

    #[test]
    fn a_shell_pane_is_idle_whenever_its_pty_is_quiet() {
        let shell = AgentClock { is_shell: true, last_output_ms: Some(NOW - 30 * MIN), ..AgentClock::default() };
        assert_eq!(verdict(limits(0.0, 30.0), NOW, &shell), Some(StopReason::IdleOutput { idle_ms: 30 * MIN }));
        let never_wrote = AgentClock { is_shell: true, ..AgentClock::default() };
        assert_eq!(verdict(limits(0.0, 1.0), NOW, &never_wrote), None);
    }

    #[test]
    fn max_runtime_wins_over_idle() {
        let agent = mid_turn(3 * HOUR, 3 * HOUR);
        assert!(matches!(verdict(limits(1.0, 1.0), NOW, &agent), Some(StopReason::MaxRuntime { .. })));
    }
}
