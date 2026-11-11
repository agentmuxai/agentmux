// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Dev builds only: a window whose page loaded but whose app never booted.
//!
//! When the dev frontend server restarts (a changed file its config imports, a
//! branch switch), windows that reload meanwhile can get the page's HTML and
//! then lose its modules. The page "loads", so the error page never shows, and
//! the window sits on the splash for ever (hours, in one sighting).
//!
//! The frontend reports it has booted (`set_window_init_status`, or
//! `pool_window_ready` for a pooled window). A window that has not reported
//! `BOOT_DEADLINE` after its page started loading is reloaded once the dev
//! server answers, at most `MAX_AUTO_RELOADS` times until it boots.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use parking_lot::Mutex;

use super::page_unreachable::{address_answers, is_dev, label_of, ReloadTask, MAX_AUTO_RELOADS};
use crate::state::AppState;

const BOOT_DEADLINE: Duration = Duration::from_secs(90);
const CHECK_EVERY: Duration = Duration::from_secs(5);

struct Waiting {
    url: String,
    since: Instant,
    reloads: u32,
}

/// The windows whose page started loading and that have not booted.
#[derive(Default)]
pub(crate) struct Windows(HashMap<String, Waiting>);

impl Windows {
    /// A page started loading in `label`. The reload count carries over: a reload
    /// the watchdog did is itself a navigation.
    pub(crate) fn navigated(&mut self, label: &str, url: &str, now: Instant) {
        let reloads = self.0.get(label).map_or(0, |w| w.reloads);
        self.0.insert(label.to_string(), Waiting { url: url.to_string(), since: now, reloads });
    }

    pub(crate) fn booted(&mut self, label: &str) {
        self.0.remove(label);
    }

    /// Windows past the deadline with reloads left.
    pub(crate) fn overdue(&self, now: Instant, deadline: Duration) -> Vec<(String, String)> {
        self.0
            .iter()
            .filter(|(_, w)| now.duration_since(w.since) >= deadline && w.reloads < MAX_AUTO_RELOADS)
            .map(|(l, w)| (l.clone(), w.url.clone()))
            .collect()
    }

    /// The window is being reloaded: count it and restart its clock. False when it is not waiting any
    /// more (it booted while the address was being checked): then there is nothing to reload.
    pub(crate) fn reloaded(&mut self, label: &str, now: Instant) -> bool {
        let Some(w) = self.0.get_mut(label) else { return false };
        w.reloads += 1;
        w.since = now;
        true
    }
}

fn windows() -> &'static Mutex<Windows> {
    static WINDOWS: OnceLock<Mutex<Windows>> = OnceLock::new();
    WINDOWS.get_or_init(|| Mutex::new(Windows::default()))
}

static STARTED: AtomicBool = AtomicBool::new(false);

/// From `on_before_browse`: an app window's main frame starts loading `url`.
pub(crate) fn note_navigation(state: &Arc<AppState>, url: &str) {
    let lower = url.trim_start().to_ascii_lowercase();
    if !is_dev() || !(lower.starts_with("http://") || lower.starts_with("https://")) {
        return;
    }
    windows().lock().navigated(&label_of(url), url, Instant::now());
    super::dev_server_watch::note_navigation(state, url);
    if !STARTED.swap(true, Ordering::SeqCst) {
        let state = Arc::clone(state);
        std::thread::spawn(move || watch(state));
    }
}

/// The frontend of `label` booted (or the window failed in a way `page_unreachable` handles).
pub(crate) fn note_booted(label: &str) {
    windows().lock().booted(label);
}

fn watch(state: Arc<AppState>) {
    loop {
        std::thread::sleep(CHECK_EVERY);
        let overdue = windows().lock().overdue(Instant::now(), BOOT_DEADLINE);
        for (label, url) in overdue {
            if state.get_browser(&label).is_none() {
                note_booted(&label);
                continue;
            }
            // The network check runs without the lock.
            if !address_answers(&url) {
                continue;
            }
            // Checked again after the connect: the window may have booted meanwhile, and a reload would undo that.
            if !windows().lock().reloaded(&label, Instant::now()) {
                continue;
            }
            tracing::warn!(label = %label, "[boot-watchdog] the page loaded but the app has not booted — reloading the window");
            let mut task = ReloadTask::new(Arc::clone(&state), label, url);
            cef::post_task(cef::ThreadId::UI, Some(&mut task));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const URL: &str = "http://localhost:5300/?ipc_port=1";

    #[test]
    fn a_window_that_boots_is_never_reloaded() {
        let mut w = Windows::default();
        let t0 = Instant::now();
        w.navigated("main", URL, t0);
        assert!(w.overdue(t0 + Duration::from_secs(89), BOOT_DEADLINE).is_empty());
        w.booted("main");
        assert!(w.overdue(t0 + Duration::from_secs(3_600), BOOT_DEADLINE).is_empty());
    }

    /// The sighting: the page loaded, the app never booted, nothing reloaded it.
    #[test]
    fn a_window_that_never_boots_is_reloaded_when_the_deadline_passes() {
        let mut w = Windows::default();
        let t0 = Instant::now();
        w.navigated("main", URL, t0);
        assert_eq!(w.overdue(t0 + BOOT_DEADLINE, BOOT_DEADLINE), vec![("main".to_string(), URL.to_string())]);
    }

    #[test]
    fn the_reloads_are_bounded_and_each_restarts_the_clock() {
        let mut w = Windows::default();
        let mut now = Instant::now();
        w.navigated("main", URL, now);
        for _ in 0..MAX_AUTO_RELOADS {
            now += BOOT_DEADLINE;
            assert_eq!(w.overdue(now, BOOT_DEADLINE).len(), 1);
            w.reloaded("main", now);
            assert!(w.overdue(now, BOOT_DEADLINE).is_empty(), "the clock restarted");
            w.navigated("main", URL, now); // the reload is a navigation; the count stays
        }
        now += BOOT_DEADLINE * 10;
        assert!(w.overdue(now, BOOT_DEADLINE).is_empty(), "no more than {MAX_AUTO_RELOADS} reloads");
    }

    /// The check of the address takes a while; a window that booted in that time is not reloaded.
    #[test]
    fn a_window_that_booted_while_its_address_was_checked_is_not_reloaded() {
        let mut w = Windows::default();
        let t0 = Instant::now();
        w.navigated("main", URL, t0);
        assert_eq!(w.overdue(t0 + BOOT_DEADLINE, BOOT_DEADLINE).len(), 1);
        w.booted("main");
        assert!(!w.reloaded("main", t0 + BOOT_DEADLINE), "no reload, no count");
        w.navigated("main", URL, t0);
        assert!(w.reloaded("main", t0));
    }

    #[test]
    fn a_window_that_boots_gets_its_reloads_back() {
        let mut w = Windows::default();
        let now = Instant::now();
        w.navigated("main", URL, now);
        w.reloaded("main", now);
        w.booted("main");
        w.navigated("main", URL, now);
        assert_eq!(w.0["main"].reloads, 0);
    }
}
