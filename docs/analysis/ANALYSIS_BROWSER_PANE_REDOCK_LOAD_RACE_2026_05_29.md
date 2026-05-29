# Browser-pane "won't load after redock" — root cause + fix

**Date:** 2026-05-29
**Symptom:** After tearing a browser pane off into a floating window and re-docking it, the pane *sometimes* doesn't finish loading — it lands blank / in an error state.

## TL;DR

Redock moves a block (keeping its `block_id`) into the target window while the
floater's old browser pane for the *same* `block_id` is still tearing down. The
target's re-create hits the reducer's `Closing` guard, which **rejects** the
create with *"still closing; retry after on_before_close"* — on the assumption
that *"the frontend retries on next tick."* **That frontend retry was never
implemented**: `browser-view.tsx::createPane` catches the error and calls
`model.onError` (→ `LoadFailed`) with no retry. Whether the pane loads depends
on a race between the floater's teardown drain and the target's re-create —
hence *sometimes*.

## The chain (confirmed by reading source)

1. **Redock** (`agentmux-srv/.../sagas/redock_floating_pane.rs`) **moves** the
   block — same `block_id` — from the floater's tab to the target window's tab.
   The floater then auto-closes (its `tab.blockids` empties).
2. Floater's browser pane for `B` → `on_before_close` → reap →
   `EnqueueBrowserPaneClose` (`Closing`), then `DrainBrowserPaneByLabel` removes
   the entry and emits `BrowserPaneClosed`.
3. Target window re-renders `B` → `browser-view.tsx::createPane` →
   `browser_pane_create` IPC → `BrowserPaneManager::create` →
   `HostCommand::TryRegisterBrowserPaneLive { block_id: B }`.
4. **Race.** If the floater's drain (step 2) hasn't completed, `B` is still
   `Closing`, so `handle_try_register_browser_pane_live` returns
   `RegisterResult::Closing` (`reducer/panes.rs:90`) and
   `BrowserPaneManager::create` returns
   `Err("…still closing; retry after on_before_close")` (`browser_panes.rs:189`).
5. `createPane` does `catch (e) { model.onError(...) }` — **no retry**
   (`browser-view.tsx:197`). Pane stuck blank / `LoadFailed`.
6. If instead the drain wins the race (`B` removed → `Fresh`), the create
   succeeds and the pane loads. → intermittent.

## Reducer-coverage verdict

- **Covered + correct:** the browser-pane create/close lifecycle is fully
  reducer-backed (`HostState.browser_panes`, `Live`/`Closing`). The
  `Closing → reject` rule is *correct* and necessary: without it the floater's
  in-flight `DrainBrowserPaneByLabel` would evict the freshly-created entry
  (drain is label-keyed; create reuses the block_id). So the state machine is
  sound and corruption-safe.
- **The gap:** there is **no deterministic re-create-after-close** at the redock
  seam. The reducer rejects and punts to a frontend retry that doesn't exist —
  a dropped handoff. This is the missing coverage.

## Fix — deterministic host-side pending-create replay

Make the host own the deferred create instead of punting it:

1. Add `AppState.pending_browser_pane_creates: Mutex<HashMap<block_id,
   PendingBrowserPaneCreate>>` (url + rect + window_label).
2. In `BrowserPaneManager::create`, on `RegisterResult::Closing`: **stash** the
   pending create keyed by `block_id` and return `Ok(())` (deferred) instead of
   `Err`. The pane is now Live-pending, not failed.
3. At the close-completion site — where `DrainBrowserPaneByLabel` returns
   `drained_browser_pane_block_id = Some(B)` (the `on_before_close` path) —
   check the pending map for `B`; if present, **replay** the create. The block
   is now absent from `browser_panes`, so `TryRegisterBrowserPaneLive` returns
   `Fresh` and the `CreateBrowserPaneTask` is posted.

This closes the loop on the **deterministic** signal (close-completion /
`BrowserPaneClosed`), not a frontend `setTimeout` retry (which the project's
"no timers / find the deterministic signal" rule forbids anyway), and keeps the
deferred state in the host/reducer layer where the rest of the pane lifecycle
lives.

### Why not a frontend retry
A frontend retry would have to poll or guess timing; the host already knows
*exactly* when the close finishes (it dispatches the drain). Host-side replay is
race-free and single-shot.

## Verification plan
- Unit: reducer already covers `Closing` return (`reducer/tests.rs`); add host
  coverage for stash-on-Closing + replay-on-drain if feasible without CEF.
- Live: repeated tear-off → redock cycles on a browser pane; the redocked pane
  must always finish loading (previously intermittent).
