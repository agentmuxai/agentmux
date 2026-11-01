// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! When an agent's usage limit lifts: `AgentFailure::resets_at_ms` and
//! `limit_window`, filled in when a `UsageLimit` failure is stored
//! (`docs/specs/SPEC_UNRESPONSIVE_AGENTS_2026_10_23.md` §3.4).
//!
//! The sources, in order:
//! 1. Claude's `rate_limit_event`, whose `status` is exactly `"rejected"`:
//!    Claude Code sends one with every request, and a rejected one comes just
//!    before the limit error, carrying the exhausted window's `resetsAt`
//!    (unix seconds) and `rateLimitType`. The persistent stdout reader hands
//!    every frame to [`observe_frame`], which keeps the latest per block.
//!    `overageStatus` is often `"rejected"` while `status` is `"allowed"`; it
//!    is never read.
//! 2. The error text, `resets Oct 26, 7pm (America/Los_Angeles)` or
//!    `resets 7pm`, parsed only in this machine's own time zone: srv has no
//!    time-zone database, so a time in any other zone is left unknown. Codex
//!    words it `... or try again at 9:44 PM.` (the machine's own zone, with the
//!    date and year when it is not today: `Nov 3rd, 2025 9:44 PM`);
//!    [`codex_failure_text`] picks that text out of Codex's `error` and
//!    `turn.failed` frames.
//!
//! A reset outside `(now, now + 8 days]` is dropped: an unknown reset still
//! leaves the agent limited, just with no time shown.

use std::collections::HashMap;
use std::sync::LazyLock;

use chrono::{DateTime, Datelike, Local, NaiveDate, NaiveTime, TimeZone};
use serde_json::Value;

use super::failure::{AgentFailure, FailureClass};

/// The longest a reset may lie ahead: a weekly limit plus a day.
const MAX_AHEAD_MS: u64 = 8 * 24 * 60 * 60 * 1000;

/// A `rate_limit_event`'s `rate_limit_info`, the fields this reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RateLimitInfo {
    pub status: String,
    pub resets_at_s: Option<u64>,
    pub window: Option<String>,
}

impl RateLimitInfo {
    /// The info a `rate_limit_event` frame carries, or `None` for any other frame.
    pub fn of_frame(frame: &Value) -> Option<Self> {
        if frame.get("type").and_then(Value::as_str) != Some("rate_limit_event") {
            return None;
        }
        let info = frame.get("rate_limit_info")?;
        Some(Self {
            status: info.get("status").and_then(Value::as_str).unwrap_or_default().to_string(),
            resets_at_s: info.get("resetsAt").and_then(Value::as_u64),
            window: info.get("rateLimitType").and_then(Value::as_str).map(str::to_string),
        })
    }
}

static LATEST: LazyLock<parking_lot::Mutex<HashMap<String, RateLimitInfo>>> = LazyLock::new(Default::default);

/// Keep `frame` as `block_id`'s latest rate-limit info if it is a
/// `rate_limit_event`; any other frame is ignored.
pub fn observe_frame(block_id: &str, frame: &Value) {
    if let Some(info) = RateLimitInfo::of_frame(frame) {
        LATEST.lock().insert(block_id.to_string(), info);
    }
}

/// Forget `block_id`'s rate-limit info: its pane closed
/// (`blockcontroller::release_block_processes`).
pub fn forget(block_id: &str) {
    LATEST.lock().remove(block_id);
}

/// `failure` with its reset filled in from what is known for `block_id`, at
/// this machine's clock and time zone. Unchanged unless it is a `UsageLimit`
/// with no reset yet.
pub fn with_reset(block_id: &str, failure: &AgentFailure) -> AgentFailure {
    if failure.code != FailureClass::UsageLimit || failure.resets_at_ms.is_some() {
        return failure.clone();
    }
    let latest = LATEST.lock().get(block_id).cloned();
    let zone = iana_time_zone::get_timezone().ok();
    fill_reset(failure, latest.as_ref(), Local::now(), zone.as_deref())
}

/// [`with_reset`]'s rule, with its inputs passed in.
pub fn fill_reset(
    failure: &AgentFailure,
    latest: Option<&RateLimitInfo>,
    now: DateTime<Local>,
    local_zone: Option<&str>,
) -> AgentFailure {
    let mut out = failure.clone();
    if failure.code != FailureClass::UsageLimit || failure.resets_at_ms.is_some() {
        return out;
    }
    let now_ms = u64::try_from(now.timestamp_millis()).unwrap_or(0);
    let in_range = |ms: u64| ms > now_ms && ms <= now_ms.saturating_add(MAX_AHEAD_MS);
    if let Some(info) = latest.filter(|i| i.status == "rejected") {
        if let Some(ms) = info.resets_at_s.map(|s| s.saturating_mul(1000)).filter(|ms| in_range(*ms)) {
            out.resets_at_ms = Some(ms);
            out.limit_window = info.window.clone();
            return out;
        }
    }
    let text = format!("{}\n{}", failure.stderr_tail, failure.detail);
    out.resets_at_ms = reset_from_text(&text, now, local_zone).filter(|ms| in_range(*ms));
    out
}

/// The reset a limit message names (`resets Oct 26, 7pm (America/Los_Angeles)`,
/// `resets 7:30pm`), in unix ms, or `None`. A named zone must be `local_zone`,
/// this machine's own; with none named the time is local (the CLI writes it in
/// the machine's zone). A time with no date is its next occurrence after `now`;
/// a date with no year, likewise.
pub fn reset_from_text(text: &str, now: DateTime<Local>, local_zone: Option<&str>) -> Option<u64> {
    claude_reset(text, now, local_zone).or_else(|| codex_reset(text, now))
}

/// The message of a Codex `error` or `turn.failed` frame, or `None` for any
/// other frame and for the `Reconnecting...` retry notices, which are not
/// failures.
pub fn codex_failure_text(frame: &Value) -> Option<String> {
    let message = match frame.get("type").and_then(Value::as_str)? {
        "turn.failed" => frame.pointer("/error/message").and_then(Value::as_str),
        "error" => frame.get("message").and_then(Value::as_str),
        _ => None,
    }?;
    (!message.contains("Reconnecting...")).then(|| message.to_string())
}

/// What `classify` should read from a frame the stdout reader kept as the run's
/// in-band error: a Claude synthetic assistant message, or a Codex failure.
pub fn inband_error_text(frame: &Value) -> String {
    if let Some(text) = codex_failure_text(frame) {
        return text;
    }
    let error = frame.get("error").and_then(Value::as_str).unwrap_or("");
    let content = frame.pointer("/message/content/0/text").and_then(Value::as_str).unwrap_or("");
    format!("{error} {content}")
}

/// Whether `frame` is the synthetic assistant message Claude Code writes when the login failed
/// (`"error": "authentication_failed"`). The result frame that ends the turn carries only prose
/// ("Failed to authenticate: OAuth session expired and could not be refreshed"), so this is what
/// tells an expired login from any other failure whatever the wording.
pub fn is_login_failure_frame(frame: &Value) -> bool {
    frame.get("type").and_then(Value::as_str) == Some("assistant")
        && frame.get("error").and_then(Value::as_str) == Some("authentication_failed")
}

/// Codex's `try again at 9:44 PM.` / `try again at Nov 3rd, 2025 9:44 PM.`.
/// A time with no date is its next occurrence after `now`: Codex leaves the
/// date out only for today.
fn codex_reset(text: &str, now: DateTime<Local>) -> Option<u64> {
    const MARK: &str = "try again at ";
    let rest = &text[text.find(MARK)? + MARK.len()..];
    let line = rest.lines().next().unwrap_or_default().split('"').next().unwrap_or_default();
    let when = line.trim().trim_end_matches('.');
    let words: Vec<&str> = when.split(|c: char| c == ',' || c.is_whitespace()).filter(|w| !w.is_empty()).collect();
    let clock = |time: &str, meridiem: &str| parse_clock(&format!("{time}{meridiem}"));
    let today = now.date_naive();
    let at = match words.as_slice() {
        [time, meridiem] => {
            let t = Local.from_local_datetime(&today.and_time(clock(time, meridiem)?)).earliest()?;
            if t > now { t } else { Local.from_local_datetime(&today.succ_opt()?.and_time(clock(time, meridiem)?)).earliest()? }
        }
        [month, day, year, time, meridiem] => {
            let digits = day.trim_end_matches(|c: char| c.is_ascii_alphabetic());
            let date = NaiveDate::from_ymd_opt(year.parse().ok()?, month_number(month)?, digits.parse().ok()?)?;
            Local.from_local_datetime(&date.and_time(clock(time, meridiem)?)).earliest()?
        }
        _ => return None,
    };
    u64::try_from(at.timestamp_millis()).ok()
}

fn claude_reset(text: &str, now: DateTime<Local>, local_zone: Option<&str>) -> Option<u64> {
    let start = text.find("resets ")? + "resets ".len();
    let rest = &text[start..];
    let line = rest.lines().next().unwrap_or_default();
    let (when, zone) = match line.find('(') {
        Some(open) => {
            let close = line[open..].find(')').map(|c| open + c)?;
            (line[..open].trim(), Some(line[open + 1..close].trim()))
        }
        None => (line.trim(), None),
    };
    if let Some(zone) = zone {
        if Some(zone) != local_zone {
            return None;
        }
    }
    // "Oct 26, 7pm", "Oct 26 at 7pm" or "7pm"
    let words: Vec<&str> = when.split(|c: char| c == ',' || c.is_whitespace()).filter(|w| !w.is_empty() && *w != "at").collect();
    let (date, time_word) = match words.as_slice() {
        [time] => (None, *time),
        [month, day, time] => (Some((month_number(month)?, day.parse::<u32>().ok()?)), *time),
        _ => return None,
    };
    let time = parse_clock(time_word)?;
    let today = now.date_naive();
    let candidate = |d: NaiveDate| Local.from_local_datetime(&d.and_time(time)).earliest();
    let at = match date {
        None => {
            let t = candidate(today)?;
            if t > now { t } else { candidate(today.succ_opt()?)? }
        }
        Some((month, day)) => {
            let this_year = candidate(NaiveDate::from_ymd_opt(today.year(), month, day)?)?;
            if this_year > now {
                this_year
            } else {
                candidate(NaiveDate::from_ymd_opt(today.year() + 1, month, day)?)?
            }
        }
    };
    u64::try_from(at.timestamp_millis()).ok()
}

fn month_number(word: &str) -> Option<u32> {
    const MONTHS: [&str; 12] = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];
    let lower = word.to_ascii_lowercase();
    let prefix = lower.get(..3)?;
    MONTHS.iter().position(|m| *m == prefix).map(|i| i as u32 + 1)
}

/// `7pm`, `7:30pm`, `12am`, `19:00`.
fn parse_clock(word: &str) -> Option<NaiveTime> {
    let lower = word.to_ascii_lowercase();
    let (digits, meridiem) = if let Some(d) = lower.strip_suffix("am") {
        (d, Some(false))
    } else if let Some(d) = lower.strip_suffix("pm") {
        (d, Some(true))
    } else {
        (lower.as_str(), None)
    };
    let (hour, minute) = match digits.split_once(':') {
        Some((h, m)) => (h.parse::<u32>().ok()?, m.parse::<u32>().ok()?),
        None => (digits.parse::<u32>().ok()?, 0),
    };
    let hour = match meridiem {
        Some(pm) if (1..=12).contains(&hour) => (hour % 12) + if pm { 12 } else { 0 },
        Some(_) => return None,
        None => hour,
    };
    NaiveTime::from_hms_opt(hour, minute, 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::failure::classify;
    use serde_json::json;

    const ZONE: &str = "America/Los_Angeles";

    fn at(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> DateTime<Local> {
        Local.from_local_datetime(&NaiveDate::from_ymd_opt(y, mo, d).unwrap().and_hms_opt(h, mi, 0).unwrap()).earliest().unwrap()
    }

    fn ms(t: DateTime<Local>) -> u64 {
        u64::try_from(t.timestamp_millis()).unwrap()
    }

    fn weekly() -> AgentFailure {
        classify(Some(1), None, "You've hit your weekly limit · resets Oct 26, 7pm (America/Los_Angeles)", None)
    }

    fn rejected(resets: DateTime<Local>, window: &str) -> RateLimitInfo {
        RateLimitInfo { status: "rejected".into(), resets_at_s: Some(ms(resets) / 1000), window: Some(window.into()) }
    }

    #[test]
    fn a_rate_limit_event_is_read_and_other_frames_are_not() {
        let frame = json!({"type": "rate_limit_event", "rate_limit_info": {
            "status": "rejected", "resetsAt": 1792886400, "rateLimitType": "seven_day",
            "overageStatus": "rejected", "unifiedWindows": {"seven_day": {"utilization": 1, "resetsAt": 1792886400}}}});
        assert_eq!(
            RateLimitInfo::of_frame(&frame),
            Some(RateLimitInfo { status: "rejected".into(), resets_at_s: Some(1792886400), window: Some("seven_day".into()) })
        );
        assert_eq!(RateLimitInfo::of_frame(&json!({"type": "result", "is_error": true})), None);
    }

    #[test]
    fn the_latest_frame_per_block_is_kept() {
        let b = format!("b-{}", uuid::Uuid::new_v4());
        observe_frame(&b, &json!({"type": "rate_limit_event", "rate_limit_info": {"status": "allowed", "resetsAt": 1}}));
        observe_frame(&b, &json!({"type": "assistant"}));
        observe_frame(&b, &json!({"type": "rate_limit_event", "rate_limit_info": {"status": "rejected", "resetsAt": 2}}));
        assert_eq!(LATEST.lock().get(&b).map(|i| (i.status.clone(), i.resets_at_s)), Some(("rejected".into(), Some(2))));
        forget(&b);
        assert!(LATEST.lock().get(&b).is_none());
    }

    const CODEX_LIMIT: &str = "You've hit your usage limit. Upgrade to Pro (https://chatgpt.com/explore/pro), visit https://chatgpt.com/codex/settings/usage to purchase more credits or try again at 9:44 PM.";

    #[test]
    fn codex_wording_gives_the_reset() {
        let now = at(2026, 10, 25, 9, 0);
        assert_eq!(reset_from_text(CODEX_LIMIT, now, Some(ZONE)), Some(ms(at(2026, 10, 25, 21, 44))), "today");
        assert_eq!(reset_from_text(CODEX_LIMIT, at(2026, 10, 25, 22, 0), Some(ZONE)), Some(ms(at(2026, 10, 26, 21, 44))), "a time already past is tomorrow's");
        assert_eq!(reset_from_text("or try again at 8:19 AM.", now, None), Some(ms(at(2026, 10, 26, 8, 19))));
        assert_eq!(reset_from_text("or try again at Oct 29th, 2026 9:44 PM.", now, None), Some(ms(at(2026, 10, 29, 21, 44))), "a date and year");
        assert_eq!(reset_from_text("or try again at Nov 3rd, 2026 12:05 AM.", now, None), Some(ms(at(2026, 11, 3, 0, 5))));
        assert_eq!(reset_from_text("or try again later.", now, None), None, "no time named");
        assert_eq!(reset_from_text("or try again at soon.", now, None), None);
    }

    #[test]
    fn only_the_synthetic_assistant_message_marks_a_failed_login() {
        let failed = json!({"type": "assistant", "error": "authentication_failed", "is_api_error_message": true,
            "message": {"content": [{"type": "text", "text": "Failed to authenticate: OAuth session expired and could not be refreshed"}]}});
        assert!(is_login_failure_frame(&failed));
        let limited = json!({"type": "assistant", "error": "rate_limit", "message": {"content": [{"type": "text", "text": "You've hit your limit"}]}});
        let result = json!({"type": "result", "is_error": true, "error": "authentication_failed"});
        let said_so = json!({"type": "assistant", "message": {"content": [{"type": "text", "text": "authentication_failed"}]}});
        for other in [limited, result, said_so] {
            assert!(!is_login_failure_frame(&other), "{other}");
        }
    }

    #[test]
    fn codex_frames_carry_the_failure_text_and_retry_notices_do_not() {
        let failed = json!({"type": "turn.failed", "error": {"message": CODEX_LIMIT}});
        let error = json!({"type": "error", "message": CODEX_LIMIT});
        assert_eq!(codex_failure_text(&failed).as_deref(), Some(CODEX_LIMIT));
        assert_eq!(codex_failure_text(&error).as_deref(), Some(CODEX_LIMIT));
        assert_eq!(codex_failure_text(&json!({"type": "error", "message": "Reconnecting... 2/5"})), None);
        assert_eq!(codex_failure_text(&json!({"type": "turn.completed"})), None);
        assert_eq!(inband_error_text(&failed), CODEX_LIMIT);
        assert_eq!(inband_error_text(&json!({"error": "authentication_failed", "message": {"content": [{"text": "401"}]}})), "authentication_failed 401");
    }

    #[test]
    fn a_codex_limit_failure_is_a_usage_limit_with_its_reset() {
        let now = at(2026, 10, 25, 9, 0);
        let f = classify(Some(1), None, CODEX_LIMIT, None);
        assert_eq!(f.code, FailureClass::UsageLimit);
        assert_eq!(fill_reset(&f, None, now, Some(ZONE)).resets_at_ms, Some(ms(at(2026, 10, 25, 21, 44))));
    }

    #[test]
    fn a_codex_reset_outside_the_next_eight_days_is_dropped() {
        let now = at(2026, 10, 25, 9, 0);
        let far = classify(Some(1), None, "You've hit your usage limit. try again at Nov 20th, 2026 9:44 PM.", None);
        assert_eq!(fill_reset(&far, None, now, Some(ZONE)).resets_at_ms, None, "more than 8 days ahead");
        let past = classify(Some(1), None, "You've hit your usage limit. try again at Oct 20th, 2026 9:44 PM.", None);
        assert_eq!(fill_reset(&past, None, now, Some(ZONE)).resets_at_ms, None, "already past");
        let week = classify(Some(1), None, "You've hit your usage limit. try again at Nov 1st, 2026 9:44 PM.", None);
        assert_eq!(fill_reset(&week, None, now, Some(ZONE)).resets_at_ms, Some(ms(at(2026, 11, 1, 21, 44))), "a week out is kept");
    }

    #[test]
    fn a_rejected_event_gives_the_reset_and_the_window() {
        let now = at(2026, 10, 23, 9, 0);
        let reset = at(2026, 10, 26, 19, 0);
        let f = fill_reset(&weekly(), Some(&rejected(reset, "seven_day")), now, Some(ZONE));
        assert_eq!(f.resets_at_ms, Some(ms(reset)));
        assert_eq!(f.limit_window.as_deref(), Some("seven_day"));
    }

    #[test]
    fn an_allowed_event_is_not_a_reset_even_with_overage_rejected() {
        let now = at(2026, 10, 23, 9, 0);
        let allowed = RateLimitInfo { status: "allowed".into(), ..rejected(at(2026, 10, 23, 14, 0), "five_hour") };
        let f = fill_reset(&weekly(), Some(&allowed), now, Some("Europe/Paris"));
        assert_eq!(f.resets_at_ms, None, "no rejected event, and the text names another zone");
        assert_eq!(f.limit_window, None);
    }

    #[test]
    fn the_text_is_the_fallback_in_the_local_zone_only() {
        let now = at(2026, 10, 23, 9, 0);
        assert_eq!(fill_reset(&weekly(), None, now, Some(ZONE)).resets_at_ms, Some(ms(at(2026, 10, 26, 19, 0))));
        assert_eq!(fill_reset(&weekly(), None, now, Some("Europe/Paris")).resets_at_ms, None);
        assert_eq!(fill_reset(&weekly(), None, now, None).resets_at_ms, None, "local zone unknown");
    }

    #[test]
    fn text_without_a_date_or_year_is_the_next_occurrence() {
        let now = at(2026, 10, 23, 16, 0);
        assert_eq!(reset_from_text("resets 3pm", now, None), Some(ms(at(2026, 10, 24, 15, 0))), "3pm has passed today");
        assert_eq!(reset_from_text("resets 7:30pm (America/Los_Angeles)", now, Some(ZONE)), Some(ms(at(2026, 10, 23, 19, 30))));
        assert_eq!(reset_from_text("resets Jan 2, 9am", now, None), Some(ms(at(2027, 1, 2, 9, 0))), "next January");
        assert_eq!(reset_from_text("resets Oct 23 at 12am", now, None), Some(ms(at(2027, 10, 23, 0, 0))));
        assert_eq!(reset_from_text("no reset here", now, None), None);
        assert_eq!(reset_from_text("resets soon", now, None), None);
        assert_eq!(reset_from_text("resets 13pm", now, None), None);
    }

    #[test]
    fn a_reset_out_of_range_is_dropped() {
        let now = at(2026, 10, 23, 9, 0);
        let past = fill_reset(&weekly(), Some(&rejected(at(2026, 10, 22, 9, 0), "seven_day")), now, None);
        assert_eq!(past.resets_at_ms, None);
        let far = fill_reset(&weekly(), Some(&rejected(at(2026, 11, 5, 9, 0), "seven_day")), now, None);
        assert_eq!(far.resets_at_ms, None, "more than 8 days ahead");
        // Exactly 8 days of milliseconds, whatever DST does that week.
        let edge = Local.timestamp_millis_opt((ms(now) + MAX_AHEAD_MS) as i64).unwrap();
        assert_eq!(fill_reset(&weekly(), Some(&rejected(edge, "seven_day")), now, None).resets_at_ms, Some(ms(edge)), "exactly 8 days");
    }

    #[test]
    fn only_a_usage_limit_without_a_reset_is_filled() {
        let now = at(2026, 10, 23, 9, 0);
        let reset = rejected(at(2026, 10, 23, 14, 0), "five_hour");
        let auth = classify(Some(1), None, "authentication_error: Invalid API key provided", None);
        assert_eq!(fill_reset(&auth, Some(&reset), now, Some(ZONE)), auth);
        let known = AgentFailure { resets_at_ms: Some(42), ..weekly() };
        assert_eq!(fill_reset(&known, Some(&reset), now, Some(ZONE)).resets_at_ms, Some(42));
    }

    #[test]
    fn the_server_rate_limit_is_not_a_usage_limit_and_gets_no_reset() {
        let now = at(2026, 10, 23, 9, 0);
        let f = classify(Some(1), None, "API Error: Server is temporarily limiting requests (not your usage limit) · Rate limited", None);
        assert_eq!(f.code, FailureClass::RateLimited);
        assert_eq!(fill_reset(&f, Some(&rejected(at(2026, 10, 23, 10, 0), "five_hour")), now, None).resets_at_ms, None);
    }

    #[test]
    fn old_meta_without_the_new_fields_still_reads_and_none_is_not_written() {
        // As an older build stored it (with its stderr tail; the state code
        // reads only code, title and resetsAtMs, which works either way).
        let old = json!({"code": "usage_limit", "title": "Usage limit reached", "detail": "", "exitCode": 1,
                         "stderrTail": "You've hit your weekly limit", "retryable": false});
        let f: AgentFailure = serde_json::from_value(old).unwrap();
        assert_eq!((f.resets_at_ms, f.limit_window.clone()), (None, None));
        let v = serde_json::to_value(&f).unwrap();
        assert!(v.get("resetsAtMs").is_none() && v.get("limitWindow").is_none());
        let with = AgentFailure { resets_at_ms: Some(5), limit_window: Some("seven_day".into()), ..f };
        let v = serde_json::to_value(&with).unwrap();
        assert_eq!((v["resetsAtMs"].clone(), v["limitWindow"].clone()), (json!(5), json!("seven_day")));
    }
}
