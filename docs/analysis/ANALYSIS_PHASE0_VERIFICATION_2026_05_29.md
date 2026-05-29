# ANALYSIS — Input-first Phase 0 verification + bench harness

**Date:** 2026-05-29
**Author:** AgentX
**Context:** Phase 0 of the input-first execution plan ([discussion #1161](https://github.com/agentmuxai/agentmux/discussions/1161)). Records the bench-harness scaffolding (0.1, shipped here) and the verification findings for the 0.3 "cheap wins" — several of which turn out to need **runtime/visual confirmation on the running app**, i.e. they are past the autonomous boundary.

---

## 0.1 — Bench harness scaffolding (shipped in this PR)

The existing benches (`bench-agent-keystroke.mjs`, `bench-term-echo.mjs`, PR #1150) are single-run with a hard absolute threshold (`fail if P95 > 50 ms`). On variable hardware that gate is flaky → gets disabled → theater. This PR adds the statistical layer the reviewers converged on, **without modifying the benches**:

- **`tools/tests/lib/bench-stats.mjs`** — pure functions: percentiles, multi-run aggregation, run-to-run variance (CoV), and a **delta-vs-baseline verdict** (`pass` / `regress` / `improve` / `no-baseline` / `noisy`) with a reporting-vs-gate exit-code policy. Unit-tested in **`bench-stats.test.mjs`** (12 tests, `node --test`, no app required — all green).
- **`tools/tests/bench-aggregate.mjs`** — runs a chosen bench N times, extracts each run's headline metric (agent → `keystroke.stats.p95`, term → `quiet.p95` — the always-present term metric; `busy`/`stream_*` only exist with `--busy`/`--stream`), aggregates, compares to a committed baseline, and prints a REPORT/GATE verdict. `--update-baseline` captures one.
- **`tools/tests/baselines/`** — schema + README documenting the pinned-device requirement and the reporting→gating promotion path.
- **`.github/workflows/input-bench-report.yml`** — `workflow_dispatch`, self-hosted `input-bench` runner, reporting-mode. Also runs the stats unit tests (which *do* work on any runner).

### ⛔ Manual-benchmark boundary (needs you)

Everything that produces *real numbers* needs the pinned hardware:

1. **Stand up self-hosted `input-bench` runner(s)** — a low-end Windows box (worst realistic user machine) and a macOS box, each running AgentMux with the pane open and CDP reachable. Document specs in `baselines/README.md`.
2. **Capture baselines** with `--update-baseline` on each device; commit the JSON.
3. **Characterize variance** over a few weeks of reporting runs (confirm CoV < 0.25).
4. **Promote to gate** (`--mode gate`) once stable — gate the low-variance synchronous-body P95 before the noisier end-to-end metric.

The statistical core, aggregation, baseline-delta logic, and CI wiring are done and verified; only the data collection is manual.

---

## 0.3 verification findings

### 0.3a — Browser-pane keydown handler → **needs runtime verification (likely a non-goal as stated)**

**Verified (code read):** only the address bar (`frontend/app/view/browser/browser-view.tsx:421`, `handleAddressKeyDown`) and the auth modal (`components/BrowserAuthModal.tsx`) have `onKeyDown`. `browser-model.ts` defines **no** `keyDownHandler` on the view model, so `appHandleKeyDown` (`store/keymodel.ts:412`) finds no block handler for a focused browser pane.

**But:** a browser pane's content is a native `CefBrowserView` child window. When it has focus, keystrokes go to Chromium's browser view — DOM `keydown` very likely **never reaches** the host SolidJS `appHandleKeyDown` at all. So adding a `keyDownHandler` to `browser-model.ts` may never fire, and intercepting at the host could *break* native input (find-in-page, text fields inside the page). The critique flagged exactly this: "every surface must have a handler" is an invariant inversion — the goal is fast input, not uniform handlers.

**Recommendation:** **Verify on the running app first** — focus a browser pane's content, press Ctrl+L / Ctrl+T / Ctrl+F, and observe whether keys are lost or handled by Chromium. Only if genuinely lost (and only for the specific app-level shortcuts) add a narrowly-scoped host handler. Treat "input goes straight to the native view" as compliant-by-absence otherwise. **Deferred pending that runtime check.**

### 0.3c — `block.scss` `backdrop-filter: blur` audit → **partly a compositing hack; do NOT blindly gate**

**Verified (code read of `frontend/app/block/block.scss`):**

| Line | Blur | Nature |
|---|---|---|
| 313 | `blur(var(--magnified-block-blur))` | **Visual** — magnified-block backdrop. Candidate typing-time offender. |
| 336 | `blur(50px)` | **Visual** — `.connstatus-overlay` backdrop (rule at block.scss:324), not the magnified-block backdrop. |
| 405 | `blur(8px)` | **Visual** — overlay backdrop. |
| 455 | `blur(0.1px)` | **NOT visual — a compositing-order hack.** Comments at 450-452/485-490 explain `backdrop-filter` is used *deliberately* to force this element to composite **above** xterm so the focus ring is visible. |

So the critic's "broader than trivial" warning is confirmed: gating *all* `backdrop-filter` behind `prefers-reduced-transparency` would **break focus-ring visibility over xterm** (the `blur(0.1px)` hack). The legitimate perf target is the *visual* blurs (313/336/405) that re-raster per frame while typing over them — and even those need profiling to confirm they're on the typing path, plus a visual check that gating them doesn't disturb layering.

**Recommendation:** Treat only the **visual** blurs (313/336/405) as candidates; **never touch the `0.1px` compositing hack** (455). Land a gate behind `prefers-reduced-transparency` only with (a) a profile showing per-frame re-raster during typing and (b) a visual confirmation focus rings/magnified blocks still render. **Deferred pending profile + visual check.**

### 0.3b — Region skip-if-unchanged + HWND cache → **design + hazard documented; needs runtime test**

**Verified:** `agentmux-cef/src/browser_panes.rs:476` `set_pane_overlay_clip` rebuilds and applies a `SetWindowRgn` to every pane HWND on each call, with no last-region cache. It already does the right async-paint thing (`bRedraw=FALSE` + `InvalidateRect`, #1097 fix #5) and the renderer already short-circuits when no pane intersects (`pane-overlay.ts`).

**The hazard the "stateless cache" framing hides:** a per-HWND cache keyed on the HWND pointer is **not** safe-by-construction — Windows recycles HWND values, so a destroyed-then-recreated pane could collide with a stale cache entry and get its clip **wrongly skipped** → an airspace regression (overlay shows over an unclipped pane, or pane content stays hidden). Correctness requires invalidating the cache entry on pane close/drain (there are two close paths: `drain_closed_label` and the explicit `close()`), which adds state and a real regression surface.

**Recommendation:** Implement with the cache keyed on HWND **plus** explicit eviction on both close paths, *or* a genuinely stateless check (re-derive and compare the intended region to what's applied). Either way the failure mode is **visual** (airspace tearing / missing clip) and must be **runtime-tested** on Windows — open menus/tooltips over browser panes during rapid hover and confirm no clip is dropped. **Deferred pending that implementation + runtime test** (a separate Rust PR). Do **not** add a host-side per-frame coalescer (a host timer racing DWM can re-introduce tearing — keep coalescing in the renderer rAF).

---

## Net

- **Autonomous + shipped:** the bench harness statistical core + aggregator + baseline schema + reporting CI + unit tests (this PR), and the keydown-IPC guard (PR #1174, merged).
- **Handoff (manual/runtime):** capture bench baselines on pinned devices; runtime-verify the browser-keydown gap; profile+visually-verify the blur gate; implement + runtime-test the region cache with close-path eviction.
