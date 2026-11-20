// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The host's side of a call from the page (`/ipc`): how long the call waited
// before the host began it, and how long the host took. The page logs its own
// total (`[perf] ipc`); with these two figures in the answer it can say where a
// slow call spent its time. The host also logs a slow call itself, because the
// page's own lines are the first to be dropped when the page is the slow part.

use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Instant;

use agentmux_common::time::now_ms_u64;
use parking_lot::Mutex;

/// A wait or a handling time from here on is logged.
const SLOW_MS: u64 = 100;
/// A send time further back than this is not a wait (a wrong clock, a made-up
/// value): the call is answered with no wait figure.
const LONGEST_WAIT_MS: u64 = 10 * 60 * 1000;
/// At most this many lines per `LOG_WINDOW_MS`; the next line logged says how
/// many were left out.
const LOG_MAX_PER_WINDOW: u32 = 10;
const LOG_WINDOW_MS: u64 = 10_000;

/// Calls being handled now, from every window.
static IN_FLIGHT: AtomicU32 = AtomicU32::new(0);
static LOG_GATE: Mutex<LogGate> = Mutex::new(LogGate::new());

/// What the answer carries back to the page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct IpcTiming {
    /// From the page's send to the host beginning the call. Left out when the
    /// page sent no usable time.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub waited_ms: Option<u64>,
    /// The host's own time on the call.
    pub handled_ms: u64,
}

/// One call's clock: begun when the host starts on the call, read when it has
/// the answer. Dropping it (the answer sent, or the request abandoned) takes the
/// call out of the in-flight count.
pub struct CallClock {
    began: Instant,
    waited_ms: Option<u64>,
    /// Calls already being handled when this one began. The page keeps at most
    /// six connections to this server, so a long wait beside five others or more
    /// points at the page's own queue.
    others: u32,
}

impl CallClock {
    /// `sent_ms` is the page's `Date.now()` when it sent the call.
    pub fn begin(sent_ms: Option<f64>) -> Self {
        let others = IN_FLIGHT.fetch_add(1, Ordering::Relaxed);
        Self { began: Instant::now(), waited_ms: waited_ms(sent_ms, now_ms_u64()), others }
    }

    /// The call's timing; a slow one is logged, by command name only.
    pub fn finish(&self, cmd: &str) -> IpcTiming {
        let timing = IpcTiming { waited_ms: self.waited_ms, handled_ms: self.began.elapsed().as_millis() as u64 };
        if is_slow(cmd, timing) {
            if let Some(left_out) = LOG_GATE.lock().admit(now_ms_u64()) {
                tracing::info!("{}", slow_line(cmd, timing, self.others, left_out));
            }
        }
        timing
    }
}

impl Drop for CallClock {
    fn drop(&mut self) {
        IN_FLIGHT.fetch_sub(1, Ordering::Relaxed);
    }
}

/// The wait between the page's send and now. None for no send time, one that is
/// not a time, or one too far back to be a wait; a send time slightly ahead of
/// the host's clock is a wait of zero.
fn waited_ms(sent_ms: Option<f64>, now_ms: u64) -> Option<u64> {
    let sent = sent_ms.filter(|s| s.is_finite() && *s > 0.0)?;
    let waited = now_ms.saturating_sub(sent as u64);
    (waited <= LONGEST_WAIT_MS).then_some(waited)
}

/// The log pipe's own calls are the most frequent ones in a storm: logging them
/// would use up the lines the other commands need.
fn is_log_pipe(cmd: &str) -> bool {
    matches!(cmd, "fe_log" | "fe_log_structured" | "fe_log_batch")
}

/// Commands whose answer waits for a person (a file dialog, a sign-in, an OS
/// confirmation): a long handling time is not a slow host.
fn waits_for_a_person(cmd: &str) -> bool {
    matches!(
        cmd,
        "show_open_file_dialog"
            | "show_open_bundle_dialog"
            | "show_save_layout_dialog"
            | "show_open_layout_dialog"
            | "vault_reveal"
            | "vault_copy"
            | "vault_export"
            | "vault_import"
            | "run_cli_login"
    )
}

/// A wait is never expected, so a long one is always slow; a long handling time
/// is slow unless the command waits for a person.
fn is_slow(cmd: &str, timing: IpcTiming) -> bool {
    if is_log_pipe(cmd) {
        return false;
    }
    timing.waited_ms.is_some_and(|w| w >= SLOW_MS) || (timing.handled_ms >= SLOW_MS && !waits_for_a_person(cmd))
}

fn slow_line(cmd: &str, timing: IpcTiming, others: u32, left_out: u32) -> String {
    let waited = timing.waited_ms.map_or_else(|| "unknown".to_string(), |w| w.to_string());
    let mut line = format!("[ipc] slow call cmd={cmd} waited_ms={waited} handled_ms={} in_flight={others}", timing.handled_ms);
    if left_out > 0 {
        line.push_str(&format!(" not_logged={left_out}"));
    }
    line
}

/// Lets through `LOG_MAX_PER_WINDOW` lines per `LOG_WINDOW_MS` and counts the
/// rest, so a storm of slow calls costs the log ten lines, not thousands.
struct LogGate {
    window_began_ms: u64,
    logged: u32,
    left_out: u32,
}

impl LogGate {
    const fn new() -> Self {
        Self { window_began_ms: 0, logged: 0, left_out: 0 }
    }

    /// Some(lines left out since the last one logged) when this line may be
    /// logged, None when it is left out.
    fn admit(&mut self, now_ms: u64) -> Option<u32> {
        if now_ms.saturating_sub(self.window_began_ms) >= LOG_WINDOW_MS {
            self.window_began_ms = now_ms;
            self.logged = 0;
        }
        if self.logged >= LOG_MAX_PER_WINDOW {
            self.left_out += 1;
            return None;
        }
        self.logged += 1;
        Some(std::mem::take(&mut self.left_out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn timing(waited_ms: Option<u64>, handled_ms: u64) -> IpcTiming {
        IpcTiming { waited_ms, handled_ms }
    }

    #[test]
    fn the_wait_is_the_time_from_the_pages_send_to_now() {
        assert_eq!(waited_ms(Some(1_000_000.0), 1_001_390), Some(1_390));
        assert_eq!(waited_ms(Some(1_000_000.4), 1_000_000), Some(0));
    }

    #[test]
    fn a_send_time_just_ahead_of_the_hosts_clock_is_no_wait() {
        assert_eq!(waited_ms(Some(1_000_003.0), 1_000_000), Some(0));
    }

    #[test]
    fn no_wait_is_reported_without_a_usable_send_time() {
        assert_eq!(waited_ms(None, 1_000_000), None);
        assert_eq!(waited_ms(Some(f64::NAN), 1_000_000), None);
        assert_eq!(waited_ms(Some(f64::INFINITY), 1_000_000), None);
        assert_eq!(waited_ms(Some(-5.0), 1_000_000), None);
        assert_eq!(waited_ms(Some(0.0), 1_000_000), None);
        // An hour back is a wrong clock or a made-up value, not a wait.
        assert_eq!(waited_ms(Some(1_000_000.0), 1_000_000 + 60 * 60 * 1000), None);
    }

    #[test]
    fn a_long_wait_is_slow_for_any_command_but_the_log_pipes_own() {
        assert!(is_slow("set_taskbar_attention", timing(Some(1_390), 1)));
        assert!(is_slow("show_open_file_dialog", timing(Some(100), 0)));
        assert!(!is_slow("set_taskbar_attention", timing(Some(99), 99)));
        assert!(!is_slow("set_taskbar_attention", timing(None, 5)));
        for cmd in ["fe_log", "fe_log_structured", "fe_log_batch"] {
            assert!(!is_slow(cmd, timing(Some(5_000), 5_000)), "{cmd}");
        }
    }

    #[test]
    fn a_long_handling_time_is_slow_unless_the_command_waits_for_a_person() {
        assert!(is_slow("input_select", timing(Some(0), 989)));
        assert!(is_slow("input_select", timing(None, 100)));
        assert!(!is_slow("show_open_file_dialog", timing(Some(2), 45_000)));
        assert!(!is_slow("vault_reveal", timing(None, 8_000)));
    }

    #[test]
    fn the_line_names_the_command_and_both_times_and_never_the_arguments() {
        assert_eq!(
            slow_line("set_taskbar_attention", timing(Some(1_390), 2), 5, 0),
            "[ipc] slow call cmd=set_taskbar_attention waited_ms=1390 handled_ms=2 in_flight=5"
        );
        assert_eq!(
            slow_line("input_select", timing(None, 989), 0, 37),
            "[ipc] slow call cmd=input_select waited_ms=unknown handled_ms=989 in_flight=0 not_logged=37"
        );
    }

    #[test]
    fn a_storm_costs_ten_lines_per_ten_seconds_and_the_next_line_counts_the_rest() {
        let mut gate = LogGate::new();
        let start = 5_000_000;
        let admitted: Vec<Option<u32>> = (0..25).map(|i| gate.admit(start + i * 10)).collect();
        assert_eq!(admitted.iter().filter(|a| a.is_some()).count(), 10);
        assert!(admitted[..10].iter().all(|a| *a == Some(0)));
        assert!(admitted[10..].iter().all(Option::is_none));
        // Still inside the ten seconds: left out too.
        assert_eq!(gate.admit(start + 9_999), None);
        // The next window's first line says how many were left out, once.
        assert_eq!(gate.admit(start + 10_000), Some(16));
        assert_eq!(gate.admit(start + 10_001), Some(0));
    }

    #[test]
    fn a_call_is_counted_as_in_flight_while_its_clock_is_held() {
        // Other tests in this binary may hold clocks of their own, so the count
        // has a floor here, not an exact value.
        let first = CallClock::begin(None);
        let second = CallClock::begin(None);
        let third = CallClock::begin(None);
        assert!(second.others >= 1);
        assert!(third.others >= 2);
        drop((first, second, third));
    }

    #[test]
    fn the_answer_carries_the_wait_only_when_it_is_known() {
        assert_eq!(serde_json::to_value(timing(Some(3), 7)).unwrap(), serde_json::json!({ "waited_ms": 3, "handled_ms": 7 }));
        assert_eq!(serde_json::to_value(timing(None, 7)).unwrap(), serde_json::json!({ "handled_ms": 7 }));
    }
}
