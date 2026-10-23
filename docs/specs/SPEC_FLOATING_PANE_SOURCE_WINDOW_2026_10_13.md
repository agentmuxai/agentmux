# Spec: "Open in Floating Pane" knows which window it came from

**Date:** 2026-10-13
**Status:** proposed
**Plan item:** #9 of `PLAN_RENDERER_MEMORY_FIXES_2026_10_12.md`. The plan's one-line description ("they don't follow the main window on minimize or close") was wrong; see §2.

## 1. What was seen

The CEF host logged "[floating-pane] could not resolve parent main HWND; floater will not cascade on minimize/destroy (orphan risk)" for **21 of 42** floating panes opened from the widgets bar in a lab benchmark.

## 2. What's actually wrong

**The warning's text is stale.**
- Floaters no longer cascade with a parent: the hook passes messages through, by design ("FLOATER INDEPENDENCE", `crates/cef/src/client/wndproc.rs:368-379,408-414`; `floating_pane.rs:98-101`).
- The parent HWND it looks for is stored (`ACTIVE_FLOATER_HWNDS`) but read by nothing.
- Floaters close with the *last* top-level window, whatever parent was recorded (invariant FP-LIFE, `docs/architecture/ARCHITECTURE_FLOATING_PANE_DOCKING_2026_05_30.md` §1.1, `reducer/quit.rs:120-122`).

The warning points at three real problems, though.

**1. The widgets bar doesn't say which window it's in.**
- `action-widgets-menu.ts:56` calls `pane.open` with `floating: true` and **no `tab_id`**.
- srv then falls back to the active tab of the *first workspace in store order* (`crates/srv/src/server/app_api/pane.rs:1109-1119` → `app_api/mod.rs:216-235`), not necessarily the window the user clicked in.
- The floating pane's `openfloatingpane` event then goes to that workspace's window (`pane.rs:192-210`). That window places the floater centred on itself (`frontend/app/store/global.ts:385-390`).
- **With two windows open, a floating pane can appear centred on the other window.**

**2. The frontend never sends `source_window_label` for this path.**
- The `openfloatingpane` handler (`frontend/app/store/global.ts:375-403`) calls `openFloatingPane` without it.
- Drag tear-offs do send it (`CrossWindowDragMonitor.win32.tsx:396,423,448`, `pane-tab-tearoff.ts:183`).
- So the host always takes its fallback.

**3. The host's fallback picks from stale records.**
- `floating_pane.rs:182-217` takes the first `FullInstance` in `window_meta`, which is HashMap order, and looks up its HWND.
- `window_meta` keeps entries for windows whose HWND is gone:
  - a parking close removes `window_hwnds` (`ui_tasks/window.rs:516-521`), and `window_meta` is only removed in `on_before_close`, which never fires for a parked browser;
  - demote removes `window_hwnds` (`window_pool.rs:1033`) but not `window_meta`;
  - so does the `IsWindow` eviction (`commands/window/lifecycle.rs:385,446`).
- So about half the picks hit a dead entry. When a pick succeeds it can be any window: a hidden pool window, another floater.
- The only behaviour that depends on the result is the mother-window resize, which only drag tear-offs request.

## 3. The change

1. **The widgets bar passes its tab.** `action-widgets-menu.ts` sends the window's active `tab_id` with `pane.open`, so srv tears off from the workspace the user is looking at, and the floater opens over that window.
2. **The frontend passes its label.** The `openfloatingpane` handler in `global.ts` adds `source_window_label: await getApi().getWindowLabel()`, as the drag paths do.
3. **The host's fallback only considers live, real windows.**
   - Prefer `"main"`.
   - Otherwise take the most recently focused top-level window that has a live `window_hwnds` entry and isn't `floating-*` or `window-pool-*`.
   - The focus tracker already records the last focused root (`[focus-track] LAST_FOCUSED_BY_ROOT`).
4. **Stale `window_meta` entries go** when a window's HWND goes: parking close, demote, the `IsWindow` eviction. This also helps the window-churn spec (`SPEC_WINDOW_CHURN_RENDERER_LEAK_2026_10_13.md`), where these entries are part of the leftover state.
5. **The warning says what it means:** `debug` level, "floating pane {label}: no source window resolved; placement and mother-resize use the fallback". The orphan-risk wording goes.

## 4. Tests

- **`action-widgets-menu.test.ts`:** the floating action sends the active `tab_id`.
- **`global.ts` handler test:** `openFloatingPane` receives `source_window_label`.
- **srv** (`pane_open_reducer_tests.rs`): a floating `pane.open` with a `tab_id` tears off from that tab's workspace, and the `openfloatingpane` event targets that workspace's window.
- **Host** (a new test module in `commands/floating_pane.rs` over a pure resolver):
  - the fallback prefers `main`;
  - it skips entries with no HWND;
  - it skips `floating-*` and `window-pool-*`.
- **Lab:** with two windows open, open a floating pane from each one's widgets bar 20 times. Every floater opens over the window it was opened from, and the host logs no unresolved-source warnings.
