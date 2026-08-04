# Implementation Plan: fix HiDPI mis-positioning of pool-promoted new windows

**Date:** 2026-06-21
**Area:** `agentmux-cef` — Windows new-window pool promotion
**Bug:** "Open another window" can open a blank/invisible window on HiDPI displays.
**Regression source:** `#1609` (`c004540c`) "feat(pool): Windows new-window pool"
**Status:** Implemented — §3.2 (clamp) + §3.3 (post-show re-assert) + telemetry landed; `clamp_rect_within` unit-tested. §3.1 (size normalization) and §3.4 (explicit repaint) deferred as follow-ups (the clamp+re-assert already guarantees on-screen at the current 960×640 size).
**Related:** `#1610`/`#1612`/`#1616` (recent pool work), `docs/retro/retro-blank-new-window-2026-06-21.md` (the layout-seed fix, orthogonal and already shipped).

---

## 1. Symptom & evidence

On Windows at 125% scaling, clicking status-bar "+ Open another window" sometimes
yields a window that renders its content but is never shown on-screen.

Live CDP inspection of the running 0.47.1 build (4 opens), `window.screenX/Y` +
`outerWidth/Height` + rendered block count:

| Window | screen pos | size | blocks | on-screen |
|--------|-----------|------|--------|-----------|
| Starter (main) | (397,222) | 2150×1176 | 75 | ✅ |
| Window 2 | (421,246) | 960×640 | 75 | ✅ |
| Window 4 | (421,246) | 960×640 | 75 | ✅ |
| **Window 3** | **(-25600,-25600)** | **159×27** | **75** | ❌ |
| pool spare ×2 | (-26214,-26214)→(-32000) | 1200×800 | 0 | off-screen (correct) |

The layout seed works (every window has 75 block-frames / the 3 panes). The defect
is purely **window placement**: Window 3 rendered but stayed off-screen.

Two numeric tells, both `× 0.8` (= `96/120`, i.e. ÷1.25 at 125% DPI):
- `-25600 = -32000 × 0.8` → Window 3 is parked at the **DPI-scaled `POOL_OFFSCREEN`**;
  it was never moved on-screen.
- `960×640 = 1200×800 × 0.8` → even the "good" windows are the **POOL default size
  virtualized**, not the intended `get_secondary_window_size` (70% of work area).

So the coordinates that promotion feeds `SetWindowPos` are in an inconsistent DPI
space, and nothing guarantees the result is on-screen.

---

## 2. Code flow (today)

`open_new_window` (`commands/window/creation.rs:251`):
```rust
let (pos_x, pos_y) = get_offset_position();                 // GetWindowRect(cur)+30  (creation.rs:388)
let (win_w, win_h) = get_secondary_window_size(pos_x,pos_y);// 70% of monitor work area (creation.rs:411)
promote_pool_window_for_new_window(state, pos_x, pos_y, win_w, win_h)  // win_w/h DROPPED on Windows
```

`promote_pool_window_for_new_window` (Windows, `window_pool.rs:1057`):
```rust
promote_pool_window(state, "", pos_x, pos_y,
    None, None,                 // width/height → POOL_WIDTH/HEIGHT defaults (1200×800)
    Some(pos_x), Some(pos_y))   // tab_anchor → final pos = (pos_x, pos_y)
```

`promote_pool_window` (Windows, `window_pool.rs:547`):
- size: `width=None` ⇒ `win_w = POOL_WIDTH (1200)`, `win_h = POOL_HEIGHT (800)` — no conversion.
- pos: anchor branch ⇒ `(pos_x, pos_y)`.
- `SetWindowPos(hwnd, HWND_TOP, pos_x, pos_y, 1200, 800, 0)` then `ShowWindow(SW_SHOW)` (`:831`,`:852`).

Pool window was created at `POOL_OFFSCREEN (-32000,-32000)`, size 1200×800 (`window_pool.rs:113-116`).

**Defects:**
- **D1 — wrong/virtualized size:** intended `win_w/win_h` (70% work area) are discarded;
  `POOL_WIDTH/HEIGHT` are used and then DPI-virtualized to 960×640. The `width.is_some()`
  branch in `promote_pool_window` (`:774`) would *double*-convert already-physical inputs,
  which is why #1609 passes `None` — trading a double-convert bug for a wrong-size bug.
- **D2 — no on-screen guarantee:** if `SetWindowPos` doesn't take effect (races the pool
  window's create/show, or DPI-context not yet established), the window is left at the
  scaled `POOL_OFFSCREEN`. Nothing detects or corrects an off-screen result.
- **D3 — DPI-context race:** the pool window's per-monitor DPI context can change as it
  moves from the off-screen origin to the destination monitor; a single pre-show
  `SetWindowPos` is applied before that settles.

---

## 3. Fix

Goal: the promoted new window lands **on-screen, correctly sized, every time**, on any
DPI. Strategy: compute the rect once in correct physical px for the destination monitor,
**clamp to the work area** (the hard guarantee), and **re-assert after show** (kill the race).

### 3.1 Thread the intended size through (fix D1)
- `promote_pool_window_for_new_window` (Windows): stop dropping `width/height`. Pass them
  to `promote_pool_window`, tagged as **already-physical** so no DPI multiply is applied.
- In `promote_pool_window`, replace the `width.is_some() ? to_physical(w) : w` logic with a
  single rule: **callers pass physical px; never multiply.** `get_secondary_window_size`
  returns work-area-derived physical px; the tear-off caller already passes
  `window.outerWidth/Height`-derived values — audit both and make the unit a documented
  contract on the function signature (e.g. rename params `phys_w/phys_h` or add a doc
  invariant). Remove the now-unneeded `to_physical` size path.

### 3.2 Compute + clamp the destination rect (fix D2 — the safety net)
Add a helper, `clamp_rect_to_work_area(x, y, w, h, anchor_x, anchor_y) -> (x,y,w,h)`:
- Resolve the destination monitor via `MonitorFromPoint((anchor_x, anchor_y), NEAREST)`.
- Read its `rcWork` (physical px, `GetMonitorInfoW`).
- Shrink `w/h` to fit the work area if larger; then shift `x/y` so the full window is
  inside `rcWork` (right/bottom edges clamped, then left/top, so a too-large or
  out-of-bounds rect can never leave the window partially or fully off-screen).
- Apply in `promote_pool_window` right before `SetWindowPos`. This alone removes the
  user-visible symptom even if subtle DPI math remains.

### 3.3 Re-assert geometry after show (fix D3)
- Order: `ShowWindow(SW_SHOW)` → then `SetWindowPos` (move+size+`HWND_TOP`) → then a
  second `SetWindowPos` with `SWP_NOZORDER|SWP_NOACTIVATE` on the **next UI tick**
  (`post_task` to the UI thread) to re-assert the rect after the window's DPI context and
  first paint have settled. Verify the final `GetWindowRect` intersects the work area; if
  not, log `pool:new-window placement off-screen` at `error` for telemetry.

### 3.4 Trigger a repaint on promotion (defensive)
- After show, invalidate/repaint the CEF host view (`InvalidateRect` on the host HWND, or
  `browser.host().was_resized()`) so the off-screen→on-screen surface is composited. Pool
  windows are painted off-screen; ensure the move forces a fresh frame. (Low risk; guards
  against a "shown but stale surface" variant.)

### 3.5 Scope guard
- Changes are **Windows-only**, confined to `agentmux-cef/src/commands/window_pool.rs`
  (and a 1-line caller change in `promote_pool_window_for_new_window`). The tear-off path
  shares `promote_pool_window`; the clamp + re-assert are safe for tear-off too (a torn-off
  window should also never land off-screen), but verify the anchor-drag UX still feels right
  (the clamp only moves a window that would otherwise be off-screen).

---

## 4. Files to change
- `agentmux-cef/src/commands/window_pool.rs`
  - `promote_pool_window` (Windows, ~547): size-unit contract (3.1), clamp (3.2), re-assert + repaint (3.3/3.4).
  - `promote_pool_window_for_new_window` (Windows, ~1057): pass real `width/height` (3.1).
  - new `clamp_rect_to_work_area` helper.
- (maybe) `agentmux-cef/src/commands/window/motion.rs` if the re-assert reuses `set_window_full_rect`.

## 5. Test plan
**Automated**
- Unit-test `clamp_rect_to_work_area`: rect bigger than work area, rect off the right/bottom,
  rect at scaled `POOL_OFFSCREEN`, negative-coord secondary monitor → result fully inside the
  given work area, size ≤ work area.

**Manual / CDP (HiDPI, 125%)**
- Open ≥6 windows via "+ Open another window" in quick succession. For each, assert via CDP
  `window.screenX/Y` is on-screen (inside a monitor work area) and `outerWidth/Height` ≈ 70%
  work area, and `blocks=75`. **Zero** windows at `~-25600` or sized `159×27`.
- Repeat on a second monitor at a different scale (e.g. 100%) by moving the source window
  there first — confirms destination-monitor DPI is used.
- Tear-off a tab → the torn window still lands under the cursor (clamp didn't regress UX).

## 6. Risks & rollback
- **Risk:** clamp interferes with intentional multi-monitor negative coords. Mitigated by
  clamping to the **destination** monitor's work area (chosen from the anchor point), not the
  primary, and only adjusting when the rect would be off that monitor.
- **Risk:** re-assert `post_task` introduces a visible reposition flicker. Mitigated by
  doing the authoritative move pre-show and the re-assert as a same-rect idempotent confirm.
- **Rollback:** the whole change is gated to the Windows promote path; reverting restores
  current behavior. If the DPI math proves fragile under time pressure, fall back to
  **disabling the Windows new-window pool** (route `open_new_window` to the cold path) —
  correct, ~3s slower first window — as a stopgap.

## 7. Verification gate (done = )
- `cargo check -p agentmux-cef` clean; `clamp_rect_to_work_area` unit tests pass.
- CDP sweep: 6/6 new windows on-screen, correctly sized, 75 blocks; none off-screen.
- Retro/PR references this plan.
