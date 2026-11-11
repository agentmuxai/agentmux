// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Dev builds only: a window whose page already loaded when its dev server goes.
//!
//! `page_unreachable` starts only from a failed page load, and a page that is up
//! never navigates when its server stops, so a booted dev app stayed on screen
//! for ever (6 minutes and counting in one live check) with only the page's own
//! "server connection lost" line in the log. Here the host checks the address
//! of every window it served on a timer, with a connect and no page load. An
//! address that stays down for `DOWN_AFTER_ROUNDS` checks is reported as that
//! window's page being unreachable: the same state, so the pools stop refilling
//! and the dev build quits after `DEV_EXIT_AFTER`. When the address answers
//! again the window is marked loaded, whether or not its page reloaded itself.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use parking_lot::Mutex;

use super::page_unreachable::{address_answers, host_and_port, is_failed, label_of, note_loaded, on_failure};
use crate::state::AppState;

const CHECK_EVERY: Duration = Duration::from_secs(5);
/// Consecutive misses before a window counts as unreachable: a dev server restarting for a few seconds is not.
const DOWN_AFTER_ROUNDS: u32 = 3;

/// What one round of checks decided.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Round {
    /// Windows (label, address) whose server is gone.
    pub(crate) down: Vec<(String, String)>,
    /// Windows this module reported down whose server answers again.
    pub(crate) back: Vec<String>,
}

/// The windows served from an address, and how each fares.
#[derive(Default)]
pub(crate) struct Served {
    urls: HashMap<String, String>,
    misses: HashMap<String, u32>,
    /// The windows this module reported down: only those are cleared here.
    reported: HashSet<String>,
}

impl Served {
    pub(crate) fn navigated(&mut self, label: &str, url: &str) {
        self.urls.insert(label.to_string(), url.to_string());
    }

    pub(crate) fn labels(&self) -> Vec<String> {
        self.urls.keys().cloned().collect()
    }

    pub(crate) fn forget(&mut self, label: &str) {
        self.urls.remove(label);
        self.misses.remove(label);
        self.reported.remove(label);
    }

    pub(crate) fn urls(&self) -> Vec<(String, String)> {
        self.urls.iter().map(|(l, u)| (l.clone(), u.clone())).collect()
    }

    /// Fold in the answers of one round of connects. `failed` tells which windows
    /// are already unreachable for another reason (a failed load): they are left to it.
    pub(crate) fn round(&mut self, answers: &HashMap<String, bool>, failed: impl Fn(&str) -> bool) -> Round {
        let mut out = Round::default();
        for (label, url) in &self.urls {
            if answers.get(label).copied().unwrap_or(true) {
                self.misses.remove(label);
                if self.reported.remove(label) {
                    out.back.push(label.clone());
                }
                continue;
            }
            let misses = self.misses.entry(label.clone()).or_insert(0);
            *misses += 1;
            if *misses >= DOWN_AFTER_ROUNDS && !self.reported.contains(label) && !failed(label) {
                self.reported.insert(label.clone());
                out.down.push((label.clone(), url.clone()));
            }
        }
        out
    }
}

fn served() -> &'static Mutex<Served> {
    static SERVED: OnceLock<Mutex<Served>> = OnceLock::new();
    SERVED.get_or_init(|| Mutex::new(Served::default()))
}

static STARTED: AtomicBool = AtomicBool::new(false);

/// From `boot_watchdog::note_navigation`: an app window's main frame starts loading `url` (dev builds, http(s)).
pub(crate) fn note_navigation(state: &Arc<AppState>, url: &str) {
    served().lock().navigated(&label_of(url), url);
    if !STARTED.swap(true, Ordering::SeqCst) {
        let state = Arc::clone(state);
        std::thread::spawn(move || watch(state));
    }
}

fn watch(state: Arc<AppState>) {
    loop {
        std::thread::sleep(CHECK_EVERY);
        for label in served().lock().labels() {
            if state.get_browser(&label).is_none() {
                served().lock().forget(&label);
            }
        }
        // The connects run without the lock, once per address: every window shares the dev server.
        let urls = served().lock().urls();
        let mut by_address: HashMap<(String, u16), bool> = HashMap::new();
        let answers: HashMap<String, bool> = urls
            .into_iter()
            .filter_map(|(label, url)| {
                let key = host_and_port(&url)?;
                let up = *by_address.entry(key).or_insert_with(|| address_answers(&url));
                Some((label, up))
            })
            .collect();
        let round = served().lock().round(&answers, is_failed);
        for (label, url) in round.down {
            tracing::warn!(label = %label, "[dev-server] the dev server no longer answers — treating the app page as unreachable");
            on_failure(&state, &url, false);
        }
        for label in round.back {
            tracing::info!(label = %label, "[dev-server] the dev server answers again");
            note_loaded(&label);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const URL: &str = "http://localhost:5300/?ipc_port=1";

    fn served_main() -> Served {
        let mut s = Served::default();
        s.navigated("main", URL);
        s
    }

    fn answers(up: bool) -> HashMap<String, bool> {
        HashMap::from([("main".to_string(), up)])
    }

    fn rounds(s: &mut Served, n: u32, up: bool) -> Round {
        let mut last = Round::default();
        for _ in 0..n {
            last = s.round(&answers(up), |_| false);
        }
        last
    }

    /// The live check: the page is up, the server stops, nothing ever fails a load.
    #[test]
    fn a_server_that_stays_down_is_reported_once() {
        let mut s = served_main();
        assert_eq!(rounds(&mut s, DOWN_AFTER_ROUNDS - 1, false), Round::default(), "not yet");
        let r = s.round(&answers(false), |_| false);
        assert_eq!(r.down, vec![("main".to_string(), URL.to_string())]);
        assert_eq!(rounds(&mut s, 50, false), Round::default(), "reported once, not every round");
    }

    #[test]
    fn a_server_that_restarts_quickly_is_not_reported() {
        let mut s = served_main();
        for _ in 0..10 {
            rounds(&mut s, DOWN_AFTER_ROUNDS - 1, false);
            assert_eq!(rounds(&mut s, 1, true), Round::default());
        }
    }

    #[test]
    fn a_server_that_comes_back_clears_the_window_it_reported() {
        let mut s = served_main();
        rounds(&mut s, DOWN_AFTER_ROUNDS, false);
        assert_eq!(rounds(&mut s, 1, true).back, vec!["main".to_string()]);
        assert_eq!(rounds(&mut s, 1, true), Round::default(), "cleared once");
        assert_eq!(rounds(&mut s, DOWN_AFTER_ROUNDS, false).down.len(), 1, "and counted afresh the next time");
    }

    /// A window that failed to load is the other path's: this one neither reports nor clears it.
    #[test]
    fn a_window_that_already_failed_to_load_is_left_to_that_path() {
        let mut s = served_main();
        for _ in 0..DOWN_AFTER_ROUNDS + 2 {
            assert_eq!(s.round(&answers(false), |l| l == "main"), Round::default());
        }
        assert_eq!(rounds(&mut s, 1, true), Round::default());
    }

    #[test]
    fn a_closed_window_is_forgotten() {
        let mut s = served_main();
        rounds(&mut s, DOWN_AFTER_ROUNDS, false);
        s.forget("main");
        assert!(s.labels().is_empty());
        assert_eq!(rounds(&mut s, 5, false), Round::default());
    }

    #[test]
    fn only_the_windows_without_an_answer_are_reported() {
        let mut s = served_main();
        s.navigated("window-pool-ab", "http://localhost:5300/?windowLabel=window-pool-ab");
        let both = HashMap::from([("main".to_string(), false), ("window-pool-ab".to_string(), true)]);
        for _ in 0..DOWN_AFTER_ROUNDS {
            let r = s.round(&both, |_| false);
            assert!(r.down.iter().all(|(l, _)| l == "main"));
        }
    }
}
