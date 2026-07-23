# Retro: macOS splash screen rows still don't "count up"

**Date:** 2026-07-22
**Severity:** Low (cosmetic — no functional impact on startup)
**Area:** `agentmux-launcher` macOS native splash (`splash_mac.rs`)
**Related PR:** #2244 (`feat(launcher): macOS splash — add an "other" row closing the total-vs-items gap`, merged 2026-07-20, commit `27d45f5d`)
**Status:** Root-caused (again) — not fixed. This retro exists because the earlier investigation's conclusion ("deferred") wasn't visible enough for Asaf to know the count-up bug was never in scope for #2244.

---

## 1. What the user saw

> "i just loaded it, and I notice the splash screen still has parts that dont count up .. what happened to that work?"

The expectation, reasonable from the outside: PR #2244 touched the splash screen's row rendering, so it should have fixed the rows that don't animate. It didn't — because it was never meant to. It fixed a *different*, narrower complaint.

---

## 2. What #2244 actually shipped

The ask behind #2244 was that the splash's "total" row and the sum of the individual stage/sub rows didn't add up — there was unaccounted time with no row explaining where it went. The fix, in `flatten_rows()` (`splash_mac.rs`, current lines ~284–309):

```rust
let accounted_ms: u64 = stages
    .iter()
    .filter_map(|s| s.done.as_ref().map(|(dur, _, _)| *dur))
    .sum();
let other_ms = ms.saturating_sub(accounted_ms);
if out.len() + 2 <= MAX_STAGE_ROWS {
    out.push(FlatRow {
        indented: false,
        label: String::new(),
        time_text: format!("other: {}", format_ms(other_ms)),
        label_color: (SUB_R, SUB_G, SUB_B),
        time_color: (SUB_R, SUB_G, SUB_B),
        ..
    });
}
```

This whole block runs only inside `if let Some(ms) = total_ms { ... }` — and `total_ms` is only `Some` once `ready_at` is set, i.e. once the host has already signaled first paint and startup is effectively over. So the "other" row is computed **once**, from already-final numbers (`total_ms - accounted_ms`), and appended already at its final value. It was never designed to animate — it has no `started_at`, no "running" state, nothing to count from. Same pattern the pre-existing "total" row already used.

The same PR added `docs/analysis/ANALYSIS_SPLASH_SCREEN_TIMING_2026_07_20.md`, which investigated the *count-up* behavior as background research while diagnosing the total-vs-items gap — but the doc is explicit that this was investigation only:

> "Not implemented here — investigation only, per the ask."

In other words: the count-up problem was found, written up, and consciously left out of #2244's scope on the same day it was documented. That framing didn't carry forward into how the PR was described/merged, so it read from the outside as "the splash counting work," when it was really "the splash total-accounting work."

---

## 3. Why rows don't count up — the actual mechanism

None of this is new to this retro; it's a restatement of `ANALYSIS_SPLASH_SCREEN_TIMING_2026_07_20.md` §2, reverified against current `main` (`c8f141e5`, v0.54.3) — nothing has touched `splash_mac.rs` since #2244.

Each row's live time comes from `StageRow`/`SubRow` (lines ~126–133): a `started_at: Instant`, and `done: Option<(u64, StartupStatus, Option<String>)>` that's only set once the matching `StageEnd`/`SubEnd` event arrives. `flatten_rows()` recomputes every row on every tick: while `done` is `None` it renders live elapsed time (`format_running(started_at)`, blue); once `done` it freezes on the final duration (green).

The render loop (`run_until_dismissed()`, ~line 685) only *ticks* — drains the event channel and redraws — on a fixed ~8–24ms cadence, and only before `ready_at` fires. So "counting up" is not sourced from a real progress stream with item counts; it's a side effect of how many ticks land between a step's Begin and End events:

- If a step's real work finishes fast enough that its `StageBegin` and `StageEnd` land in the channel before the splash's next drain, they're processed back-to-back in one tick — the row is created **already done**. No "running" frame is ever painted, so it just snaps to its final value.
- If the Begin→End gap spans more than roughly one tick, at least one intermediate `format_running` frame gets drawn — that's the visible count-up.

There is no minimum-duration threshold anywhere in the crate forcing a step to show at least one running frame. It's a pure emergent effect of tick cadence vs. how fast each startup step happens to run — not a deliberate on/off per row. That's consistent with what "parts that don't count up" looks like from the outside: fast steps (and the two structurally-static rows, "total" and "other") don't animate; slower steps do.

A second, related defect from the same analysis: a row whose `End` event never arrives before `ready_at` fires gets frozen forever at whatever the one forced final-refresh produced — stuck in the "in progress" (blue) color, never turns green, never updates again.

---

## 4. Root cause, one line

**The count-up bug and the total-vs-items-gap bug are two independent defects that happen to live in the same function (`flatten_rows`) and the same PR's diagnosis pass. #2244 fixed the gap; the count-up race (tick cadence vs. event-arrival timing in `run_until_dismissed`) was identified, documented, and explicitly deferred the same day — and nothing since has picked it back up.**

---

## 5. Why this wasn't caught sooner

- The deferral was recorded in an analysis doc (`docs/analysis/...`), not in a follow-up task, issue, or a "known limitation" note in the PR description itself — so there was no forcing function to come back to it.
- The PR title/description read as "splash screen work" broadly, which made "did that fix the counting?" a reasonable but wrong inference from the outside.
- No `docs/retro/` entry existed for the splash counters before now — this is the first.

---

## 6. If we want to actually fix it (not implemented here)

Per `ANALYSIS_SPLASH_SCREEN_TIMING_2026_07_20.md` §7, two concrete options, neither implemented:

1. **Insert a minimum artificial delay** before processing a same-tick Begin+End pair, so every row gets at least one "running" paint frame regardless of how fast the real work was. Simple, but adds latency to an already latency-sensitive startup path — needs a small enough constant (~1 tick) to stay invisible.
2. **Decouple rendering from the drain loop** — redraw on a fixed timer independent of `try_recv`, so a fast item's single tick-worth of "running" state still gets painted before the End event is processed. More correct, more invasive (event draining and repaint currently share one loop).

Either fix should also close the "row frozen forever if End never arrives before `ready_at`" edge case, since both are symptoms of the same drain-loop-vs-wall-clock coupling.

**Recommendation:** treat this as a real (if low-severity) follow-up ticket, not just an analysis doc — that's the gap that let it silently ride along on #2244 without becoming a decision anyone signed off on.

---

## 7. Timeline

- **2026-07-20** — #2244 opened/merged (commit `27d45f5d`): adds the static "other" row, closing the total-vs-sum-of-items gap. Same PR adds `ANALYSIS_SPLASH_SCREEN_TIMING_2026_07_20.md`, which investigates and documents the separate count-up race but explicitly defers a fix.
- **2026-07-22** — Asaf loads a fresh build, notices rows still don't count up, asks what happened to "that work." This retro written to make the scope split (gap-fix vs. count-up) and the deferred status explicit and discoverable.
