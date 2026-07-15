# Report: New Window color-flash — live §4.3 measurement (the gap PR #2163 couldn't verify)

**Date:** 2026-07-15
**Type:** Follow-up measurement, no code changed
**Governing report:** `docs/specs/REPORT_NEW_WINDOW_STARTUP_COLOR_FLASH_2026_07_14.md`
(§8's own words: *"this needs a real Windows visual smoke test to confirm,
which wasn't possible from this environment"*)
**Trigger:** User report, the day after PR #2163 shipped: still seeing
roughly five distinct screen colors including a brief white during "New
Window," on a build that includes that fix.

---

## 1. What this report is

PR #2163 shipped three fixes (§4.1 backstop color, §4.2 `on_load_end` gating,
§4.3 a downstream-ordering race in the pool-promote path) and was explicit
that §4.3 — "the dominant cause" per its own ranking, since it's the path a
real "New Window" click almost always takes on a warm app — was the one
fix it had the **least** confidence was fully resolved, specifically because
verifying it needs a live Windows GUI session the authoring environment
didn't have.

This environment does. This report supplies that missing measurement.

## 2. Method

Live dev instance running `bf1b4e64` (includes PR #2163 in full). Triggered
`window.api.openNewWindow()` directly via CDP against the running app (not a
synthetic test), with a timestamp taken immediately before the call, then
read the host log's `pool:new-window`-target lines for the same window
label.

## 3. Measurement

```
18:54:53.511Z  — openNewWindow() called
18:54:53.779Z  — "[pool] CEF Views set_bounds + show on cached Window
                  (macOS-parity, UI thread)"   [target: pool:new-window]
18:54:53.7796Z — "[pool] CEF Views show completed — safe to proceed"
                  elapsed_ms: 2
18:54:53.915Z  — "[pool:new-window] served from pool — skipping cold-path
                  window creation"
```

**268ms** between the trigger and the first evidence that CEF's own Views
compositor was told to show anything. The CEF-side show call itself, once
posted, completes in 2ms (confirming PR #2163's own "safe to proceed"
blocking wait works as designed — the *downstream* actions genuinely wait
for compositor-show completion). The 268ms is entirely upstream of that
wait: it's the native Win32 `SetWindowPos` → `set_taskbar_hidden`
(hide→show cycle) → `ShowWindow` sequence (`window_pool.rs:1330-1362`,
report §4.3 steps 1-3), which runs synchronously and completes before the
CEF Views show task is even posted.

## 4. What this confirms

- **The gap is real, and it's not small.** 268ms is well inside the range a
  human perceives as a distinct flash rather than a single instantaneous
  transition — this is not a 16ms single-frame blip that PR #2163's fixes
  might have already made imperceptible.
- **PR #2163's actual fix (the blocking wait) did what it claimed** — it
  closes a *different* race (downstream actions no longer fire before the
  compositor show starts) — but does not and structurally cannot shrink
  this specific gap, because the gap is entirely in code that runs *before*
  the blocking wait begins.
- **The report's own ranking holds up under live data**: §4.3 is confirmed
  as a real, still-open, measurable gap, distinct from and larger than
  whatever §4.1/§4.2 improved.

## 5. What this report does NOT do

Per explicit decision: **no reorder attempted.** `RESEARCH_CEF_PREWARM_WINDOW_BLANK_ON_WINDOWS_2026_06_21.md`
(cited in the original report) already tried the naive "show CEF Views
before the native Win32 show" reorder once and it caused a worse regression
— a fully blank window, not just a flash — because `set_taskbar_hidden`'s
own hide→show cycle unconditionally re-hides whatever was shown before it,
including a CEF Views window shown early. Any real fix here has to account
for that constraint, not just move lines around; it is exactly the kind of
change that deserves careful, isolated implementation with live telemetry
verification (the same method used in this report) as its acceptance test —
not just code review — given the demonstrated history of this exact class
of reorder backfiring.

## 6. Recommendation for whoever picks this up

- Use this report's method (CDP-triggered `openNewWindow()` + host-log
  `pool:new-window` timestamps) as the acceptance test: a real fix should
  show this gap shrink to something close to the CEF-side show's own 2ms,
  not just "looks better."
- Read `RESEARCH_CEF_PREWARM_WINDOW_BLANK_ON_WINDOWS_2026_06_21.md` before
  touching `window_pool.rs:1330-1412` — it documents exactly why the
  current (still-gapped) ordering exists and what naive reorder to avoid.
- Consider whether the 268ms gap can be *masked* rather than *closed* as a
  lower-risk interim step — e.g., keeping the in-page splash
  (`#startup-loading`) painted at the CEF-compositor level (not just DOM)
  through this specific window, if CEF exposes a way to hold a composited
  frame during a Views-window show. Not investigated here; flagged as an
  alternative worth scoping before committing to the native reorder.

## 7. References

- `docs/specs/REPORT_NEW_WINDOW_STARTUP_COLOR_FLASH_2026_07_14.md` (the
  governing investigation; §4.3, §7, §8 are what this report follows up on)
- `docs/research/RESEARCH_CEF_PREWARM_WINDOW_BLANK_ON_WINDOWS_2026_06_21.md`
  (why the current non-atomic ordering exists; read before attempting any
  reorder)
- `agentmux-cef/src/commands/window_pool.rs:1330-1412` (the code this
  measurement is about)
- `agentmux-cef/src/ui_tasks/pool.rs:1-81` (the blocking-wait mechanism
  PR #2163 added, confirmed working correctly by this measurement)
