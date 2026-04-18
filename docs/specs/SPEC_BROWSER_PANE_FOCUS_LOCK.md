# SPEC: Browser Pane Focus Lock

Status: draft
Date: 2026-04-18
Owner: AgentA
Reported by: user, v0.33.262 (post Phase 4 modularization + install_pane_focus_redirect wire-up)
Related: `SPEC_BROWSER_PANE_LIFECYCLE.md` §5 race #2 and §5 race #5,
`SPEC_BROWSER_PANE_LIFECYCLE_TESTS.md` (L1/L3 seam pattern).

## 1. Symptom

> "If I select inside the browser, search, and I try typing anywhere else
> including the address bar or any other tab, the only place that typing
> works is in the browser pane. It appears that after I type into the
> browser it is locked in."

Once Windows-level keyboard focus transfers to the pane's outer HWND,
clicking any HTML input outside the pane (address bar, terminal prompt,
tab name, another widget) doesn't unstick it. The HTML `activeElement`
moves visibly, but `WM_KEYDOWN` keeps routing to the pane's renderer
until something forcibly reclaims HWND focus.

## 2. How focus is supposed to work today

Two layers, coupled indirectly through IPC.

### Chromium-level focus

- `CefBrowser::host().set_focus(1|0)` tells Chromium "this browser is
  focused." Chromium routes input events to its renderer based on this
  flag plus its own internal state.
- In `browser_panes.rs::focus` (pane) and `defocus_all` (all panes),
  we flip this flag.

### Windows-level focus

- `SetFocus(hwnd)` is the OS API that decides which HWND receives
  `WM_KEYDOWN` / `WM_CHAR` / `WM_MOUSEWHEEL`.
- AgentMux calls this from two places:
  - `browser_panes.rs::focus` — after `host.set_focus(1)` it also calls
    `SetFocus(pane_hwnd)` under `ALLOW_PANE_FOCUS_ONCE=true` so the
    subclass allows the otherwise-redirected `WM_SETFOCUS`.
  - `ipc.rs::main_window_focus` — calls `SetFocus(top_level_hwnd)` where
    `top_level_hwnd` comes from `find_own_top_level_window()`.

### Subclass redirect

- `pane/hwnd.rs::install_pane_focus_redirect` subclasses the pane outer
  HWND + descendants. On every `WM_SETFOCUS`:
  - If `ALLOW_PANE_FOCUS_ONCE` is true → consume the flag, pass through.
  - Else → call `SetFocus(GetParent(hwnd))` and return 0 (Chromium's
    page-load / JS `window.focus()` steals get redirected back).
- **Does NOT intercept `WM_KILLFOCUS`.**

## 3. Root cause (three-part desync)

### 3a. `defocus_all` doesn't touch Windows-level focus

`browser_panes.rs::defocus_all` calls `host.set_focus(0)` on every Live
pane. That's *Chromium-level only*. The pane's HWND still holds Windows-
level focus. Even if Chromium internally marks itself defocused, Windows
keeps sending `WM_KEYDOWN` to the same HWND.

Evidence: `browser_panes.rs::defocus_all` body at
`agentmux-cef/src/browser_panes.rs:300-311` — no `SetFocus` call.

### 3b. `main_window_focus` is fire-and-forget from the frontend

The address-bar `onFocus` in `browser-view.tsx:147-152` calls
`invokeCommand("main_window_focus", {}).catch(() => {})`. No await. The
Solid event handler returns immediately; Windows keeps routing keys.

If anything else on the page requests focus (e.g. a click handler that
fires after `onFocus`, or page-layer JS `window.focus()`), it can race
the async IPC.

### 3c. No `WM_KILLFOCUS` handling in the subclass

When `main_window_focus` *does* eventually call `SetFocus(parent_hwnd)`,
Windows sends `WM_KILLFOCUS` to the pane HWND. Our subclass forwards it
to the original Chromium WndProc. Chromium processes it, but since the
pane's descendants (Chrome_WidgetWin_1, Chrome_RenderWidgetHostHWND)
have *also* been subclassed, the message-chain interactions are harder
to reason about. There's no explicit path that marks "this pane should
no longer accept focus" — `ALLOW_PANE_FOCUS_ONCE` is single-use, so a
follow-on Chromium-internal `SetFocus` during close / render / nav ends
up accepted by the subclass path.

### 3d. What actually resets the lock today

Empirically the only exits from focus-lock are:
1. **Clicking a different pane** — `browser_pane_focus` fires for the
   new block_id with `ALLOW_PANE_FOCUS_ONCE=true`; the new pane's HWND
   gets `SetFocus`; old pane loses it implicitly.
2. **Closing the pane** — `DestroyWindow` eliminates the target HWND;
   Windows has to reassign focus somewhere else.
3. **Triggering a navigation inside the locked pane** — Chromium
   rebuilds `Chrome_RenderWidgetHostHWND` and we reinstall the subclass
   (`on_load_end_pane`), which in turn starts the focus state fresh.

None of these are "click the address bar," which is what the user is
trying to do.

## 4. Race in the IPC path

```
T0   user clicks address bar
T0+0 SolidJS onFocus runs; sets activeElement to input
T0+0 invokeCommand("main_window_focus") enqueued     ── fire-and-forget
     (onFocus returns, JS continues)
T0+k Rust IPC thread picks up main_window_focus
T0+k ipc.rs: SetFocus(top_level_hwnd)
T0+k ipc.rs: defocus_all() → host.set_focus(0) on pane
T0+k returns

  ── meanwhile ──

T0+ε pane's renderer fires window.focus() (page script, or our own
     client.rs pane `on_set_focus` on load end, or a scroll-into-view)
T0+ε Chromium calls SetFocus(pane_hwnd)
T0+ε Subclass WM_SETFOCUS: ALLOW_PANE_FOCUS_ONCE is false → redirect
     to parent via SetFocus(parent)

  ── user types ──

Tn   WM_KEYDOWN delivered to whoever has focus. If T0+ε happened AFTER
     T0+k's SetFocus(top_level_hwnd), the redirect lands keys on the
     top-level, which propagates to the address bar. Good.
     If T0+ε happened BEFORE T0+k, the subclass redirects to parent —
     but the parent-walk reaches the *pane's parent*, which might be
     a Chromium-owned Chrome_WidgetWin_1, not the top-level Views
     window. Result: focus bounces around inside the pane's tree
     rather than escaping to main.
```

So the lock is real and the race is the cause. The fix has to break
the race decisively, not defensively.

## 5. Fix

Four pieces. Analogous structure to how we split `close()` with
`PaneCloseOps` — put a trait seam so the Win32/Chromium side is
testable, fix the actual bug behind the seam.

### 5.1. `PaneFocusOps` trait (new, pane/ops.rs or its own file)

```rust
pub trait PaneFocusOps {
    /// Release Chromium-level focus on the given browser AND Win32 focus
    /// on its HWND. Also SetFocus the provided fallback HWND (top-level
    /// window) so keystrokes route there immediately.
    fn release_browser_focus(&self, label: &str, fallback_hwnd: usize);

    /// Acquire Chromium + Win32 focus on the given browser. Sets
    /// ALLOW_PANE_FOCUS_ONCE, calls host.set_focus(1), calls SetFocus.
    fn acquire_browser_focus(&self, label: &str);
}
```

Production impl wraps `&Arc<AppState>` + the Win32 calls. Mock impl in
tests records calls; tests assert both Chromium AND Win32 sides fire.

### 5.2. `defocus_all` does both layers

Replace the current body. Instead of just Chromium:

```rust
pub fn defocus_all(&self, state: &Arc<AppState>, fallback_hwnd: usize) {
    let labels = self.lifecycle.live_labels();
    let ops = AppStateFocusOps(state);
    for label in &labels {
        ops.release_browser_focus(label, fallback_hwnd);
    }
}
```

The `fallback_hwnd` is the top-level Views HWND; IPC `main_window_focus`
passes it in so the release walk deterministically lands focus on the
main window before the next message is processed.

### 5.3. Subclass intercepts `WM_KILLFOCUS` + reads explicit focus state

Add a per-HWND `is_focused: AtomicBool` (in the same `PANE_WNDPROCS`
map, alongside the original WndProc pointer). Replace the `ALLOW_PANE_FOCUS_ONCE`
single-use gate with:

- `WM_SETFOCUS`: allow iff `is_focused[hwnd] == true`; otherwise redirect.
- `WM_KILLFOCUS`: set `is_focused[hwnd] = false`; forward normally.

`acquire_browser_focus` flips `is_focused[hwnd] = true` before `SetFocus`.
`release_browser_focus` flips `is_focused[hwnd] = false` before
`SetFocus(fallback)` — so any Chromium-internal re-focus attempt that
arrives after release is redirected.

This turns the flag from "single-shot override" into a "should this pane
be taking focus right now" predicate — much easier to reason about.

### 5.4. Frontend: make `main_window_focus` awaitable + gate re-focus

In `browser-view.tsx`, the address bar's `onFocus` currently does:

```tsx
onFocus={(e) => {
    e.currentTarget.select();
    invokeCommand("main_window_focus", {}).catch(() => {});
}}
```

Change to `await` the IPC before any follow-on code runs. More
importantly: gate the `onMouseEnter` hover-focus path on a
`recentlyDefocused` window (say 250ms) so moving the cursor back over
the pane after clicking an input doesn't immediately refocus the pane.

### 5.5. Optional: per-block focus state surfaced to frontend

`BrowserPaneManager` already has `live_labels()`. Add `focused_label()`
returning the one pane (if any) currently marked `is_focused`. Surface
via IPC; frontend uses it for the pane-has-focus visual indicator
(accent border etc.) — useful for debugging the lock separately from
fixing it.

## 6. Tests

Structured per the pyramid in `SPEC_BROWSER_PANE_LIFECYCLE_TESTS.md`.

### L1 Rust (pane/focus.rs mock tests)

| # | Test | Asserts |
|---|---|---|
| 1 | `release_browser_focus_calls_both_layers` | Mock records both `set_focus(0)` on Chromium AND `SetFocus(fallback)` on Win32 |
| 2 | `acquire_then_release_flips_state` | `is_focused[hwnd]` flips true on acquire, false on release |
| 3 | `release_rejects_subsequent_wm_setfocus` | With state flipped to false, a simulated `WM_SETFOCUS` handler path redirects rather than allowing |
| 4 | `defocus_all_iterates_live_panes_only` | Closing/missing panes skipped |

### L2 frontend (browser-model + browser-view)

| # | Test | Asserts |
|---|---|---|
| 1 | `address_bar_focus_awaits_main_window_focus` | IPC is awaited, not fire-and-forget |
| 2 | `hover_reentry_respects_cooldown` | After `main_window_focus`, `onMouseEnter` does NOT re-call `browser_pane_focus` for 250ms |
| 3 | `giveFocus_skips_when_closed` (existing) | No regression |

### L3 Rust integration (with mocked CEF)

| # | Scenario | Asserts |
|---|---|---|
| 1 | `click_address_bar_releases_pane_focus` | main_window_focus handler invokes release_browser_focus with top-level HWND as fallback; pane's is_focused = false |
| 2 | `chromium_internal_focus_steal_rejected_after_release` | Simulated `WM_SETFOCUS` after release → redirected; `is_focused` stays false |
| 3 | `new_pane_focus_doesnt_leak_to_old_pane` | Click pane A (acquire), click pane B (release A, acquire B) → A's is_focused=false, B's is_focused=true, main's HWND no longer holds focus |

## 7. Not in scope

- Wayland / macOS behavior — focus semantics are OS-specific. The pane
  HWND subclass is Windows-only today (`#[cfg(target_os = "windows")]`),
  the fix here stays Windows-only.
- `AgentMuxPaneFocusHandler` — the Chromium-layer FocusHandler that
  cancels NAVIGATION focus. That path still matters but is orthogonal
  to the Windows-level issue.
- Cross-window focus (multiple AgentMux windows). Assumed one window
  throughout.

## 8. Order of delivery

1. Write the L1 + L2 tests *failing* against current main.
2. Add `PaneFocusOps` trait + production impl.
3. Rewrite `defocus_all` and `focus` to go through the trait.
4. Replace `ALLOW_PANE_FOCUS_ONCE` with the per-HWND `is_focused` map.
5. Add `WM_KILLFOCUS` handling.
6. Frontend: await the IPC, add hover cooldown.
7. L3 integration tests green.
8. User smoke test via task dev.

Each step is one commit; all on one branch. No portable build until
step 8.
