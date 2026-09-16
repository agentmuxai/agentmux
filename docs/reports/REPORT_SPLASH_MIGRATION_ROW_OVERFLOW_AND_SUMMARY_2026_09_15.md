# REPORT — Splash migration rows: too many detailed entries, and overflow drops the newest one instead of scrolling to it

**Date:** 2026-09-15
**Scope:** The per-migration sub-rows added by `feat(srv,launcher): show real
per-migration progress on the splash instead of a silent clock (#3223)`
(commit `e03678a39`), and how all three platform splash renderers
(`agentmux-launcher/src/splash.rs` [Windows], `splash_mac.rs` [macOS],
`splash_linux/mod.rs` [Linux]) display a stage's `subs` list once it's
longer than the fixed row budget.
**Not in scope:** the broader splash timing/DRY rethink already covered by
`docs/reports/REPORT_SPLASH_SCREEN_ARCHITECTURE_RETHINK_2026_09_14.md` — this
report is one specific, newer symptom sitting on top of that architecture,
and reuses its file references rather than re-deriving them.

---

## 1. The ask, restated

> "there are too many detailed entries, we just need summaries over them...
> if the list overflows, it needs to start a scroll with the latest entry
> keeping the latest entry in view"

Two distinct changes:

1. **Don't render one permanent row per migration.** #3223 wires each
   individual DB migration's begin/end into a `sub_begin`/`sub_end` pair on
   the already-open "backend" stage (`agentmux-launcher/src/srv_spawner.rs`,
   `agentmux-srv/src/migrations/runner.rs:296,299`), so a batch of pending
   migrations produces one sub-row each. That's the right level of detail
   for a *single* slow migration (the feature's actual goal — see #3223's
   commit message) but not for a batch of many.
2. **When the row list is longer than the visible area, keep the newest
   entry in view (scroll), not drop it.**

## 2. Why the row count can get large: 34 migrations can run in one batch

The registry currently has 34 migration modules
(`agentmux-srv/src/migrations/m0000_bootstrap.rs` through
`m0033_narrow_skill_global_uniqueness_index.rs`). `run_pending_migrations`
runs every migration the local DB hasn't applied yet in a single call
(`agentmux-srv/src/migrations/runner.rs:264`), and #3223 emits a
`sub_begin`/`sub_end` pair for each one. A data dir that is several
migrations behind — a fresh install, a long-idle machine, or a per-build
isolated channel (`task package`'s per-build data dirs, see the repo's
`CLAUDE.md` "Data isolation is per-BUILD" section) — can legitimately run
most or all 34 in one boot.

## 3. Current overflow behavior: hard cap, oldest-first, drop-the-newest

All three platforms flatten `StageTimeline.stages` (shared model, added in
PR #3222 / `splash_core.rs`) into a fixed-size row list every frame, then
stop as soon as the cap is hit — none of them scroll:

| Platform | Row budget | Where it stops | Behavior once full |
|---|---|---|---|
| Windows | `MAX_STAGE_ROWS = 12` (`splash.rs:91`) | `splash.rs:515` (stage rows), `splash.rs:545` (sub rows) | `if row >= MAX_STAGE_ROWS { break; }` — iterates stages/subs in arrival order and stops, so rows *after* the 12th (including a still-running migration, and the trailing `total`/`other` annotations at `splash.rs:588,595`) are simply never drawn. |
| macOS | `MAX_STAGE_ROWS = 12` (`splash_mac.rs:103`) | `flatten_rows`, `splash_mac.rs:154` (stage rows), `:180` (sub rows) | Same shape: `if out.len() >= MAX_STAGE_ROWS { break; }`. The `total`/`other` rows are appended last and only added `if out.len() + 2 <= MAX_STAGE_ROWS` / `< MAX_STAGE_ROWS` (`:221,230`) — on a full panel they're the first casualty, then the newest migration rows. |
| Linux | `STAGE_MAX_LINES = 4` (`splash_linux/mod.rs:77`) | `lines()`, `:167` (stage lines), `:177` (sub line) | Much smaller budget, but already mitigates the sub-row problem by design: only `s.subs.last()` is ever shown per stage (`:171`) — one line, always the most recent migration, no history at all. The *stage*-level list (4 lines total, shared across every top-level stage) still has the same drop-the-tail-when-full behavior if there are ever more than ~4 stages in flight. |

The net effect on Windows/macOS: because rows are drawn **oldest-first** and
drawing simply **stops** at the cap, a long migration batch fills the panel
with the *earliest* migrations and never shows the batch's later members —
including whichever one is currently running. That is the opposite of "keep
the latest entry in view": today it keeps the earliest ones and freezes
there, which is exactly the "looks stuck" symptom the 09-14 architecture
report was already about, just from a different cause (a full row budget
rather than a missing tick).

## 4. Aside: the Windows 11 vs. Windows 10 observation

Splash rendering is not branched by Windows version — `splash.rs` is one
binary path for every Windows release. The difference you're seeing is
almost certainly **migration debt, not OS version**: whichever machine
showed many granular sub-rows had more pending migrations left to apply on
that particular boot (further behind, or a fresher/isolated data dir);
the one that showed none was already fully migrated, so `run_pending_migrations`
had zero rows to emit. Worth confirming by checking each machine's applied-
migration count rather than treating this as a platform difference to fix
separately.

## 5. Recommendation

Two changes, matching the two-part ask, both belonging in the shared
`splash_core.rs` (the row *model* is already shared there since PR #3222;
only each platform's own flatten/draw loop is not — see the 09-14 report's
"three reimplementations" finding, which this exact gap reproduces at a
smaller scale):

1. **Summarize instead of enumerating.** Once a stage's `subs` list passes a
   small threshold, collapse the completed ones to a single running tally
   (e.g. "Migrations: 7/34 applied") and keep showing only the
   *currently-running* sub-row (or the most recently finished one, briefly)
   in full detail — rather than a permanent row per migration. This is the
   same shape Linux already accidentally arrived at (`subs.last()`), just
   without the count.
2. **Scroll instead of truncating when still over budget.** For whatever
   row list remains after summarizing (top-level stages, or a sub-list a
   caller deliberately wants shown in full), replace the current
   "stop appending at the cap" logic with a windowed view that keeps the
   *last* N entries rather than the *first* N — i.e. flip
   `if out.len() >= MAX { break }` (drop-newest) to something that retains
   the tail (drop-oldest, or a proper scroll offset if partial-row
   scrolling is wanted). Since all three platforms already rebuild their
   flat row list from scratch every frame from the same shared
   `StageTimeline`, this is a single shared helper in `splash_core.rs` (e.g.
   `visible_rows(stages, budget) -> Vec<Row>`) that each platform's
   existing flatten function calls, instead of three separate fixes to
   `splash.rs`, `splash_mac.rs`, and `splash_linux/mod.rs` — the DRY lesson
   already documented for this codebase applies again here.

Not decided here: the exact summarization copy/threshold (e.g. show the
last 1 vs last 3 sub-rows in detail before collapsing) — a small design
call left for whoever implements this.
