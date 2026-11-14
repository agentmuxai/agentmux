// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! How long a browser pane takes to open, and to go to a new page, phase by phase (#47585).
//!
//! **DO NOT REMOVE.** "Opening a pane feels slow" was, for a day, three wrong answers in a row because
//! nothing said which part of the open was slow. The UI-thread stall that finally explained it (a process
//! started for every load) showed only in sampled stacks. Every pane open and every navigation of a pane
//! now logs one line, so the next slow open starts from numbers.
//!
//! An open is the span from the host receiving `browser_pane_create` to the main frame finishing its load;
//! a navigation is the span from `on_before_browse` to the same. Each is cut at the points the host can see:
//!
//! | mark        | where                                                                          | gap named |
//! |-------------|--------------------------------------------------------------------------------|-----------|
//! | `Requested` | the request arrived (`ipc.rs`) / the navigation began (`on_before_browse`)     |           |
//! | `Started`   | the create task began on the UI thread (`CreateBrowserPaneTask`)               | `queue`   |
//! | `Handed`    | the engine was asked for the browser (jar, parent window, `browser_host_create_browser` / the overlay) | `create` |
//! | `Committed` | the main frame's document started (`on_load_start`)                            | `engine` (an open), `response` (a navigation) |
//! | `Loaded`    | the main frame finished (`on_load_end`)                                        | `load`    |
//!
//! `queue` is how long the create waited for the UI thread, which is where a stalled UI thread shows first.
//! `create` and `engine` are the host's and the engine's own work. A page's first paint is not a mark: CEF
//! tells a windowed or Views pane nothing about paints, and asking (DevTools `Page.enable`) takes the page's
//! dialogs from the host; `Loaded` is the nearest signal the host has without it.
//!
//! One line per finished open or navigation, at `info` (`warn` from [`SLOW_MS`]): filter with
//! `muxlog host pane-open`. The block id is the one in `pane-trace`'s lines, so the two read together.

use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use parking_lot::Mutex;

/// An open or navigation at least this long is logged at `warn`.
const SLOW_MS: u64 = 2000;

/// A timeline that has seen no mark for this long was lost (an open whose page never loaded, a pane closed
/// mid-load): a new one for its block replaces it.
const STALE: Duration = Duration::from_secs(60);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Phase {
    Requested,
    Started,
    Handed,
    Committed,
    Loaded,
}

impl Phase {
    const COUNT: usize = 5;
    const fn index(self) -> usize {
        self as usize
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Open,
    Navigate,
}

impl Kind {
    fn word(self) -> &'static str {
        match self {
            Kind::Open => "open",
            Kind::Navigate => "nav",
        }
    }

    /// The name of the gap that ends at `phase`.
    fn gap_name(self, phase: Phase) -> &'static str {
        match (self, phase) {
            (_, Phase::Requested) => "",
            (_, Phase::Started) => "queue",
            (_, Phase::Handed) => "create",
            (Kind::Open, Phase::Committed) => "engine",
            (Kind::Navigate, Phase::Committed) => "response",
            (_, Phase::Loaded) => "load",
        }
    }
}

/// One open or navigation: when each mark came, from the request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Timeline {
    pub kind: Kind,
    at: [Option<Duration>; Phase::COUNT],
}

impl Timeline {
    fn new(kind: Kind) -> Self {
        let mut at = [None; Phase::COUNT];
        at[Phase::Requested.index()] = Some(Duration::ZERO);
        Self { kind, at }
    }

    /// The first time each phase is reached counts; a retry or a second callback doesn't move it.
    fn mark(&mut self, phase: Phase, at: Duration) {
        self.at[phase.index()].get_or_insert(at);
    }

    fn last(&self) -> Duration {
        self.at.iter().flatten().copied().max().unwrap_or_default()
    }

    /// The time between each phase reached and the one reached before it, in order.
    pub(crate) fn gaps(&self) -> Vec<(&'static str, u64)> {
        let mut out = Vec::new();
        let mut before = Duration::ZERO;
        for phase in [Phase::Started, Phase::Handed, Phase::Committed, Phase::Loaded] {
            if let Some(at) = self.at[phase.index()] {
                out.push((self.kind.gap_name(phase), at.saturating_sub(before).as_millis() as u64));
                before = at;
            }
        }
        out
    }

    pub(crate) fn total_ms(&self) -> u64 {
        self.last().as_millis() as u64
    }

    /// `[pane-open] block=ab12cd34 open via=views queue=3ms create=12ms engine=240ms load=800ms total=1055ms slowest=load`
    pub(crate) fn line(&self, block: &str, via: &str) -> String {
        let gaps = self.gaps();
        let mut line = format!("[pane-open] block={} {} via={via}", short(block), self.kind.word());
        for (name, ms) in &gaps {
            line.push_str(&format!(" {name}={ms}ms"));
        }
        line.push_str(&format!(" total={}ms", self.total_ms()));
        if let Some((name, _)) = gaps.iter().max_by_key(|(_, ms)| *ms) {
            line.push_str(&format!(" slowest={name}"));
        }
        line
    }

    pub(crate) fn is_slow(&self) -> bool {
        self.total_ms() >= SLOW_MS
    }
}

fn short(block: &str) -> String {
    block.chars().take(8).collect()
}

struct Live {
    began: Instant,
    last_mark: Instant,
    timeline: Timeline,
}

/// The opens and navigations in flight, by block. A pane has at most one: a navigation begun during an open
/// is the open's own first load and does not start another.
#[derive(Default)]
pub(crate) struct Book {
    live: HashMap<String, Live>,
}

impl Book {
    pub(crate) fn begin(&mut self, kind: Kind, block: &str, now: Instant) {
        if kind == Kind::Navigate {
            // The first `on_before_browse` of an open is the open's own first load, not a navigation.
            let in_open = self
                .live
                .get(block)
                .is_some_and(|l| l.timeline.kind == Kind::Open && now.saturating_duration_since(l.last_mark) < STALE);
            if in_open {
                return;
            }
        }
        self.live.insert(
            block.to_string(),
            Live { began: now, last_mark: now, timeline: Timeline::new(kind) },
        );
    }

    pub(crate) fn mark(&mut self, block: &str, phase: Phase, now: Instant) {
        if let Some(live) = self.live.get_mut(block) {
            live.timeline.mark(phase, now.saturating_duration_since(live.began));
            live.last_mark = now;
        }
    }

    /// `Loaded`: the timeline is done and leaves the book.
    pub(crate) fn finish(&mut self, block: &str, now: Instant) -> Option<Timeline> {
        let mut live = self.live.remove(block)?;
        live.timeline.mark(Phase::Loaded, now.saturating_duration_since(live.began));
        Some(live.timeline)
    }

    pub(crate) fn forget(&mut self, block: &str) {
        self.live.remove(block);
    }
}

fn book() -> &'static Mutex<Book> {
    static CELL: OnceLock<Mutex<Book>> = OnceLock::new();
    CELL.get_or_init(Mutex::default)
}

/// A pane open was asked for.
pub(crate) fn begin_open(block: &str) {
    book().lock().begin(Kind::Open, block, Instant::now());
}

/// A main-frame navigation of an existing pane began (no-op during an open).
pub(crate) fn begin_navigation(block: &str) {
    book().lock().begin(Kind::Navigate, block, Instant::now());
}

/// The open or navigation of `block` reached `phase`.
pub(crate) fn mark(block: &str, phase: Phase) {
    book().lock().mark(block, phase, Instant::now());
}

/// The pane went away or was re-attached without a load: nothing more will come.
pub(crate) fn forget(block: &str) {
    book().lock().forget(block);
}

/// The main frame finished loading: log the line. `via` is the pane's kind (`views`, `hwnd`).
pub(crate) fn loaded(block: &str, via: &str) {
    let done = book().lock().finish(block, Instant::now());
    if let Some(timeline) = done {
        let line = timeline.line(block, via);
        if timeline.is_slow() {
            tracing::warn!(target: "pane-open", "{line}");
        } else {
            tracing::info!(target: "pane-open", "{line}");
        }
    }
}

#[cfg(test)]
#[path = "open_timing_tests.rs"]
mod tests;
