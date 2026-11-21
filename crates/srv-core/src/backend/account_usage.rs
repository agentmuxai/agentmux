// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! How much of an account's usage windows is used, from the report Claude Code sends with every
//! request.
//!
//! Every request's `rate_limit_event` frame carries
//! `rate_limit_info.unifiedWindows.<window>.{utilization, resetsAt}` for each plan window
//! (`five_hour`, `seven_day`, ...), whatever its `status` (CLI 2.1.301: `utilization` is a fraction
//! from 0 to 1, `resetsAt` is unix seconds). Every agent on one account reports the same windows, so
//! the readings are kept per **account**, not per pane: the newest frame from any pane wins, a pane
//! closing loses nothing, and a window's reading is dropped once its reset has passed.
//!
//! A reading that crosses a notice threshold (`agent:usagenoticeat`, `agent:usagenoticehighat`; a
//! percent of the window, 0 turns one off) gives the leaders of the account's agents one
//! `[AgentMux]` note, once per window per threshold. The note is held for the leader's next turn or
//! joins the turn it is in: it never starts one.
//!
//! Providers other than Claude report nothing here; [`UsageStatus::NotReported`] says so.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, LazyLock, OnceLock};

use serde::Serialize;
use serde_json::Value;

use crate::backend::agent_state::RUN_ACCOUNT_META_KEY;
use crate::backend::obj::{self, Block};
use crate::backend::storage::store::Store;

/// `agent:usagenoticeat`: the percent of a usage window from which the account's leaders get a note.
pub const SETTING_NOTICE_AT: &str = "agent:usagenoticeat";
pub const DEFAULT_NOTICE_AT: u64 = 75; // plain digits: settings-defaults.test.ts reads it
/// `agent:usagenoticehighat`: the percent from which they get a second, stronger one.
pub const SETTING_NOTICE_HIGH_AT: &str = "agent:usagenoticehighat";
pub const DEFAULT_NOTICE_HIGH_AT: u64 = 90; // plain digits, as above

/// One window's latest reading for an account.
#[derive(Debug, Clone, PartialEq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct UsageWindow {
    /// How much of the window is used, 0 to 100 (it may pass 100 when the limit is hit).
    pub used_percent: f64,
    /// When the window resets (unix ms).
    #[ts(type = "number")]
    pub resets_at_ms: u64,
    /// When this reading was taken (unix ms).
    #[ts(type = "number")]
    pub observed_at_ms: u64,
    /// How old the reading is now.
    #[ts(type = "number")]
    pub age_ms: u64,
}

/// Whether an account's usage is known.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
#[serde(rename_all = "snake_case")]
pub enum UsageStatus {
    /// At least one window has a reading that has not reset.
    Reported,
    /// A provider that reports usage, with no reading yet (nothing has run on the account since
    /// srv started, or its windows have reset).
    NotYet,
    /// A provider whose CLI reports nothing srv reads.
    NotReported,
}

/// An account's usage as an agent or a window reads it.
#[derive(Debug, Clone, PartialEq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct AccountUsage {
    pub status: UsageStatus,
    /// By the provider's name for the window (`five_hour`, `seven_day`, ...).
    pub windows: BTreeMap<String, UsageWindow>,
}

impl AccountUsage {
    /// What `provider`'s account at `now_ms` reads.
    pub fn of(provider: &str, account_id: &str, now_ms: u64) -> Self {
        if !reports_usage(provider) {
            return Self { status: UsageStatus::NotReported, windows: BTreeMap::new() };
        }
        let windows = of_account(account_id, now_ms);
        let status = if windows.is_empty() { UsageStatus::NotYet } else { UsageStatus::Reported };
        Self { status, windows }
    }
}

/// Whether the provider's CLI reports usage windows that srv reads: Claude's `rate_limit_event`.
/// Codex and Gemini report nothing srv reads, so their accounts say `not_reported`.
pub fn reports_usage(provider: &str) -> bool {
    provider.eq_ignore_ascii_case("claude") || provider.eq_ignore_ascii_case("anthropic")
}

struct Entry {
    reading: UsageWindow,
    /// The thresholds already told for this window's current reset (bit 0: the first, bit 1: the high one).
    told: u8,
}

static BY_ACCOUNT: LazyLock<parking_lot::Mutex<HashMap<String, HashMap<String, Entry>>>> = LazyLock::new(Default::default);
static CONFIG: OnceLock<Arc<crate::backend::wconfig::ConfigState>> = OnceLock::new();

/// Attach the live settings (bootstrap), so the notice thresholds can be read.
pub fn install_config(config: Arc<crate::backend::wconfig::ConfigState>) {
    let _ = CONFIG.set(config);
}

/// The two notice thresholds, in percent; 0 is off.
fn thresholds() -> [u64; 2] {
    let read = |key: &str, default: u64| CONFIG.get().and_then(|c| c.extra_u64(key)).unwrap_or(default);
    [read(SETTING_NOTICE_AT, DEFAULT_NOTICE_AT), read(SETTING_NOTICE_HIGH_AT, DEFAULT_NOTICE_HIGH_AT)]
}

/// The windows a `rate_limit_event` frame reports: name, utilization (0 to 1) and reset (unix
/// seconds). Empty for any other frame, and for a window with no utilization.
pub fn windows_of_frame(frame: &Value) -> Vec<(String, f64, u64)> {
    if frame.get("type").and_then(Value::as_str) != Some("rate_limit_event") {
        return Vec::new();
    }
    let Some(windows) = frame.pointer("/rate_limit_info/unifiedWindows").and_then(Value::as_object) else {
        return Vec::new();
    };
    windows
        .iter()
        .filter_map(|(name, w)| {
            let utilization = w.get("utilization").and_then(Value::as_f64).filter(|u| u.is_finite() && *u >= 0.0)?;
            Some((name.clone(), utilization, w.get("resetsAt").and_then(Value::as_u64)?))
        })
        .collect()
}

/// A threshold a reading has newly crossed.
#[derive(Debug, Clone, PartialEq)]
pub struct Crossing {
    pub window: String,
    /// The threshold, in percent.
    pub threshold: u64,
    pub used_percent: f64,
    pub resets_at_ms: u64,
}

/// Keep `windows` (from [`windows_of_frame`]) as `account`'s latest at `now_ms`, and say which
/// thresholds they newly cross. A reading for a window that resets earlier than the one kept is a
/// frame from the window before: ignored. A later reset is a new window, and tells again.
pub fn observe(account: &str, windows: &[(String, f64, u64)], now_ms: u64) -> Vec<Crossing> {
    let limits = thresholds();
    let mut by_account = BY_ACCOUNT.lock();
    let kept = by_account.entry(account.to_string()).or_default();
    let mut out = Vec::new();
    for (name, utilization, resets_at_s) in windows {
        let resets_at_ms = resets_at_s.saturating_mul(1000);
        let used_percent = utilization * 100.0;
        let told = match kept.get(name) {
            Some(old) if old.reading.resets_at_ms > resets_at_ms => continue,
            Some(old) if old.reading.resets_at_ms == resets_at_ms => old.told,
            _ => 0,
        };
        let mut told = told;
        // The highest threshold crossed and not yet told; a jump over both says only the high one.
        let mut crossed = None;
        for (bit, limit) in limits.iter().enumerate() {
            if *limit > 0 && used_percent >= *limit as f64 && told & (1 << bit) == 0 {
                told |= 1 << bit;
                crossed = Some(*limit);
            }
        }
        if let Some(threshold) = crossed {
            out.push(Crossing { window: name.clone(), threshold, used_percent, resets_at_ms });
        }
        let reading = UsageWindow { used_percent, resets_at_ms, observed_at_ms: now_ms, age_ms: 0 };
        kept.insert(name.clone(), Entry { reading, told });
    }
    out
}

/// `account`'s windows that have not reset at `now_ms`, with their age.
pub fn of_account(account: &str, now_ms: u64) -> BTreeMap<String, UsageWindow> {
    let mut by_account = BY_ACCOUNT.lock();
    let Some(kept) = by_account.get_mut(account) else { return BTreeMap::new() };
    kept.retain(|_, e| e.reading.resets_at_ms > now_ms);
    let out = kept
        .iter()
        .map(|(name, e)| (name.clone(), UsageWindow { age_ms: now_ms.saturating_sub(e.reading.observed_at_ms), ..e.reading.clone() }))
        .collect();
    if kept.is_empty() {
        by_account.remove(account);
    }
    out
}

/// The account a pane's run launched on.
fn account_of(block: &Block) -> Option<String> {
    block.meta.get(RUN_ACCOUNT_META_KEY).and_then(|a| a.as_str()).filter(|a| !a.is_empty()).map(str::to_string)
}

/// Any frame of `block_id`'s CLI: a `rate_limit_event` with usage is kept for the pane's account, and
/// its leaders told when it crosses a threshold. The persistent stdout reader calls it for every
/// frame; anything else returns at once.
pub fn observe_claude_frame(block_id: &str, store: &Arc<Store>, frame: &Value) {
    let windows = windows_of_frame(frame);
    if windows.is_empty() {
        return;
    }
    let Some(account) = store.get::<Block>(block_id).ok().flatten().as_ref().and_then(account_of) else { return };
    let crossings = observe(&account, &windows, agentmux_common::time::now_ms_u64());
    if crossings.is_empty() {
        return;
    }
    let store = Arc::clone(store);
    tokio::task::spawn_blocking(move || {
        for crossing in &crossings {
            notify_leaders(&store, &account, crossing);
        }
    });
}

/// The panes that hear of a crossing: for each live agent pane on `account`, its lead's first open
/// pane on this machine, or the pane itself when it has none (it reports to the person). Each once.
pub(crate) fn note_targets(store: &Store, account: &str) -> Vec<String> {
    let blocks = store.get_all::<Block>().unwrap_or_default();
    let mut targets: Vec<String> = Vec::new();
    for block in blocks.iter().filter(|b| account_of(b).as_deref() == Some(account)) {
        let def_id = obj::meta_get_string(&block.meta, "agentId", "");
        if def_id.is_empty() || super::blockcontroller::get_controller(&block.oid).is_none() {
            continue;
        }
        let target = super::worker_notes::leader_pane_of(store, &def_id).unwrap_or_else(|| block.oid.clone());
        if !targets.contains(&target) {
            targets.push(target);
        }
    }
    targets
}

/// "Sat 3:10 PM", in this machine's time zone.
fn local_label(ms: u64) -> String {
    chrono::DateTime::from_timestamp_millis(ms as i64)
        .map(|t| t.with_timezone(&chrono::Local).format("%a %-I:%M %p").to_string())
        .unwrap_or_default()
}

/// What a window is called to a reader.
fn window_words(window: &str) -> String {
    match window {
        "five_hour" => "5-hour".to_string(),
        "seven_day" => "weekly".to_string(),
        other => other.replace('_', " "),
    }
}

/// The note's body (the `[AgentMux]` prefix is added by the sender).
pub(crate) fn note_body(crossing: &Crossing) -> String {
    let used = crossing.used_percent.floor();
    let advice = if crossing.used_percent >= 90.0 {
        "At 100% every agent on it stops until the reset: finish what is running and hold new work that can wait."
    } else {
        "Slow work that can wait; at 100% every agent on it stops until the reset."
    };
    format!(
        "An account that some of your agents run on has used {used}% of its {} usage window (it resets {}). {advice} \
         Read it any time with Identity accounts.",
        window_words(&crossing.window),
        local_label(crossing.resets_at_ms),
    )
}

fn notify_leaders(store: &Store, account: &str, crossing: &Crossing) {
    let targets = note_targets(store, account);
    let body = note_body(crossing);
    let text = format!("[AgentMux] {}", crate::backend::reactive::sanitize::quote_in_system_note(&body));
    // One key per window: the high note replaces the first one if it has not been read yet.
    let key = format!("usage-{}", crossing.window);
    for block in &targets {
        super::blockcontroller::system_note_into_turn_or_hold(block, &key, &text);
    }
}

#[cfg(test)]
#[path = "account_usage_tests.rs"]
mod tests;
