# SPEC: Browser Pane Lifecycle — Automated Test Coverage

Status: draft
Date: 2026-04-18
Owner: AgentA
Follows: `SPEC_BROWSER_PANE_LIFECYCLE.md` (lifecycle design)
Motivation: three lifecycle bugs in two days, each reproduced only by a human
clicking "close" on a portable build, each root-caused from a single
host-log trace. No test has ever caught a pane lifecycle regression.

## 1. Goals

1. **Catch regressions before they ship.** Every change to `browser_panes.rs`,
   the pane-related part of `client.rs`, `browser-model.ts`, or
   `browser-view.tsx` should be exercised by tests that run in CI before a
   build is packaged.
2. **Make the state machine the source of truth.** Tests prove the transition
   table in `SPEC_BROWSER_PANE_LIFECYCLE.md` §6 is what the code actually
   implements. Future refactors that touch the state machine must update
   tests alongside code.
3. **Test the observable behavior, not the implementation.** A test for
   "close pane → app stays alive" must fail when the behavior is broken
   even if the code path that achieves it changes (force_close, DestroyWindow,
   graceful, native close). No brittle asserts on call-counts to specific CEF
   APIs.

Non-goals: simulating Chromium's internal widget tree, GPU compositor, or
site-isolation process model. Those belong in an end-to-end harness (§6).

## 2. Test levels

Four layers, fastest to slowest:

| Layer | Runs where | Runtime | What it covers | Blocks merge? |
|-------|-----------|---------|----------------|---------------|
| **L1: Rust state unit** | `cargo test` in `agentmux-cef` | <1 s | `BrowserPaneManager` transitions, label sequencing, counter invariants — no CEF, no threads | **yes** |
| **L2: Frontend unit** | `npm test` (vitest) | <5 s | `BrowserViewModel.closed` gating, `BrowserViewComponent` `onCleanup` ordering — no CEF, no real DOM | **yes** |
| **L3: Rust integration (mocked CEF)** | `cargo test --features test-mocks` | <5 s | Close cascade scenarios with a fake `Browser`/`BrowserHost` trait, fake `AppState` lock patterns | **yes** |
| **L4: CEF smoke** | manual or nightly | 30 s–2 min | Real CEF binary starts, opens a pane, closes it, asserts process stays alive | **no** (too flaky for pre-merge) |

**Pre-merge gate** = L1 + L2 + L3 green. L4 runs nightly and on demand.

## 3. Layer 1 — `BrowserPaneManager` state machine

File: `agentmux-cef/src/browser_panes.rs` (add `#[cfg(test)] mod tests` at
bottom).

### 3.1 Test seam

Today `BrowserPaneManager::close()` calls `state.browsers.lock().get(&label)`
and `browser.host().close_browser(0)`. Both are unmockable. Extract a thin
trait so tests can inject behavior:

```rust
pub trait PaneCefBridge: Send + Sync {
    fn browser_lookup(&self, label: &str) -> Option<MockBrowser>;
    fn close_browser(&self, browser: &MockBrowser, force: i32);
    fn load_url(&self, browser: &MockBrowser, url: &str);
    fn set_focus(&self, browser: &MockBrowser, focus: bool);
    fn notify_resize(&self, browser: &MockBrowser);
}
```

`BrowserPaneManager` becomes generic over this bridge. Production wires the
real `AppState` + `cef::Browser`; tests pass a `Vec<Event>`-recording bridge.

### 3.2 Tests to write (retrospective — each maps to a bug we already hit)

| # | Test name | What it proves | Would have caught |
|---|-----------|----------------|-------------------|
| 1 | `close_flips_to_closing` | After `close(block_id)`, subsequent `focus/resize/navigate/go_back/go_forward/reload` are no-ops (bridge records nothing) | Stale-IPC focus race (v0.33.250) |
| 2 | `drain_removes_entry` | After `close → drain_closed_label(label)`, `panes` is empty for that block_id | Drain-label mismatch |
| 3 | `drain_decrements_cascade_counter` | `PANE_CLOSE_IN_PROGRESS` returns to 0 after drain | Counter wedge at high value |
| 4 | `drain_saturates_on_underflow` | Calling drain twice (or with no matching label) doesn't underflow the counter | Spurious-drain edge case |
| 5 | `recreate_while_closing_rejected` | `create(block_id, ...)` returns Err while the same block_id is in Closing state | **Reagent review on #430** — deterministic-label collision |
| 6 | `recreate_after_drain_succeeds` | After drain, `create(block_id, ...)` succeeds and the new entry has a fresh label (seq+1) | Label-reuse bug |
| 7 | `concurrent_close_dedupes` | Two threads calling `close(block_id)` simultaneously only increment the cascade counter once | Double-decrement risk |
| 8 | `navigate_on_existing_live_loads_url` | `create(block_id, url2)` while Live re-navigates instead of creating a new pane | Regression of the "fast re-navigate" path |
| 9 | `focus_after_close_is_noop` | `focus()` called after `close()` does not call `set_focus` on the bridge | Stale hover IPC during teardown |
| 10 | `label_seq_monotonic` | Three creates in a row produce labels ending `-1`, `-2`, `-3` | Label collision under rapid create/close cycling |

### 3.3 Style

- Pure synchronous code in tests — spawn real threads only for #7.
- No `tokio::test`, no `#[allow(clippy::await_holding_lock)]`, no real CEF.
- Each test <20 lines.
- Fixtures live in `browser_panes/testutil.rs` (mock bridge + builder).

## 4. Layer 2 — Frontend state gating (vitest)

File: `frontend/app/view/browser/browser-model.test.ts` (new).

### 4.1 What we test

`BrowserViewModel` owns observable state (URL, title, loading, error) and
methods that issue IPC side-effects. Today's `dispose()` flips `_closed`,
and `navigate / goBack / goForward / reload / giveFocus` gate on it.

Mock `invokeCommand` and `RpcApi.SetMetaCommand`. Assert behavior from
outside.

| # | Test name | What it proves | Would have caught |
|---|-----------|----------------|-------------------|
| 1 | `closed_false_after_construction` | Fresh VM is not marked closed | Baseline |
| 2 | `dispose_flips_closed` | `vm.dispose()` → `vm.closed === true` | Previous empty `dispose()` no-op |
| 3 | `navigate_after_dispose_is_noop` | `vm.navigate(url)` after dispose does not call SetMeta, does not change URL | Navigate-after-close crash path |
| 4 | `giveFocus_after_dispose_returns_false` | post-dispose `vm.giveFocus()` returns false, no IPC | Hover-after-close SetFocus on dying HWND |
| 5 | `goBack_after_dispose_noop` | same | Back button clicked during teardown |
| 6 | `goForward_after_dispose_noop` | same | Forward during teardown |
| 7 | `reload_after_dispose_noop` | same | Reload during teardown |
| 8 | `history_not_mutated_after_dispose` | `vm.dispose(); vm.navigate(url)` does not push to history | Stale history pollution |

### 4.2 Component test — `BrowserViewComponent`

File: `frontend/app/view/browser/browser-view.test.tsx` (new). Use Solid's
test utils + a minimal DOM (happy-dom). Mock `invokeCommand`.

| # | Test name | What it proves | Would have caught |
|---|-----------|----------------|-------------------|
| 1 | `cleanup_fires_close_before_disconnecting_observers` | Close IPC is invoked BEFORE ResizeObserver.disconnect and clearInterval | Close-ordering regression |
| 2 | `hover_after_closed_does_not_fire_focus_ipc` | Setting `model.closed = true` then dispatching mouseenter on placeholder → no IPC | Hover-after-close race |
| 3 | `syncPosition_gated_on_closed` | Calling `syncPosition` after `model.closed = true` → no resize IPC | Stale ResizeObserver tick |
| 4 | `paneCreated_gates_close_ipc` | If `paneCreated === false` (create failed), cleanup does NOT fire browser_pane_close | Double-close on never-created pane |

## 5. Layer 3 — Rust integration with mocked CEF (cascade scenarios)

File: `agentmux-cef/tests/pane_lifecycle_integration.rs`.

These exercise the FULL interaction between `BrowserPaneManager`, the
`PANE_CLOSE_IN_PROGRESS` counter, and a fake `AgentMuxHandler` whose
`do_close`/`on_before_close` method set is the real one from `client.rs`.
The fake closes don't go through CEF — we call `do_close` / `on_before_close`
directly from the test, simulating the sequence a real CEF teardown would
generate.

### 5.1 Scenarios (each becomes one test)

| # | Scenario | What it proves | Would have caught |
|---|----------|----------------|-------------------|
| 1 | `pane_close_alone_leaves_main_alive` | Call `panes.close(block_id)` → simulate pane's `on_before_close(pane_browser)` → assert main handler's `on_before_close` was NOT called | Baseline |
| 2 | `main_do_close_during_pane_close_is_cancelled` | Counter > 0 → call main handler's `do_close(main_browser)` → returns true, main not unregistered | **The cascade guard (v0.33.252)** |
| 3 | `pane_do_close_during_pane_close_is_allowed` | Counter > 0 → call pane handler's `do_close(pane_browser)` → returns false, pane tears down | **THE BUG WE'RE HITTING NOW** — v0.33.252 silently cancels the pane too, orphaning the browser content. This test fails on current HEAD. |
| 4 | `drain_restores_counter` | After drain, counter is 0 — a subsequent real main close is allowed | Counter-wedge regression |
| 5 | `two_concurrent_pane_closes_counter_hits_2_then_0` | Two panes closed simultaneously; both drains must fire before counter reaches 0 | Double-decrement bug, early-unguard |
| 6 | `user_main_close_while_no_panes_quits` | No panes in flight → main `do_close` returns false, on_before_close fires, quit_message_loop called (via recorded hook) | Permanent-unclosable-main regression |
| 7 | `user_main_close_while_pane_closing_is_cancelled_once` | Acknowledged limitation — this test documents the trade-off from the commit message ("user will re-press close"). Makes it intentional. | Surprising UX change |

### 5.2 Test seam for `AgentMuxHandler`

`quit_message_loop()` is a FFI call we can't intercept mid-test. Wrap it:

```rust
// client.rs
pub(crate) fn do_quit_message_loop() {
    #[cfg(not(test))]
    cef::quit_message_loop();
    #[cfg(test)]
    tests::record_quit();
}
```

Same pattern for `backend_close_window` (spawns a thread — tests replace
with a recording closure), `emit_event_all_windows`, and `set_window_icon`.
Keep the production call paths unchanged; tests flip the cfg.

## 6. Layer 4 — CEF smoke test (manual/nightly)

Too flaky to block merges, but valuable as a real-world check. Design:

1. A tiny test harness binary (`agentmux-cef-smoke`) that launches the real
   agentmux-cef in a subprocess with `AGENTMUX_SMOKE_TEST=1`.
2. Host binary reads that env var on startup and, after init, posts a
   scripted sequence to the CEF UI thread:
   - Create a browser pane pointing at `about:blank`.
   - Wait for `on_after_created` (200 ms timeout).
   - Call `browser_panes.close(...)`.
   - Wait for `drain_closed_label` callback (500 ms timeout).
   - Assert host process is still running (browser_list.len() > 0 for main).
3. Write a JSON result to stdout; exit 0 on pass, 1 on fail.
4. Nightly CI job runs the smoke test on Windows; failure opens an issue.

This is the only test that can ever catch a real Chromium cascade regression.
Keep it small and stable.

## 7. Infrastructure we need to add

| Item | Where | Effort |
|------|-------|--------|
| `PaneCefBridge` trait + `BrowserPaneManager` generic refactor | `browser_panes.rs` | 1 day |
| Mock bridge (`testutil.rs`) | `browser_panes.rs` | 2 h |
| `#[cfg(test)]` shims for `quit_message_loop` / `backend_close_window` / `emit_event_all_windows` | `client.rs` | 1 h |
| vitest setup for `@/app/view/browser` | `vitest.config.ts` (already present) | 0 |
| Solid test utils + happy-dom | `package.json` dev-dep | 30 min |
| CI wiring (fail build on L1/L2/L3 red) | `Taskfile.yml` + `package.json` | 1 h |
| `agentmux-cef-smoke` harness binary | new crate in workspace | 3 days (nightly value, lower priority) |

## 8. Bugs this coverage would have caught retrospectively

Cross-ref against the last two days of lifecycle work:

| Bug | Discovered how | Caught by test # |
|-----|---------------|------------------|
| Stale IPC focus/resize after close (theorized race #2 in spec) | Code review of spec | L1 #1, L2 #3, L2 #4 |
| Deterministic-label collision on recreate | Reagent review on #430 | L1 #5, L1 #6 |
| Dual-decrement if drain fires twice | Self-review | L1 #4, L3 #4, L3 #5 |
| Cascade fires main do_close on pane close | Host log trace | L3 #1, L3 #2 |
| **Cascade guard silently cancels the pane's do_close too** (v0.33.252 orphan) | **User report right now** | **L3 #3** |

Five bugs, zero caught before shipping. With this spec implemented, #3–#5
fail in CI before a build leaves the machine.

## 9. Implementation order

1. **L2 first** (1 day) — vitest harness for BrowserViewModel + component.
   Gives immediate value with zero Rust refactoring risk. Lands before any
   more lifecycle changes.
2. **L1 next** (2 days) — state machine tests after the `PaneCefBridge`
   extraction. Forces us to write test-friendly code in `browser_panes.rs`.
3. **L3 third** (2 days) — integration tests, requires the `#[cfg(test)]`
   shims in `client.rs`. This is the layer that would have caught today's
   "cancel the pane too" bug — write it before shipping the next lifecycle
   change.
4. **L4 deferred** — only pursue after L1–L3 are stable and wired into the
   pre-merge gate.

## 10. Open questions

- **Should `BrowserPaneManager` own `AgentMuxHandler` state?** Today the
  counter is a free-standing static. If `BrowserPaneManager` held a
  reference to the handler registry, tests could inject easier. Trade-off:
  more coupling for testability. Probably yes, but revisit once L1/L3 are
  in place.
- **Do we need property-based tests?** `proptest` on the state machine
  (random sequences of `create/navigate/close/drain/focus`) would catch
  invariant violations we can't anticipate. Low priority until the above
  example-based tests are landing.
- **CEF smoke test on macOS/Linux?** Browser panes are Windows-only today
  (`#[cfg(target_os = "windows")]` gates the HWND work). Smoke harness
  stays Windows-only until we have pane support elsewhere.

## 11. Not in scope

- Unit tests for the CEF `BrowserView` delegate — that code is CEF-internal
  glue, covered by L4 only.
- Tests for `client.rs` main-window lifecycle unrelated to panes (multi-window
  taskbar grouping, FullInstance/Subwindow). Separate spec.
- Frontend e2e (`task dev` + Playwright) — worth doing for the whole app
  but out of scope for the pane-specific case.
