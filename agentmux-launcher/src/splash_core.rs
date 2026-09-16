// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Shared startup-splash timeline logic: the row model, event application
//! with same-tick count-up deferral, a frozen-row finalizer, duration
//! formatting, and total/other reconciliation — used identically by all
//! three platform splash renderers (`splash.rs` Windows, `splash_mac.rs`
//! macOS, `splash_linux/` Linux).
//!
//! WHY THIS EXISTS NOW, NOT EARLIER. Before this module, each platform
//! reimplemented this exact logic from scratch — `splash_mac.rs`'s own doc
//! comment explained why at the time: "editing two already-working splashes
//! I cannot build or run to verify" was too risky for the PR that first
//! needed a macOS stage panel. The predictable result (see
//! `docs/reports/REPORT_SPLASH_SCREEN_ARCHITECTURE_RETHINK_2026_09_14.md`
//! §1): the same-tick count-up deferral fix landed only on macOS, the
//! frozen-row defect (a row whose `End` never arrives stays visually
//! "running" forever once the render loop stops redrawing on a timer) was
//! never fixed on ANY platform — including macOS, despite it having the
//! count-up fix — and total/other reconciliation existed only on macOS, as
//! a bare unreconciled `"total:"` line on Windows and not at all on Linux.
//! Three independent implementations meant every fix needed to land three
//! times, and in practice none of them ever did.
//!
//! Platform-specific compositing (Win32 GDI blitting, Cocoa NSTextField
//! layout, X11/Wayland pixel buffers) stays in each platform's own file —
//! only the pure logic (no FFI, no windowing, no `Instant` mocking needed)
//! lives here, which is also what makes it unit-testable without a live
//! window on any platform, including the two this was written on without
//! being able to run them.

use std::time::{Duration, Instant};

use crate::startup_events::{StartupEvent, StartupStatus};

/// A completed sub-item or stage's outcome: elapsed ms, status, and an
/// optional short annotation carried over from the originating event.
pub type Outcome = (u64, StartupStatus, Option<String>);

pub struct SubEntry {
    pub id: String,
    pub label: String,
    pub started_at: Instant,
    pub done: Option<Outcome>,
}

pub struct StageEntry {
    pub stage: &'static str,
    pub label: &'static str,
    pub started_at: Instant,
    pub done: Option<Outcome>,
    pub subs: Vec<SubEntry>,
}

/// Identifies which row a `Begin` event created, so the same-tick deferral
/// in `apply_tick` can recognize an `End` for a row created in the very same
/// batch.
#[derive(PartialEq)]
enum BeginKey {
    Stage(&'static str),
    Sub(&'static str, String),
}

/// The full timeline: every stage/sub-item seen so far, plus the state the
/// same-tick deferral needs to carry between calls.
#[derive(Default)]
pub struct StageTimeline {
    pub stages: Vec<StageEntry>,
    deferred: Vec<StartupEvent>,
}

impl StageTimeline {
    pub fn new() -> Self {
        Self { stages: Vec::new(), deferred: Vec::new() }
    }

    fn apply_one(&mut self, ev: StartupEvent) {
        match ev {
            StartupEvent::StageBegin { stage, label } => {
                self.stages.push(StageEntry {
                    stage,
                    label,
                    started_at: Instant::now(),
                    done: None,
                    subs: Vec::new(),
                });
            }
            StartupEvent::StageEnd { stage, duration_ms, status, detail } => {
                if let Some(row) = self.stages.iter_mut().rev().find(|r| r.stage == stage) {
                    row.done = Some((duration_ms, status, detail));
                }
            }
            StartupEvent::SubBegin { stage, id, label } => {
                if let Some(row) = self.stages.iter_mut().rev().find(|r| r.stage == stage) {
                    row.subs.push(SubEntry { id, label, started_at: Instant::now(), done: None });
                }
            }
            StartupEvent::SubEnd { stage, id, duration_ms, status, detail } => {
                if let Some(row) = self.stages.iter_mut().rev().find(|r| r.stage == stage) {
                    if let Some(sub) = row.subs.iter_mut().rev().find(|s| s.id == id) {
                        sub.done = Some((duration_ms, status, detail));
                    }
                }
            }
        }
    }

    /// Apply one tick's worth of freshly-drained events, deferring any `End`
    /// whose matching `Begin` *also* arrived in this same batch to the next
    /// call — guaranteeing every row spends at least one call (and, since
    /// every platform's render loop redraws every tick while anything is
    /// running, at least one paint) in the "running" state before it can
    /// complete.
    ///
    /// Without this, a step whose real work finishes inside a single drain
    /// interval creates its row already-done — no "running" frame is ever
    /// painted, so it just snaps to its final value instead of visibly
    /// counting up
    /// (`docs/analysis/ANALYSIS_SPLASH_SCREEN_TIMING_2026_07_20.md` §2 first
    /// diagnosed this; fixed only on macOS at the time — see this module's
    /// doc comment for why it's here, for every platform, now).
    ///
    /// Returns `true` if anything changed (a redraw is warranted).
    pub fn apply_tick(&mut self, fresh: Vec<StartupEvent>) -> bool {
        let mut changed = false;
        let deferred = std::mem::take(&mut self.deferred);
        for ev in deferred {
            self.apply_one(ev);
            changed = true;
        }
        let mut began_this_tick: Vec<BeginKey> = Vec::new();
        for ev in fresh {
            let defer = match &ev {
                StartupEvent::StageEnd { stage, .. } => {
                    began_this_tick.contains(&BeginKey::Stage(stage))
                }
                StartupEvent::SubEnd { stage, id, .. } => {
                    began_this_tick.contains(&BeginKey::Sub(stage, id.clone()))
                }
                _ => false,
            };
            if defer {
                self.deferred.push(ev);
                continue;
            }
            match &ev {
                StartupEvent::StageBegin { stage, .. } => {
                    began_this_tick.push(BeginKey::Stage(stage));
                }
                StartupEvent::SubBegin { stage, id, .. } => {
                    began_this_tick.push(BeginKey::Sub(stage, id.clone()));
                }
                _ => {}
            }
            self.apply_one(ev);
            changed = true;
        }
        changed
    }

    /// Called once, at the moment the splash detects readiness (or its
    /// safety timeout), for every stage/sub-item still lacking a matching
    /// `End`. Without this, such a row stays visually "running" (live
    /// counter, running color) forever once the render loop stops
    /// redrawing on a timer and switches to redrawing only on-change (or,
    /// pre-refactor Linux, stops updating entirely once composited for
    /// dismiss) — the row is drawn once more at the freeze and then never
    /// again, frozen exactly where it was.
    ///
    /// See `docs/reports/REPORT_SPLASH_SCREEN_ARCHITECTURE_RETHINK_2026_09_14.md`
    /// §2.2 for an already-shipping concrete instance this closes: Windows'
    /// `first-run-wait` sub-row has no `sub_end` call anywhere in the
    /// codebase and was frozen "running" for the rest of every boot that
    /// triggered it, on every platform, including macOS despite macOS
    /// already having the count-up fix — the two defects are independent.
    ///
    /// Reuses `StartupStatus::Warn` rather than adding a new status variant:
    /// "didn't receive a definitive completion signal before the splash
    /// dismissed" is itself a real warning-worthy condition, not a third
    /// rendering concept every platform's color table would need a new arm
    /// for.
    ///
    /// Flushes `self.deferred` first, as genuine completions — NOT as
    /// abandoned/interrupted work. An entry sitting in `deferred` is a real
    /// `End` that already arrived; it was only held back one tick so its
    /// row got at least one "running" paint first (`apply_tick`'s deferral).
    /// If readiness is detected in that exact same tick, finalizing before
    /// flushing would discard the row's true, already-known outcome and
    /// replace it with a synthetic "interrupted" one — wrong, and worse on
    /// Windows than elsewhere: its dismiss path calls this once and then
    /// freezes the summary for the whole hold + fade with no further
    /// `apply_tick` call, so the wrong value would be the only one the user
    /// ever sees (codex P1 / reagent P1 on PR #3222).
    pub fn finalize_running(&mut self, at: Instant) {
        let deferred = std::mem::take(&mut self.deferred);
        for ev in deferred {
            self.apply_one(ev);
        }
        for stage in &mut self.stages {
            if stage.done.is_none() {
                let ms = at.saturating_duration_since(stage.started_at).as_millis() as u64;
                stage.done = Some((ms, StartupStatus::Warn, Some("interrupted".to_string())));
            }
            for sub in &mut stage.subs {
                if sub.done.is_none() {
                    let ms = at.saturating_duration_since(sub.started_at).as_millis() as u64;
                    sub.done = Some((ms, StartupStatus::Warn, Some("interrupted".to_string())));
                }
            }
        }
    }
}

// ── Visible rows: summarize, then keep the latest in view ────────────────────

/// How many of a stage's sub-items are rendered individually. Anything older
/// collapses into a single tally row.
///
/// Two, not one: the newest sub-item is usually the one still running, and
/// seeing only it gives no sense of progress — the one just finished, with its
/// real duration, is what tells you the batch is moving. Anything beyond that
/// is history the user cannot act on, which is exactly the "too many detailed
/// entries" complaint in
/// `docs/reports/REPORT_SPLASH_MIGRATION_ROW_OVERFLOW_AND_SUMMARY_2026_09_15.md`.
pub const SUB_DETAIL_MAX: usize = 2;

/// What one rendered line stands for. Platform renderers match on this and
/// draw it their own way (GDI text, an `NSTextField` pair, a formatted
/// string) — the *selection* of what to draw is shared, which is the part
/// that was wrong in three places independently.
pub enum RowKind<'a> {
    Stage(&'a StageEntry),
    Sub(&'a SubEntry),
    /// Stands in for sub-items collapsed out of the detail list.
    /// `done` of `total` sub-items in this stage have completed.
    SubTally { done: usize, total: usize },
}

pub struct Row<'a> {
    pub kind: RowKind<'a>,
    /// Sub-items and tallies are indented under their stage; stages are not.
    pub indented: bool,
}

/// The label for a collapsed-sub-items row. Shared so all three platforms
/// word it identically.
pub fn tally_label(done: usize, total: usize) -> String {
    format!("{done}/{total} done")
}

/// Flatten `stages` into at most `budget` rows: summarize each stage's older
/// sub-items into a tally, then — if that still doesn't fit — keep the
/// **newest** rows rather than the oldest.
///
/// The keep-the-newest half is the behavior change. Every platform previously
/// iterated stages oldest-first and stopped dead at its row cap
/// (`if row >= MAX_STAGE_ROWS { break }`), so a long sub-item burst filled the
/// panel with the *earliest* entries and never showed the one currently
/// running — the opposite of what a progress display is for, and the reason a
/// slow startup looked frozen rather than busy. See the report above.
pub fn visible_rows(stages: &[StageEntry], budget: usize) -> Vec<Row<'_>> {
    // One block per stage: its own row first, then its (summarized) sub rows.
    // Trimming happens block-wise below — an indented row must never outlive
    // the stage row it belongs under.
    let mut blocks: Vec<Vec<Row<'_>>> = Vec::new();
    for stage in stages {
        let mut block = vec![Row { kind: RowKind::Stage(stage), indented: false }];
        let subs = &stage.subs;
        if subs.len() > SUB_DETAIL_MAX {
            // Counted over ALL of the stage's sub-items, not just the hidden
            // ones: this is the stage's progress ratio, so it has to keep
            // rising as the two always-shown items finish too.
            let done = subs.iter().filter(|s| s.done.is_some()).count();
            block.push(Row {
                kind: RowKind::SubTally { done, total: subs.len() },
                indented: true,
            });
            for sub in &subs[subs.len() - SUB_DETAIL_MAX..] {
                block.push(Row { kind: RowKind::Sub(sub), indented: true });
            }
        } else {
            for sub in subs {
                block.push(Row { kind: RowKind::Sub(sub), indented: true });
            }
        }
        blocks.push(block);
    }

    // Fill from the newest block backwards. A block that only partially fits
    // keeps its stage row plus as many of its newest sub rows as are left —
    // never the children alone, which would render as indented rows dangling
    // under whatever stage happened to precede them.
    let mut out: Vec<Row<'_>> = Vec::new();
    let mut remaining = budget;
    for block in blocks.into_iter().rev() {
        if remaining == 0 {
            break;
        }
        let mut chunk: Vec<Row<'_>> = if block.len() <= remaining {
            block
        } else {
            let mut it = block.into_iter();
            let header = it.next().expect("every block starts with its stage row");
            let subs: Vec<Row<'_>> = it.collect();
            let keep = remaining - 1;
            let skip = subs.len() - keep;
            let mut chunk = Vec::with_capacity(remaining);
            chunk.push(header);
            chunk.extend(subs.into_iter().skip(skip));
            chunk
        };
        remaining -= chunk.len();
        chunk.append(&mut out);
        out = chunk;
    }
    out
}

// ── Formatting ───────────────────────────────────────────────────────────────

pub fn format_ms(ms: u64) -> String {
    if ms >= 10_000 {
        format!("{:.0}s", ms as f64 / 1000.0)
    } else if ms >= 1_000 {
        format!("{:.1}s", ms as f64 / 1000.0)
    } else {
        format!("{ms}ms")
    }
}

pub fn format_running(started_at: Instant) -> String {
    let s = started_at.elapsed().as_secs_f32();
    if s >= 10.0 {
        format!("> {:.0}s", s)
    } else {
        format!("> {:.1}s", s)
    }
}

/// Truncates to `max` chars, appending `..` when it does. Char-based (not
/// byte-based) so it never splits a multi-byte codepoint.
pub fn trunc(s: &str, max: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max {
        s.to_string()
    } else {
        chars[..max].iter().collect::<String>() + ".."
    }
}

// ── Total / other reconciliation ────────────────────────────────────────────

/// Sum of every *completed* top-level stage's own reported duration.
/// Only completed stages are summed: a stage's own duration already covers
/// whatever its subs took (summing subs too would double-count), and an
/// undone stage would inflate the sum with an ever-changing partial value.
pub fn accounted_ms(stages: &[StageEntry]) -> u64 {
    stages.iter().filter_map(|s| s.done.as_ref().map(|(dur, ..)| *dur)).sum()
}

/// `(accounted_ms, other_ms)` given a wall-clock `total_ms`. `other_ms` is
/// real elapsed time genuinely uncovered by any instrumented stage (process-
/// spawn scheduling gaps, cef_init-end -> first-paint, etc.) — not a double-
/// count or a bug; see
/// `docs/analysis/ANALYSIS_SPLASH_SCREEN_TIMING_2026_07_20.md` §5.
/// `saturating_sub` guards the should-be-impossible case where accounted
/// time exceeds the wall-clock total.
pub fn reconcile(stages: &[StageEntry], total_ms: u64) -> (u64, u64) {
    let accounted = accounted_ms(stages);
    (accounted, total_ms.saturating_sub(accounted))
}

// ── Hold duration ────────────────────────────────────────────────────────────

/// How long the splash holds its completed, frozen summary on screen before
/// fading out. Same env var, same default, on all three platforms — before
/// this module, Windows and macOS each parsed `AGENTMUX_SPLASH_HOLD_MS`
/// independently (both happened to converge on the same `2000` default, not
/// the original spec's documented `3000` — kept here as the actual shipped
/// behavior rather than "fixing" it to match a spec nothing currently
/// honors) and Linux used a differently-named, differently-scoped
/// `min_hold()` answering a different question ("minimum total time
/// visible" rather than "pause after ready, before dismissing") — see
/// `docs/reports/REPORT_SPLASH_SCREEN_ARCHITECTURE_RETHINK_2026_09_14.md`
/// §4.4. Shortened to `min(1000ms)` for a very fast start (<500ms total) so
/// a near-instant cold start doesn't force an artificially long hold.
pub fn hold_duration(total_ms: u64) -> Duration {
    let hold_ms = std::env::var("AGENTMUX_SPLASH_HOLD_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(2000);
    let hold_ms = if total_ms < 500 { hold_ms.min(1000) } else { hold_ms };
    Duration::from_millis(hold_ms)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread::sleep;

    fn begin(stage: &'static str) -> StartupEvent {
        StartupEvent::StageBegin { stage, label: stage }
    }
    fn end(stage: &'static str, ms: u64) -> StartupEvent {
        StartupEvent::StageEnd { stage, duration_ms: ms, status: StartupStatus::Ok, detail: None }
    }

    #[test]
    fn a_begin_alone_leaves_the_row_running() {
        let mut tl = StageTimeline::new();
        assert!(tl.apply_tick(vec![begin("prep")]));
        assert_eq!(tl.stages.len(), 1);
        assert!(tl.stages[0].done.is_none());
    }

    #[test]
    fn an_end_on_a_later_tick_completes_the_row() {
        let mut tl = StageTimeline::new();
        tl.apply_tick(vec![begin("prep")]);
        assert!(tl.apply_tick(vec![end("prep", 42)]));
        assert_eq!(tl.stages[0].done.as_ref().unwrap().0, 42);
    }

    #[test]
    fn a_same_tick_begin_and_end_is_deferred_not_applied_immediately() {
        // The count-up bug this module exists to fix on every platform: a
        // fast step whose Begin+End land in the same drain batch must NOT
        // be created already-done — the row needs at least one call in the
        // "running" state.
        let mut tl = StageTimeline::new();
        tl.apply_tick(vec![begin("prep"), end("prep", 5)]);
        assert!(tl.stages[0].done.is_none(), "same-tick End must be deferred, not applied immediately");
        tl.apply_tick(vec![]);
        assert_eq!(tl.stages[0].done.as_ref().unwrap().0, 5, "deferred End must apply on the next tick");
    }

    #[test]
    fn two_events_in_one_block_do_not_interfere() {
        let mut tl = StageTimeline::new();
        tl.apply_tick(vec![begin("prep"), begin("migrations"), end("prep", 5)]);
        // "prep" is deferred (same-tick begin+end); "migrations" is not
        // (its Begin has no matching End this tick) and stays running.
        assert!(tl.stages[0].done.is_none());
        assert!(tl.stages[1].done.is_none());
        tl.apply_tick(vec![]);
        assert_eq!(tl.stages[0].done.as_ref().unwrap().0, 5);
        assert!(tl.stages[1].done.is_none(), "an unrelated sibling row must not be finalized by an unrelated tick");
    }

    #[test]
    fn finalize_running_completes_every_open_row_and_leaves_finished_ones_alone() {
        let mut tl = StageTimeline::new();
        tl.apply_tick(vec![begin("prep")]);
        tl.apply_tick(vec![]);
        tl.apply_tick(vec![end("prep", 10)]);
        tl.apply_tick(vec![begin("host")]); // never completes
        sleep(Duration::from_millis(5));

        tl.finalize_running(Instant::now());

        assert_eq!(tl.stages[0].done.as_ref().unwrap().0, 10, "an already-done row must be left exactly as it was");
        let host_done = tl.stages[1].done.as_ref().expect("finalize must complete every still-open row");
        assert_eq!(host_done.1, StartupStatus::Warn);
    }

    #[test]
    fn finalize_running_also_closes_open_sub_rows() {
        // The concrete shipping bug this closes: a sub-row (e.g. Windows'
        // `first-run-wait`) with no matching SubEnd anywhere.
        let mut tl = StageTimeline::new();
        tl.apply_tick(vec![begin("host")]);
        tl.apply_tick(vec![]);
        tl.apply_tick(vec![StartupEvent::SubBegin {
            stage: "host",
            id: "first-run-wait".into(),
            label: "First run can take longer".into(),
        }]);
        tl.apply_tick(vec![]);

        tl.finalize_running(Instant::now());

        assert!(tl.stages[0].subs[0].done.is_some(), "an orphaned sub-row must be finalized, not left running forever");
    }

    #[test]
    fn finalize_running_flushes_a_same_tick_deferral_as_the_real_completion_not_as_interrupted() {
        // codex P1 / reagent P1 on PR #3222: readiness detected in the exact
        // same tick a fast stage's Begin+End were deferred (same-tick count-up
        // fix) must not discard that real, already-known completion and
        // replace it with a synthetic "interrupted" Warn outcome.
        let mut tl = StageTimeline::new();
        tl.apply_tick(vec![begin("prep"), end("prep", 7)]);
        assert!(tl.stages[0].done.is_none(), "precondition: the End is deferred, not yet applied");

        tl.finalize_running(Instant::now());

        let (ms, status, _) = tl.stages[0].done.as_ref().expect("must be completed one way or another");
        assert_eq!(*ms, 7, "the row's TRUE duration must survive, not a synthetic elapsed-since-start value");
        assert_eq!(*status, StartupStatus::Ok, "a real completion must not be downgraded to Warn/interrupted");
    }

    #[test]
    fn accounted_and_other_reconcile_exactly_against_total() {
        let mut tl = StageTimeline::new();
        tl.apply_tick(vec![begin("prep")]);
        tl.apply_tick(vec![end("prep", 100)]);
        tl.apply_tick(vec![begin("backend")]);
        tl.apply_tick(vec![end("backend", 250)]);

        let (accounted, other) = reconcile(&tl.stages, 500);
        assert_eq!(accounted, 350);
        assert_eq!(other, 150);
        assert_eq!(accounted + other, 500, "accounted + other must equal total exactly, by construction");
    }

    #[test]
    fn reconcile_never_panics_when_accounted_exceeds_total() {
        // Should-be-impossible (a stage can't legitimately take longer than
        // the whole splash), but a formatting bug must never become a panic.
        let mut tl = StageTimeline::new();
        tl.apply_tick(vec![begin("prep")]);
        tl.apply_tick(vec![end("prep", 1000)]);
        let (accounted, other) = reconcile(&tl.stages, 10);
        assert_eq!(accounted, 1000);
        assert_eq!(other, 0);
    }

    #[test]
    fn an_undone_stage_is_excluded_from_accounted_ms() {
        let mut tl = StageTimeline::new();
        tl.apply_tick(vec![begin("prep")]);
        tl.apply_tick(vec![end("prep", 100)]);
        tl.apply_tick(vec![begin("backend")]); // still running
        assert_eq!(accounted_ms(&tl.stages), 100);
    }

    /// Builds a timeline with one stage carrying `n` sub-items, the last of
    /// which is still running — the shape a migration batch actually has.
    fn timeline_with_subs(n: usize) -> StageTimeline {
        let mut tl = StageTimeline::new();
        tl.apply_tick(vec![begin("backend")]);
        tl.apply_tick(vec![]);
        for i in 0..n {
            let id = format!("m{i:04}");
            tl.apply_tick(vec![StartupEvent::SubBegin {
                stage: "backend",
                id: id.clone(),
                label: format!("migration {i}"),
            }]);
            // Every sub but the last completes.
            if i + 1 < n {
                tl.apply_tick(vec![StartupEvent::SubEnd {
                    stage: "backend",
                    id,
                    duration_ms: 10,
                    status: StartupStatus::Ok,
                    detail: None,
                }]);
            }
        }
        tl.apply_tick(vec![]);
        tl
    }

    fn labels(rows: &[Row<'_>]) -> Vec<String> {
        rows.iter()
            .map(|r| match &r.kind {
                RowKind::Stage(s) => s.label.to_string(),
                RowKind::Sub(s) => s.label.clone(),
                RowKind::SubTally { done, total } => tally_label(*done, *total),
            })
            .collect()
    }

    #[test]
    fn a_short_sub_list_is_shown_in_full_with_no_tally() {
        let tl = timeline_with_subs(2);
        let rows = visible_rows(&tl.stages, 12);
        assert_eq!(labels(&rows), vec!["backend", "migration 0", "migration 1"]);
    }

    #[test]
    fn a_long_sub_list_collapses_the_older_entries_into_a_tally() {
        // 9 subs: 8 completed + 1 running. Detail keeps the newest two; the
        // tally reports the stage's whole ratio, so 8 of 9 are done.
        let tl = timeline_with_subs(9);
        let rows = visible_rows(&tl.stages, 12);
        assert_eq!(
            labels(&rows),
            vec!["backend", "8/9 done", "migration 7", "migration 8"],
            "older sub-items must collapse into one tally, not occupy a row each"
        );
    }

    #[test]
    fn the_running_sub_item_survives_summarization() {
        // The whole point of the display: whatever is happening right now
        // must be on screen.
        let tl = timeline_with_subs(20);
        let rows = visible_rows(&tl.stages, 12);
        let running = rows
            .iter()
            .filter_map(|r| match &r.kind {
                RowKind::Sub(s) if s.done.is_none() => Some(s.label.clone()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(running, vec!["migration 19"]);
    }

    #[test]
    fn overflow_keeps_the_newest_rows_not_the_oldest() {
        // The defect this function exists to fix: every platform used to
        // iterate oldest-first and stop at the cap, so the newest rows —
        // including the running one — were the ones dropped.
        let mut tl = StageTimeline::new();
        for stage in ["prep", "backend", "host", "cef_init", "paint"] {
            tl.apply_tick(vec![begin(stage)]);
            tl.apply_tick(vec![]);
        }
        let rows = visible_rows(&tl.stages, 3);
        assert_eq!(labels(&rows), vec!["host", "cef_init", "paint"]);
    }

    #[test]
    fn overflow_never_orphans_a_sub_row_from_its_stage() {
        // ReAgent P1 / codex P2 on PR #3268: trimming the flat row list by
        // count alone could cut inside a stage block, dropping the stage row
        // while keeping its indented children — which then render dangling
        // under whatever stage happened to precede them. Linux's 4-line
        // budget hits this with an ordinary prep -> backend(migrations) ->
        // host sequence.
        let mut tl = timeline_with_subs(5); // "backend" + 5 subs
        tl.apply_tick(vec![begin("host")]);
        tl.apply_tick(vec![]);
        let rows = visible_rows(&tl.stages, 4);

        assert!(
            matches!(rows[0].kind, RowKind::Stage(_)),
            "the first visible row must be a stage row, never an orphaned child"
        );
        // Every indented row is preceded, somewhere above, by a stage row.
        let mut seen_stage = false;
        for row in &rows {
            match row.kind {
                RowKind::Stage(_) => seen_stage = true,
                _ => assert!(seen_stage, "an indented row appeared before any stage row"),
            }
        }
    }

    #[test]
    fn a_partially_fitting_stage_keeps_its_header_over_its_oldest_child() {
        // When only part of a block fits, the stage row is mandatory and the
        // rows given up are its oldest (the tally first, then older detail).
        let tl = timeline_with_subs(5);
        let rows = visible_rows(&tl.stages, 2);
        assert_eq!(labels(&rows), vec!["backend", "migration 4"]);
    }

    #[test]
    fn a_zero_budget_yields_no_rows_rather_than_panicking() {
        let tl = timeline_with_subs(5);
        assert!(visible_rows(&tl.stages, 0).is_empty());
    }

    #[test]
    fn stages_are_not_indented_but_subs_and_tallies_are() {
        let tl = timeline_with_subs(9);
        let rows = visible_rows(&tl.stages, 12);
        let indents: Vec<bool> = rows.iter().map(|r| r.indented).collect();
        assert_eq!(indents, vec![false, true, true, true]);
    }

    #[test]
    fn hold_duration_shortens_for_a_very_fast_start() {
        std::env::remove_var("AGENTMUX_SPLASH_HOLD_MS");
        assert_eq!(hold_duration(100), Duration::from_millis(1000));
        assert_eq!(hold_duration(5000), Duration::from_millis(2000));
    }
}
