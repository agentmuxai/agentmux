# REPORT — browser pane stuck on the loading brain after a tab tear-off: host events delivered to the wrong window

**Date:** 2026-09-06
**Author:** AgentX
**Status:** implemented — fix in PR #3035 (§5), pending review/merge. Not yet live-verified in the reporting instance — the fix is host-side (Rust) and needs a rebuilt host to take effect.
**Instance:** `channel:local-main-b28b7a-2cd6e3d7`, v0.55.37, Windows 11.
**Platform:** all — the misrouted call sites are platform-independent Rust.

---

## 1. The report, restated precisely

The operator tore a tab containing a browser pane (claude.ai) off the main window into a second window. From then on the pane showed the loading "brain" overlay permanently. Typing a new URL in the address bar did navigate (the page behind the overlay changed), but the overlay never cleared, the address bar never updated to the landed URL, and the tab title stayed at the optimistic `claude.ai` rather than the page's real title. No error was shown and nothing crashed.

## 2. What the logs proved before any code was read

Host log: `~/.agentmux/channels/local-main-b28b7a-2cd6e3d7/versions/0.55.37/logs/agentmux-host-v0.55.37.log.2026-09-06`. Pane block `2ac146c2-097b-4a17-89b8-fc548e09f5b5` (`2ac146c` below).

| Time | Side | Observation |
|---|---|---|
| 21:09:33 | host | Pane `2ac146c` created in window `main`, browser label `browser-pane-2ac146c2-…-1`. |
| 21:09:33 → 21:24:45 | frontend | View model `yuurk6` in `main`'s renderer receives 42 `browser-pane-nav-state` events. Overlay behaves normally. |
| 21:24:45 | host | `[dnd:cef] start_cross_drag drag_type=Tab source_window=main`. Pane closed in `main`; pool window promoted (`WindowInstanceAssigned num: 2`). |
| 21:24:45 | host | Pane `2ac146c` re-registered with `window_label=window-pool-930b71ac8b86455eaf3a785f2f2e247d`, browser label `…-2`. |
| 21:24:45.397 | frontend | `yuurk6` disposed cleanly; its listener unsubscribed. |
| 21:24:45.906 | frontend | New view model `4az61e` constructed in window 2's renderer; subscribes to `browser-pane-nav-state`. |
| 21:24:46 → 21:24:48 | host | Host emits `emit-nav-state … is_loading=false` and `emit-title-change title="Sign in - Claude"` for `2ac146c` — the host side is healthy and did finish the load. |
| 21:24:46 → end | frontend | `4az61e` receives **zero** host events of any kind. Outbound `invokeCommand` (navigate) succeeds. |

So the host produced the load-end; the pane's live view model never got it. The gap is delivery.

## 3. Root cause

All five host→page push sites for browser panes call `emit_event_from_state`
(`agentmux-cef/src/events.rs`), which delivers to **the window labelled `main`**,
falling back to "any browser" only if `main` is absent:

| Site | Event |
|---|---|
| `agentmux-cef/src/browser_pane/callbacks.rs` (main-frame loading tracker) | `browser-pane-nav-state` |
| `agentmux-cef/src/browser_pane/callbacks.rs` (`on_load_end`, url-only) | `browser-pane-nav-state` |
| `agentmux-cef/src/browser_pane/callbacks.rs` (`on_loading_state_change`) | `browser-pane-nav-state` |
| `agentmux-cef/src/client/display.rs` (`on_title_change`) | `browser-pane-title-change` |
| `agentmux-cef/src/client/display.rs` (`on_favicon_urlchange`) | `browser-pane-favicon-urls` |

After the tear-off the pane's view model lives in window 2's renderer. `main` still exists, so the lookup succeeds, `execute_java_script` runs in `main`'s renderer, and the `agentmux-event` `CustomEvent` fires there — where no view model for that `block_id` is subscribed any more. The event is dropped silently: no warning on the host (the target browser was found), no log on the frontend (nobody is listening).

Why that presents as a stuck brain: `frontend/app/view/browser/browser-view.tsx` clears the overlay only on a true→false transition of the pane's `loadingAtom`, which is driven solely by the `is_loading` field of `browser-pane-nav-state`. Navigation still works because `invokeCommand` is request/response and window-agnostic — which is exactly the "navigates but never finishes" shape reported.

The pane registry already knows the right answer. `HostState.browser_panes[block_id].window_label` is written by the reducer (`reducer/panes.rs`, `TryRegisterBrowserPaneLive`) before the CEF browser is created, and the log confirms it held `window-pool-930b71ac…` for the recreated pane. `AppState::browser_pane_window_label` exposes it. Nothing in the five sites consulted it.

## 4. This is a known bug class that was half-fixed

PR #2597 (2026-08) hit the identical problem for `browser-pane-clicked` and left this comment in `browser_pane/hwnd.rs`:

> Route to the pane's ACTUAL owning window, not "main" — a pane torn off into its own floating window has its own JS context; `emit_event_from_state`'s "main"/first-available fallback delivered this to the wrong window (or none) for floating panes …

It rerouted three events (click, shortcut, context menu) via `browser_pane_window_label` + `emit_event_to_window`. The five events that drive the loading overlay, address bar, title and favicon were never touched. `git log -S` puts the nav-state site's `emit_event_from_state` call at 2026-05-02 (#667), re-touched by #2642 (2026-08-17, brain-flicker fix) without changing the routing. **Not a 0.55.37 regression** — latent since May, surfacing whenever a browser pane lives in any window other than `main` while `main` is still open. Tear-off is the common way to get there; opening a browser widget directly in a second window hits the same path.

## 5. Fix

Host-side only; no frontend change (the frontend already filters by `block_id`).

- New `events::emit_browser_pane_event(state, block_id, event, payload)`: resolves the pane's owning window from the reducer entry and delivers with `emit_event_to_window`. If the pane has no entry, or the entry's `window_label` is empty (the legacy `EnqueueBrowserPaneCreate` path, which has no production caller), it warns and falls back to the previous `main` routing so nothing that worked before regresses. The reducer lock is released before emitting, since `emit_event_to_window` re-locks it for its own lookup.
- The routing decision is a pure function, `events::browser_pane_event_target(&HostState, block_id)`, so the reducer tests pin it without CEF handles.
- All five sites above now call the new helper.
- Targeted delivery was chosen over `emit_event_to_top_level_windows` deliberately: `state/mod.rs` documents that per-pane events are routed per-window so a hostile page in one pane cannot observe another pane's traffic, and broadcast would widen that.

Tests added (`agentmux-cef/src/reducer/tests.rs`):
- `browser_pane_event_target_is_the_owning_window_not_main` — pane registered in `window-pool-930b71ac` resolves to that label.
- `browser_pane_event_target_is_none_for_unknown_or_windowless_pane` — unknown block and legacy empty-label entry both return `None` (fallback path).

**Fix PR:** #3035 — https://github.com/agentmuxai/agentmux/pull/3035

## 6. Workaround in a running unpatched instance

The pane in the second window will never recover on its own. Drag the tab back into the main window, or close it and open a fresh browser pane there; events route correctly again because pane and `main` coincide.

## 7. Open-issue canvas (browser pane, as of 2026-09-06)

Surveyed all open issues in `agentmuxai/agentmux` for browser-pane topics (browser, brain, overlay, favicon, tear-off, floating, nav-state, address bar, webview, CEF, multi-window, title). **No open issue describes this symptom; the misrouting was untracked.**

| # | Title | Relation to this root cause |
|---|---|---|
| #768 | Phantom browser pane (orphan + tearoff): host/frontend lifecycle divergence | **Possibly related.** Same scenario (browser panes landing in a second window and never coming alive) but attributed to HWND reparenting: "host reparented the HWNDs but frontend never re-handshook against the new chrome." Its last comment (07-11) says the repro predates pool-served tear-off and must be re-reproduced. **Re-test against this fix.** |
| #2908 | Ctrl+Wheel zoom does not work in floating panes | Unrelated — input delivery into child-HWND floaters, not host→page routing. |
| #2551 | OAuth sign-in popup sizing | Unrelated — Chrome-runtime popup. |
| #1190 | Wire keyboard shortcuts (Ctrl+L/T/W/F) | Unrelated feature request; may be partly stale after #2597 shipped shortcut routing. |
| #1569 | macOS/Linux orphan reconciler can't detect crash-orphans | Unrelated. |
| #2873 | Tear-off silently fails when another floating pane sits under the drop point | Unrelated (DnD target resolution); multi-window. |
| #3028 | Cold-path `open_new_window` creates a window that is never shown | Unrelated; open PR #3030 targets it. |

Closed sibling: #1461 (redock black page) — HWND lifecycle, not event routing.
No open PR touches `emit_event_from_state`, nav-state, title-change or favicon routing.

## 8. Separate cosmetic finding

The `{"isTrusted":true}` unhandled-rejection lines in the frontend log during this session are `favicon-load fail` for the claude.ai favicon. Cosmetic and unrelated to the overlay; not addressed here.

## 9. Follow-ups

- Live-verify in a rebuilt host: tear a browser-pane tab into a second window; overlay must clear, address bar and title must update. Then re-run #768's tear-off repro.
- `emit_event_from_state` still has other callers outside the pane paths. Each is a candidate for the same class of bug if its payload is per-pane or per-window; worth a sweep, out of scope here.
