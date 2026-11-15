// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! srv's own panics, counted and made visible.
//!
//! On 2026-11-15 narko's srv panicked about 30,000 times in two hours. The panics went to the log and nowhere else, `/health`
//! kept answering, and nobody knew until an agent's send failed (docs/incident/RCA_BUILD_SLOT_IDLE_NOTE_POISONS_MESSAGE_HANDLER_2026_11_15.md).
//!
//! - [`install_hook`] chains a panic hook that counts every panic by where it happened. The log stays as it is: the earlier hook
//!   runs first, and the hook itself only counts.
//! - [`start_reporter`] looks at the count every few seconds. Above [`WARN_ABOVE`] panics in the last minute it publishes the
//!   `srv-panics` event (the status bar's backend dot and notices follow it) and, once an hour, holds an `[AgentMux]` note for
//!   every agent. The event is persisted, so a window that opens mid-storm gets the current state.
//! - [`snapshot`] is what `/health` and `app.health` report.
//!
//! The decisions are pure ([`Counts`]), so they are tested without a clock or a panic.

use std::collections::HashMap;
use std::panic::PanicHookInfo;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::Duration;

/// More panics than this in the last minute is a warning.
pub const WARN_ABOVE: u32 = 20;
/// More than this is critical.
pub const CRITICAL_ABOVE: u32 = 200;
/// The key of the held note: a later note replaces an earlier one that its agent has not read yet.
pub const NOTE_KEY: &str = "srv-panics";

const WINDOW_SECS: u64 = 60;
/// Distinct places remembered; more are counted together as [`OTHER`].
const MAX_PLACES: usize = 64;
const OTHER: &str = "(other places)";
/// The same note to the agents at most this often.
const NOTE_EVERY_MS: u64 = 60 * 60 * 1000;
const LOOK_EVERY: Duration = Duration::from_secs(5);
const TOP_PLACES: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Normal,
    Warn,
    Critical,
}

impl Level {
    pub fn as_str(self) -> &'static str {
        match self {
            Level::Normal => "normal",
            Level::Warn => "warn",
            Level::Critical => "critical",
        }
    }

    /// The level for this many panics in the last minute.
    pub fn for_count(last_minute: u32) -> Level {
        if last_minute > CRITICAL_ABOVE {
            Level::Critical
        } else if last_minute > WARN_ABOVE {
            Level::Warn
        } else {
            Level::Normal
        }
    }
}

/// One place that panicked, and how often.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct PanicPlace {
    /// `backend/reactive/handler.rs:1416`: the source file below `src/`, and the line.
    pub location: String,
    #[ts(type = "number")]
    pub count: u64,
}

/// The `srv-panics` event's payload (`mps_events::SrvPanics`), shaped like the host's `health-signal` payloads so the status
/// bar's store takes it unchanged: `kind` is `panics`, and a `normal` level clears the signal.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct SrvPanicsEvent {
    /// Always `panics`.
    #[ts(type = "\"panics\"")]
    pub kind: String,
    /// `normal`, `warn` or `critical`.
    #[ts(type = "\"normal\" | \"warn\" | \"critical\"")]
    pub level: String,
    /// When this episode began (ms since the epoch); absent while normal.
    #[ts(type = "number | null")]
    pub since_ms: Option<u64>,
    /// Panics in the last minute.
    pub last_minute: u32,
    /// Panics since srv started.
    #[ts(type = "number")]
    pub total: u64,
    /// Times the message handler's lock was found poisoned and recovered since srv started.
    #[ts(type = "number")]
    pub recovered_lock: u64,
    /// The places that panicked most, biggest first.
    pub top: Vec<PanicPlace>,
}

#[derive(Debug, Clone, Copy, Default)]
struct Bucket {
    second: u64,
    count: u32,
}

/// What the watch knows: a per-second ring for the last minute, a total, and a count per place.
#[derive(Debug)]
pub struct Counts {
    total: u64,
    ring: [Bucket; WINDOW_SECS as usize],
    places: HashMap<String, u64>,
    level: Level,
    since_ms: Option<u64>,
    noted_at_ms: Option<u64>,
}

/// What a look found: the event to publish, and the note to hold for the agents.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Step {
    pub publish: Option<SrvPanicsEvent>,
    pub note: Option<String>,
}

impl Default for Counts {
    fn default() -> Self {
        Counts { total: 0, ring: [Bucket::default(); WINDOW_SECS as usize], places: HashMap::new(), level: Level::Normal, since_ms: None, noted_at_ms: None }
    }
}

impl Counts {
    /// One panic at `location`, at `now_ms`.
    pub fn record(&mut self, location: &str, now_ms: u64) {
        self.total = self.total.saturating_add(1);
        let second = now_ms / 1000;
        let bucket = &mut self.ring[(second % WINDOW_SECS) as usize];
        if bucket.second != second {
            *bucket = Bucket { second, count: 0 };
        }
        bucket.count = bucket.count.saturating_add(1);
        let key = if self.places.contains_key(location) || self.places.len() < MAX_PLACES { location } else { OTHER };
        *self.places.entry(key.to_string()).or_insert(0) += 1;
    }

    /// Panics in the last [`WINDOW_SECS`] seconds.
    pub fn last_minute(&self, now_ms: u64) -> u32 {
        let now = now_ms / 1000;
        self.ring.iter().filter(|b| b.count > 0 && b.second <= now && now - b.second < WINDOW_SECS).fold(0u32, |sum, b| sum.saturating_add(b.count))
    }

    fn top(&self) -> Vec<PanicPlace> {
        let mut places: Vec<PanicPlace> = self.places.iter().map(|(location, count)| PanicPlace { location: location.clone(), count: *count }).collect();
        places.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.location.cmp(&b.location)));
        places.truncate(TOP_PLACES);
        places
    }

    /// The state now, as the event and the health report carry it.
    pub fn snapshot(&self, now_ms: u64, recovered_lock: u64) -> SrvPanicsEvent {
        let last_minute = self.last_minute(now_ms);
        let level = Level::for_count(last_minute);
        SrvPanicsEvent {
            kind: "panics".to_string(),
            level: level.as_str().to_string(),
            since_ms: if level == Level::Normal { None } else { Some(self.since_ms.unwrap_or(now_ms)) },
            last_minute,
            total: self.total,
            recovered_lock,
            top: self.top(),
        }
    }

    /// One look. The level follows the last minute, so a storm that ends clears itself one quiet minute later. An active level is
    /// published on every look (its counts move); a change to normal is published once. The note goes out when the level leaves
    /// normal, and then at most once an hour.
    pub fn look(&mut self, now_ms: u64, recovered_lock: u64) -> Step {
        let was = self.level;
        let last_minute = self.last_minute(now_ms);
        let level = Level::for_count(last_minute);
        if level != Level::Normal && self.since_ms.is_none() {
            self.since_ms = Some(now_ms);
        }
        if level == Level::Normal {
            self.since_ms = None;
        }
        self.level = level;
        let event = self.snapshot(now_ms, recovered_lock);
        let mut step = Step::default();
        if level != Level::Normal || was != Level::Normal {
            step.publish = Some(event.clone());
        }
        let rising = was == Level::Normal && level != Level::Normal;
        if rising && self.noted_at_ms.is_none_or(|at| now_ms.saturating_sub(at) >= NOTE_EVERY_MS) {
            self.noted_at_ms = Some(now_ms);
            step.note = Some(note_text(&event));
        }
        step
    }
}

/// What the agents are told.
pub fn note_text(event: &SrvPanicsEvent) -> String {
    let place = event.top.first().map(|p| format!(" (most often at {})", p.location)).unwrap_or_default();
    format!(
        "[AgentMux] srv is panicking: {} times in the last minute, {} since it started{place}. \
         Messages between agents, spawns or the Kanban may fail until AgentMux is restarted; restarting AgentMux (not the computer) clears a stuck messaging path. \
         Tell your leader.",
        event.last_minute, event.total
    )
}

/// `crates\srv\src\backend\reactive\handler.rs` and line 1416 as `backend/reactive/handler.rs:1416`: below the last `src/`, with
/// forward slashes, so a build path never leaves the machine.
pub fn short_location(file: &str, line: u32) -> String {
    let file = file.replace('\\', "/");
    let tail = file.rsplit_once("/src/").map(|(_, tail)| tail).unwrap_or_else(|| file.rsplit('/').next().unwrap_or(&file));
    format!("{tail}:{line}")
}

static WATCH: OnceLock<Mutex<Counts>> = OnceLock::new();

fn watch() -> MutexGuard<'static, Counts> {
    // Counts never panics while it holds this lock, so the lock is never poisoned; if it were, the numbers are still numbers.
    WATCH.get_or_init(|| Mutex::new(Counts::default())).lock().unwrap_or_else(|e| e.into_inner())
}

/// What this srv's panics add up to now, with the handler lock's recoveries.
pub fn snapshot() -> SrvPanicsEvent {
    let recovered = crate::backend::reactive::get_global_handler().recovered_lock_count();
    watch().snapshot(agentmux_common::time::now_ms_u64(), recovered)
}

/// Counts every panic from now on. The earlier hook (the default one, which writes the message) runs first and unchanged.
pub fn install_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        previous(info);
        record(info);
    }));
}

fn record(info: &PanicHookInfo<'_>) {
    let place = info.location().map(|l| short_location(l.file(), l.line())).unwrap_or_else(|| "(unknown place)".to_string());
    watch().record(&place, agentmux_common::time::now_ms_u64());
}

/// Holds `text` for every agent that is registered: it waits for each agent's next turn and starts none.
fn hold_for_agents(text: &str) {
    let handler = crate::backend::reactive::get_global_handler();
    for agent in handler.list_agents() {
        if let Err(e) = handler.hold_system_note(&agent.agent_id, NOTE_KEY, text) {
            tracing::debug!(agent = %agent.agent_id, error = %e, "srv panics: an agent was not told");
        }
    }
}

/// Looks at the count every few seconds, on a thread of its own: publishes the event and holds the note. A failed look costs
/// that look, not the watch.
pub fn start_reporter(broker: Arc<crate::backend::mps::Broker>) {
    let spawned = std::thread::Builder::new().name("srv-panic-watch".into()).spawn(move || loop {
        std::thread::sleep(LOOK_EVERY);
        let step = std::panic::catch_unwind(|| {
            let recovered = crate::backend::reactive::get_global_handler().recovered_lock_count();
            watch().look(agentmux_common::time::now_ms_u64(), recovered)
        });
        let Ok(step) = step else { continue };
        if let Some(event) = &step.publish {
            broker.publish_event::<crate::backend::mps_events::SrvPanics>(event, vec![], 1);
        }
        if let Some(text) = &step.note {
            tracing::warn!(note = %text, "srv panics: telling the agents");
            let _ = std::panic::catch_unwind(|| hold_for_agents(text));
        }
    });
    if let Err(e) = spawned {
        tracing::warn!(error = %e, "srv panics: the watch did not start");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: u64 = 1_000_000_000;

    fn storm(counts: &mut Counts, n: u32, at_ms: u64) {
        for _ in 0..n {
            counts.record("backend/reactive/handler.rs:1416", at_ms);
        }
    }

    #[test]
    fn the_level_follows_the_panics_of_the_last_minute() {
        assert_eq!(Level::for_count(0), Level::Normal);
        assert_eq!(Level::for_count(WARN_ABOVE), Level::Normal, "more than N, not N");
        assert_eq!(Level::for_count(WARN_ABOVE + 1), Level::Warn);
        assert_eq!(Level::for_count(CRITICAL_ABOVE), Level::Warn);
        assert_eq!(Level::for_count(CRITICAL_ABOVE + 1), Level::Critical);
    }

    #[test]
    fn the_last_minute_counts_only_the_last_sixty_seconds() {
        let mut c = Counts::default();
        storm(&mut c, 5, T0);
        storm(&mut c, 3, T0 + 30_000);
        assert_eq!(c.last_minute(T0 + 30_000), 8);
        assert_eq!(c.last_minute(T0 + 59_000), 8, "59 seconds after the first");
        assert_eq!(c.last_minute(T0 + 60_000), 3, "the first five are 60 seconds old");
        assert_eq!(c.last_minute(T0 + 90_000), 0);
        assert_eq!(c.total, 8, "the total never drops");
    }

    #[test]
    fn a_bucket_is_reused_when_its_second_comes_round_again() {
        let mut c = Counts::default();
        storm(&mut c, 4, T0);
        storm(&mut c, 2, T0 + 60_000); // the same ring slot, a minute later
        assert_eq!(c.last_minute(T0 + 60_000), 2, "the old second is gone");
    }

    #[test]
    fn a_quiet_srv_publishes_and_notes_nothing() {
        let mut c = Counts::default();
        storm(&mut c, WARN_ABOVE, T0);
        assert_eq!(c.look(T0 + 1000, 0), Step::default());
    }

    #[test]
    fn a_storm_publishes_every_look_and_notes_the_agents_once() {
        let mut c = Counts::default();
        storm(&mut c, WARN_ABOVE + 1, T0);
        let first = c.look(T0 + 1000, 0);
        let event = first.publish.expect("a warning is published");
        assert_eq!((event.kind.as_str(), event.level.as_str(), event.since_ms, event.last_minute), ("panics", "warn", Some(T0 + 1000), WARN_ABOVE + 1));
        let note = first.note.expect("the agents are told when it starts");
        assert!(note.starts_with("[AgentMux] srv is panicking: 21 times in the last minute") && note.contains("backend/reactive/handler.rs:1416"), "{note}");
        let second = c.look(T0 + 6000, 0);
        assert!(second.publish.is_some(), "the counts move, so it is published again");
        assert_eq!(second.publish.unwrap().since_ms, Some(T0 + 1000), "the episode began once");
        assert_eq!(second.note, None, "the agents are not told twice");
    }

    #[test]
    fn it_goes_critical_and_clears_itself_after_a_quiet_minute() {
        let mut c = Counts::default();
        storm(&mut c, CRITICAL_ABOVE + 1, T0);
        assert_eq!(c.look(T0 + 1000, 0).publish.unwrap().level, "critical");
        let ended = c.look(T0 + 61_000, 0);
        let event = ended.publish.expect("the change back to normal is published, so the signal clears");
        assert_eq!((event.level.as_str(), event.since_ms, event.last_minute), ("normal", None, 0));
        assert_eq!(c.look(T0 + 66_000, 0), Step::default(), "and then it is quiet again");
        assert_eq!(event.total, CRITICAL_ABOVE as u64 + 1, "the total stays");
    }

    #[test]
    fn a_second_storm_within_the_hour_is_published_but_not_noted() {
        let mut c = Counts::default();
        storm(&mut c, WARN_ABOVE + 1, T0);
        assert!(c.look(T0 + 1000, 0).note.is_some());
        c.look(T0 + 61_000, 0); // clear
        storm(&mut c, WARN_ABOVE + 1, T0 + 120_000);
        let again = c.look(T0 + 121_000, 0);
        assert!(again.publish.is_some() && again.note.is_none(), "the agents heard of it a few minutes ago");
        c.look(T0 + 181_000, 0); // clear
        let later = T0 + 61 * 60 * 1000;
        storm(&mut c, WARN_ABOVE + 1, later);
        assert!(c.look(later + 1000, 0).note.is_some(), "an hour on, they are told again");
    }

    #[test]
    fn the_top_places_are_the_biggest_three_and_the_list_stays_bounded() {
        let mut c = Counts::default();
        for (place, n) in [("a.rs:1", 5), ("b.rs:2", 9), ("c.rs:3", 7), ("d.rs:4", 1)] {
            for _ in 0..n {
                c.record(place, T0);
            }
        }
        let top: Vec<_> = c.top().into_iter().map(|p| (p.location, p.count)).collect();
        assert_eq!(top, [("b.rs:2".to_string(), 9), ("c.rs:3".to_string(), 7), ("a.rs:1".to_string(), 5)]);
        for i in 0..(MAX_PLACES + 40) {
            c.record(&format!("many/{i}.rs:1"), T0);
        }
        assert!(c.places.len() <= MAX_PLACES + 1, "{} places", c.places.len());
        assert!(c.places.contains_key(OTHER), "the rest are counted together");
    }

    #[test]
    fn a_location_loses_its_build_path_and_its_backslashes() {
        assert_eq!(short_location("crates\\srv\\src\\backend\\reactive\\handler.rs", 1416), "backend/reactive/handler.rs:1416");
        assert_eq!(short_location("/home/someone/work/agentmux/crates/srv/src/main.rs", 7), "main.rs:7");
        assert_eq!(short_location("/rustc/abc123/library/core/src/result.rs", 1), "result.rs:1");
        assert_eq!(short_location("lonely.rs", 3), "lonely.rs:3", "no src folder: the file name");
    }

    #[test]
    fn the_hook_counts_a_real_panic_and_leaves_the_earlier_hook_in_place() {
        // One test owns the process-wide hook: it chains whatever is there and counts what panics after it.
        let before = watch().snapshot(agentmux_common::time::now_ms_u64(), 0).total;
        let saw_first = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = saw_first.clone();
        let original = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |_| flag.store(true, std::sync::atomic::Ordering::SeqCst)));
        install_hook();
        let panicked = std::thread::spawn(|| panic!("counted")).join().is_err();
        std::panic::set_hook(original);
        assert!(panicked);
        assert!(saw_first.load(std::sync::atomic::Ordering::SeqCst), "the earlier hook still runs");
        let after = watch().snapshot(agentmux_common::time::now_ms_u64(), 0);
        assert!(after.total > before, "the panic was counted");
        assert!(after.top.iter().any(|p| p.location.starts_with("backend/panic_watch.rs:")), "at its place: {:?}", after.top);
    }
}
