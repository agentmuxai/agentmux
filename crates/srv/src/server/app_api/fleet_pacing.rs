// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! When each delivery of a broadcast starts (`fleet::fleet_broadcast_impl`).
//!
//! A broadcast used to start a turn on every target in the same second. Each
//! turn that starts in a pane on screen costs that window's page a burst of
//! work on its one thread, and sixteen of them in four seconds left the page
//! drawing four or five frames a second
//! (docs/reports/REPORT_UI_CHOP_ON_FLEET_BROADCAST_AND_UI_BUSY_METER_2026_11_19.md).
//!
//! The starts are spread by what each one costs this AgentMux, as a token
//! bucket: a small burst goes at once, the rest follow at a steady rate with a
//! little jitter, and the whole spread has a ceiling. A broadcast to two agents
//! is as fast as before; one to thirty takes a few seconds. A target that costs
//! this AgentMux nothing (another machine, another channel) is never delayed,
//! and a delivery that is slow or fails holds up no other: each starts at its
//! own time.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use super::AppState;
use crate::backend::obj::{self, Block, Workspace};
use crate::backend::rpc_types::FleetBroadcastProgressEvent;
use crate::backend::storage::store::Store;

/// A turn that starts in a pane a window of this AgentMux is showing: its
/// page draws it.
pub(crate) const COST_ON_SCREEN: f32 = 1.0;
/// A turn that starts on this AgentMux in a pane no window is showing: the
/// machine starts the work and the page gets its messages, but draws nothing.
pub(crate) const COST_OFF_SCREEN: f32 = 0.25;
/// A target this AgentMux only hands on (another machine or channel), or one
/// that has no route and fails at once.
pub(crate) const COST_NONE: f32 = 0.0;

/// Cost that goes at once, before pacing begins: two panes on screen.
const BURST: f32 = 2.0;
/// The time one unit of cost takes after the burst: two and a half turn starts
/// a second on screen, against the four a second that was too many.
const GAP: Duration = Duration::from_millis(400);
/// Each paced start moves by up to this share of its gap, either way, so the
/// starts do not land on a fixed beat.
const JITTER: f64 = 0.2;
/// The last delivery starts no later than this after the send: past it the
/// gap shrinks instead.
pub(crate) const CEILING: Duration = Duration::from_secs(6);

/// How long after the send each target's delivery starts, in the order of
/// `costs` (one per target, a `COST_*`). `noise` gives a value in `0..1` for
/// each paced start; 0.5 is no jitter.
pub(crate) fn start_offsets(costs: &[f32], mut noise: impl FnMut() -> f64) -> Vec<Duration> {
    let total: f32 = costs.iter().map(|c| c.max(0.0)).sum();
    let paced = (total - BURST).max(0.0) as f64;
    // The gap for one unit of cost: the steady one, or less when that would pass the ceiling.
    let gap = if paced > 0.0 { GAP.as_secs_f64().min(CEILING.as_secs_f64() / paced) } else { 0.0 };
    let mut spent = 0.0_f32;
    costs
        .iter()
        .map(|cost| {
            let cost = cost.max(0.0);
            spent += cost;
            let over = (spent - BURST) as f64;
            if cost == 0.0 || over <= 0.0 {
                return Duration::ZERO;
            }
            let jitter = (noise().clamp(0.0, 1.0) * 2.0 - 1.0) * JITTER * gap * cost as f64;
            // In whole milliseconds, so the ceiling is met exactly and not a rounding step short of it.
            let at_ms = ((over * gap + jitter) * 1000.0).round().clamp(0.0, CEILING.as_millis() as f64);
            Duration::from_millis(at_ms as u64)
        })
        .collect()
}

/// A repeatable stream of values in `0..1` from `seed` (splitmix64), for the
/// jitter: a broadcast's own id seeds it, so no random source is needed.
pub(crate) fn noise_from(seed: u64) -> impl FnMut() -> f64 {
    let mut state = seed;
    move || {
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        (z >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// Told as each delivery ends: how many have ended, of how many.
type Progress = Arc<dyn Fn(usize, usize) + Send + Sync>;

/// One broadcast's pace: when each target starts, and who hears as each
/// delivery ends.
pub(crate) struct Pace {
    offsets: Vec<Duration>,
    began: tokio::time::Instant,
    progress: Option<Progress>,
}

impl Pace {
    /// Every delivery at once, and nobody told.
    pub(crate) fn at_once() -> Self {
        Self::new(Vec::new(), None)
    }

    /// `offsets` in the order of the targets; a target past its end starts at once.
    pub(crate) fn new(offsets: Vec<Duration>, progress: Option<Progress>) -> Self {
        Self { offsets, began: tokio::time::Instant::now(), progress }
    }

    /// Returns when target `index` may start.
    pub(crate) async fn until_start(&self, index: usize) {
        if let Some(offset) = self.offsets.get(index).filter(|d| !d.is_zero()) {
            tokio::time::sleep_until(self.began + *offset).await;
        }
    }

    /// One more delivery has ended, delivered or failed: `done` of `total`.
    pub(crate) fn ended(&self, done: usize, total: usize) {
        if let Some(progress) = &self.progress {
            progress(done, total);
        }
    }
}

/// Those of `block_ids` a window of this AgentMux is showing: in the active
/// tab of a workspace that a window has open.
pub(crate) fn blocks_on_screen(store: &Store, block_ids: &[String]) -> HashSet<String> {
    let windows = store.get_all::<obj::Window>().unwrap_or_default();
    let shown_tabs: HashSet<String> = store
        .get_all::<Workspace>()
        .unwrap_or_default()
        .into_iter()
        .filter(|ws| !ws.activetabid.is_empty() && windows.iter().any(|w| w.workspaceid == ws.oid))
        .map(|ws| ws.activetabid)
        .collect();
    block_ids
        .iter()
        .filter(|id| {
            let tab = store.get::<Block>(id).ok().flatten().and_then(|b| b.parentoref.strip_prefix("tab:").map(str::to_string));
            tab.is_some_and(|t| shown_tabs.contains(&t))
        })
        .cloned()
        .collect()
}

/// What each of `targets` costs this AgentMux: `local` holds the ones that
/// start a turn here, `on_screen` those of them a window is showing.
pub(crate) fn costs_of(targets: &[String], local: &HashSet<String>, on_screen: &HashSet<String>) -> Vec<f32> {
    targets
        .iter()
        .map(|t| match (local.contains(t), on_screen.contains(t)) {
            (true, true) => COST_ON_SCREEN,
            (true, false) => COST_OFF_SCREEN,
            (false, _) => COST_NONE,
        })
        .collect()
}

/// The longest `progress_id` a sender may give (a UUID is 36).
const PROGRESS_ID_MAX: usize = 64;

/// The pace for one broadcast to `targets`, of which `local` start a turn on
/// this AgentMux. With a `progress_id`, each delivery that ends is published
/// as `fleetbroadcastprogress`.
pub(crate) async fn pace_for(state: &AppState, targets: &[String], local: HashSet<String>, msg_id: &str, progress_id: Option<String>) -> Pace {
    let (store, ids) = (state.mstore.clone(), local.iter().cloned().collect::<Vec<_>>());
    // If the look fails every local target counts as off screen: paced lightly, never held up.
    let on_screen = tokio::task::spawn_blocking(move || blocks_on_screen(&store, &ids)).await.unwrap_or_default();
    let seed = uuid::Uuid::parse_str(msg_id).map(|id| id.as_u128() as u64).unwrap_or_default();
    let offsets = start_offsets(&costs_of(targets, &local, &on_screen), noise_from(seed));
    let progress = progress_id.filter(|id| !id.is_empty() && id.len() <= PROGRESS_ID_MAX).map(|id| {
        let broker = state.broker.clone();
        Arc::new(move |done: usize, total: usize| {
            let event = FleetBroadcastProgressEvent { id: id.clone(), sent: done as u32, total: total as u32 };
            broker.publish_event::<crate::backend::mps_events::FleetBroadcastProgress>(&event, vec![], 0);
        }) as Progress
    });
    Pace::new(offsets, progress)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(costs: &[f32]) -> Vec<u64> {
        start_offsets(costs, || 0.5).iter().map(|d| d.as_millis() as u64).collect()
    }

    #[test]
    fn one_or_two_agents_get_it_at_once() {
        assert_eq!(ms(&[COST_ON_SCREEN]), vec![0]);
        assert_eq!(ms(&[COST_ON_SCREEN, COST_ON_SCREEN]), vec![0, 0]);
        assert!(ms(&[]).is_empty());
    }

    #[test]
    fn past_the_burst_the_starts_follow_at_a_steady_rate() {
        assert_eq!(ms(&[COST_ON_SCREEN; 5]), vec![0, 0, 400, 800, 1200]);
    }

    #[test]
    fn thirty_panes_on_screen_finish_at_the_ceiling_not_after_it() {
        let offsets = ms(&[COST_ON_SCREEN; 30]);
        assert_eq!(&offsets[..2], &[0, 0]);
        assert_eq!(*offsets.last().unwrap(), CEILING.as_millis() as u64);
        assert!(offsets.windows(2).all(|w| w[0] <= w[1]));
        // The gap shrank: 28 paced starts in 6 s, not 28 x 400 ms.
        assert_eq!(offsets[2], 214);
    }

    #[test]
    fn a_target_that_costs_nothing_is_never_delayed_wherever_it_sits() {
        let costs = [COST_ON_SCREEN, COST_NONE, COST_ON_SCREEN, COST_ON_SCREEN, COST_NONE, COST_ON_SCREEN];
        assert_eq!(ms(&costs), vec![0, 0, 0, 400, 0, 800]);
        assert_eq!(ms(&[COST_NONE; 40]), vec![0; 40]);
    }

    #[test]
    fn a_pane_no_window_shows_takes_a_quarter_of_a_slot() {
        let costs = [COST_ON_SCREEN, COST_ON_SCREEN, COST_OFF_SCREEN, COST_OFF_SCREEN, COST_OFF_SCREEN, COST_OFF_SCREEN, COST_ON_SCREEN];
        assert_eq!(ms(&costs), vec![0, 0, 100, 200, 300, 400, 800]);
    }

    // The broadcast the report describes: 31 targets, 14 of them panes in the sender's window.
    #[test]
    fn fourteen_panes_among_thirty_one_targets_start_over_about_five_seconds() {
        let mut costs = vec![COST_NONE; 31];
        for c in costs.iter_mut().step_by(2).take(14) {
            *c = COST_ON_SCREEN;
        }
        let offsets = ms(&costs);
        let on_screen: Vec<u64> = offsets.iter().zip(&costs).filter(|(_, c)| **c > 0.0).map(|(o, _)| *o).collect();
        assert_eq!(on_screen.len(), 14);
        assert_eq!(on_screen.last(), Some(&4800));
        // Four starts on screen in the first second (the burst and two more), three in any later one.
        for (i, at) in on_screen.iter().enumerate() {
            let in_the_next_second = on_screen[i..].iter().filter(|o| **o < at + 1000).count();
            assert!(in_the_next_second <= 4, "{in_the_next_second} starts within a second of {at} ms");
        }
        assert!(offsets.iter().zip(&costs).filter(|(_, c)| **c == 0.0).all(|(o, _)| *o == 0));
    }

    #[test]
    fn jitter_moves_a_start_by_a_fifth_of_its_gap_at_most_and_never_before_the_send_or_past_the_ceiling() {
        let early: Vec<u64> = start_offsets(&[COST_ON_SCREEN; 5], || 0.0).iter().map(|d| d.as_millis() as u64).collect();
        let late: Vec<u64> = start_offsets(&[COST_ON_SCREEN; 5], || 1.0).iter().map(|d| d.as_millis() as u64).collect();
        assert_eq!(early, vec![0, 0, 320, 720, 1120]);
        assert_eq!(late, vec![0, 0, 480, 880, 1280]);
        let capped = start_offsets(&[COST_ON_SCREEN; 30], || 1.0);
        assert!(capped.iter().all(|d| *d <= CEILING));
        // A value outside 0..1 is treated as the nearest end.
        assert_eq!(start_offsets(&[COST_ON_SCREEN; 3], || 7.0)[2].as_millis(), 480);
    }

    #[test]
    fn the_noise_is_repeatable_per_seed_and_stays_in_range() {
        let mut a = noise_from(42);
        let mut b = noise_from(42);
        let mut c = noise_from(43);
        let first: Vec<f64> = (0..50).map(|_| a()).collect();
        assert_eq!(first, (0..50).map(|_| b()).collect::<Vec<f64>>());
        assert_ne!(first, (0..50).map(|_| c()).collect::<Vec<f64>>());
        assert!(first.iter().all(|v| (0.0..1.0).contains(v)));
        // Not stuck on one side of the middle.
        assert!(first.iter().any(|v| *v < 0.5) && first.iter().any(|v| *v > 0.5));
    }

    fn tab_in(store: &Store, active: bool, with_window: bool) -> String {
        use crate::backend::obj::Tab;
        let mut shown = Tab { oid: uuid::Uuid::new_v4().to_string(), ..Default::default() };
        let mut other = Tab { oid: uuid::Uuid::new_v4().to_string(), ..Default::default() };
        store.insert(&mut shown).unwrap();
        store.insert(&mut other).unwrap();
        let mut ws = Workspace { oid: uuid::Uuid::new_v4().to_string(), tabids: vec![shown.oid.clone(), other.oid.clone()], activetabid: shown.oid.clone(), ..Default::default() };
        store.insert(&mut ws).unwrap();
        if with_window {
            let mut w = obj::Window { oid: uuid::Uuid::new_v4().to_string(), workspaceid: ws.oid.clone(), ..Default::default() };
            store.insert(&mut w).unwrap();
        }
        if active { shown.oid } else { other.oid }
    }

    fn block_in(store: &Store, tab_id: &str) -> String {
        let mut block = Block { oid: uuid::Uuid::new_v4().to_string(), parentoref: format!("tab:{tab_id}"), ..Default::default() };
        store.insert(&mut block).unwrap();
        block.oid
    }

    #[test]
    fn a_pane_is_on_screen_only_in_the_active_tab_of_a_workspace_a_window_shows() {
        let store = Store::open_in_memory().unwrap();
        let shown = block_in(&store, &tab_in(&store, true, true));
        let background_tab = block_in(&store, &tab_in(&store, false, true));
        let no_window = block_in(&store, &tab_in(&store, true, false));
        let ids = vec![shown.clone(), background_tab, no_window, "no-such-block".to_string()];
        assert_eq!(blocks_on_screen(&store, &ids), HashSet::from([shown]));
        assert!(blocks_on_screen(&store, &[]).is_empty());
    }

    #[test]
    fn a_target_costs_by_where_its_turn_runs_and_whether_a_window_shows_it() {
        let targets: Vec<String> = ["shown", "hidden", "remote"].iter().map(|s| s.to_string()).collect();
        let local = HashSet::from(["shown".to_string(), "hidden".to_string()]);
        let on_screen = HashSet::from(["shown".to_string()]);
        assert_eq!(costs_of(&targets, &local, &on_screen), vec![COST_ON_SCREEN, COST_OFF_SCREEN, COST_NONE]);
    }

    fn ids(n: usize) -> Vec<String> {
        (0..n).map(|i| format!("blk-{i}")).collect()
    }

    /// Runs a broadcast to `n` panes on screen, each delivery taking `each`, and gives back when each started
    /// (ms after the send, in target order) and the progress reports as they came.
    async fn paced_run(n: usize, each: Duration) -> (Vec<u64>, Vec<(usize, usize)>) {
        use std::sync::Mutex;
        let began = tokio::time::Instant::now();
        let starts: Arc<Mutex<Vec<(String, u64)>>> = Arc::default();
        let reports: Arc<Mutex<Vec<(usize, usize)>>> = Arc::default();
        let (starts2, reports2) = (starts.clone(), reports.clone());
        let pace = Pace::new(start_offsets(&vec![COST_ON_SCREEN; n], || 0.5), Some(Arc::new(move |done: usize, total: usize| reports2.lock().unwrap().push((done, total)))));
        let out = super::super::fleet::run_broadcast(ids(n), "x", "m", pace, |b| Some(format!("agent-{b}")), move |block, _text, _origin| {
            starts2.lock().unwrap().push((block, began.elapsed().as_millis() as u64));
            async move {
                tokio::time::sleep(each).await;
                Ok(())
            }
        })
        .await;
        assert_eq!(out.iter().map(|o| o.block_id.clone()).collect::<Vec<_>>(), ids(n), "outcomes come back in target order");
        let starts = starts.lock().unwrap();
        let at = |id: &String| starts.iter().find(|(b, _)| b == id).map(|(_, ms)| *ms).unwrap();
        let in_order = ids(n).iter().map(at).collect();
        let reports = reports.lock().unwrap().clone();
        (in_order, reports)
    }

    #[tokio::test(start_paused = true)]
    async fn deliveries_start_at_their_paced_times_and_the_sender_hears_of_each() {
        let (starts, reports) = paced_run(5, Duration::ZERO).await;
        assert_eq!(starts, vec![0, 0, 400, 800, 1200]);
        assert_eq!(reports, vec![(1, 5), (2, 5), (3, 5), (4, 5), (5, 5)]);
    }

    // Each delivery takes two seconds, far longer than the gap: the later ones still start on time.
    #[tokio::test(start_paused = true)]
    async fn a_slow_delivery_holds_up_no_other() {
        let (starts, reports) = paced_run(5, Duration::from_secs(2)).await;
        assert_eq!(starts, vec![0, 0, 400, 800, 1200]);
        assert_eq!(reports.last(), Some(&(5, 5)));
    }

    #[tokio::test(start_paused = true)]
    async fn two_agents_are_sent_to_at_once() {
        let (starts, reports) = paced_run(2, Duration::ZERO).await;
        assert_eq!(starts, vec![0, 0]);
        assert_eq!(reports, vec![(1, 2), (2, 2)]);
    }
}
