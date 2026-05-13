# SPEC: Robust DPI Handling for Window-Header Drag

**Status:** Draft
**Date:** 2026-05-13
**Author:** AgentX
**Supersedes (in part):** `docs/retro/BUG_WINDOW_DRAG_CURSOR_DRIFT_2026-05-07.md`
**Related PR:** #734 (`6c7dfe3b`) — original absolute-positioning fix

---

## 1. Problem

The original cursor-drift fix (PR #734, May 7) eliminated the stale-rect race in `move_window_by` by switching to absolute positioning: frontend snapshots `initWinX/Y` via `get_window_position` on mousedown, then sends `set_window_position(initWin + cursorDelta)` on each mousemove.

The bug **has returned** on Windows 11 — but not on Windows 10. The user reports the same symptom as the original: cursor visibly drifts off the click point during drag.

## 2. Root cause

A **latent unit mismatch** in the original fix, masked by Win10's default 100% scale and exposed by Win11's default 125% scale.

### Coordinate spaces in play

| Source | Unit | Authority |
|---|---|---|
| `e.screenX`, `clientX`, `pageX` in CEF/Chromium JS | **CSS pixels** (physical ÷ devicePixelRatio) | Blink stores internally in physical pixels, divides by combined browser-zoom (which folds in device scale factor under `use-zoom-for-dsf`, default on Windows since Chrome 54) before exposing to JS. See [Chromium Blink Coordinate Spaces](https://www.chromium.org/developers/design-documents/blink-coordinate-spaces/). |
| `GetWindowRect` / `SetWindowPos` (Win32, PMv2 process) | **Physical pixels** in the target monitor's DPI context | [Win32 PMv2 docs](https://learn.microsoft.com/en-us/windows/win32/hidpi/high-dpi-desktop-application-development-on-windows): "since a `PROCESS_PER_MONITOR_AWARE` app uses the actual DPI of the monitor, logical and physical coordinates are identical." |
| `window.devicePixelRatio` | physical ÷ CSS | Reflects display density × browser/page zoom. At Win11 125%, `dpr = 1.25`. Updates when window crosses monitors with different DPI. |

### The math, broken

In `frontend/app/hook/useWindowDrag.win32.ts:123-124`:

```ts
const tx = initWinX + (e.screenX - clickScreenX);   // physical + CSS = mismatched units
const ty = initWinY + (e.screenY - clickScreenY);
```

At 100% scale, CSS == physical, so the bug is invisible. At Win11's default 125%:

- Cursor moves 100 CSS pixels (`e.screenX` delta = 100)
- True physical motion = 125 pixels
- We send `setWindowPos(initWinX + 100)` instead of `initWinX + 125`
- Window moves 100 physical pixels while cursor moves 125 — drift of 25 px per 125 px of cursor motion = 20% lag
- Visible after the cursor moves ~50 px; obvious by 200 px

### Why Win10 vs Win11 is the discriminator

It's the **default scale**, not the OS version directly:

- Windows 10 ships at 100% by default on most hardware. `dpr = 1.0`, math accidentally works.
- Windows 11 ships at 125% by default on most laptops, 150% on smaller-screen laptops, 200% on Surface-class devices. `dpr > 1`, math breaks.

A Win10 user manually set to 125% would see the bug. A Win11 user at 100% wouldn't.

### Files involved

- `frontend/app/hook/useWindowDrag.win32.ts` — frontend hook (the unit mismatch lives here, lines 60-65 + 75-78 + 123-124)
- `agentmux-cef/src/commands/window.rs:162-176` — `get_window_position` (returns physical, correct)
- `agentmux-cef/src/commands/window.rs:223-257` — `set_window_position` (consumes physical, correct)

The Rust side is fine. The fix is entirely frontend.

## 3. Prior art

This exact class of bug has been reported across the borderless-window ecosystem:

| Project | Issue | Symptom |
|---|---|---|
| Neutralinojs | [#874](https://github.com/neutralinojs/neutralinojs/issues/874) | Borderless drag with multi-monitor differential scaling; their attempted `pageX * ratio` fix was incomplete |
| Alacritty | [#8448](https://github.com/alacritty/alacritty/issues/8448) | "Window jumps right until it's off the mouse entirely" on Win11 cross-monitor drag |
| Electron | [#14787](https://github.com/electron/electron/issues/14787) | Broken cursor on HiDPI frameless windows |
| Electron | [#8533](https://github.com/electron/electron/issues/8533) | App declared only System-DPI-aware; required `EnableNonClientDpiScaling` + `WM_DPICHANGED` handling |
| Electron | [#10659](https://github.com/electron/electron/issues/10659) | `setPosition` off by a few pixels at non-100% scale |

Common thread: any borderless app that re-implements drag in JS without a DPR multiplier eventually hits this. The OS-native move loop (`WM_NCLBUTTONDOWN` + `HTCAPTION`) handles DPI for free — but CEF doesn't expose NC messages to subclassed wndprocs (per the comment at `useWindowDrag.win32.ts:7`).

## 4. Best practices (synthesized from research)

Ordered most-essential to nice-to-have:

### 4.1 Multiply CSS-pixel deltas by `devicePixelRatio` — the core fix

```ts
const dpr = window.devicePixelRatio || 1;
const tx = initWinX + Math.round((e.screenX - clickScreenX) * dpr);
const ty = initWinY + Math.round((e.screenY - clickScreenY) * dpr);
```

`Math.round` is non-negotiable. Fractional DPRs (1.25, 1.5, 1.75) accumulate floating-point error; sub-pixel `SetWindowPos` calls land at different rounded positions, causing visible jitter ([WICG discussion](https://discourse.wicg.io/t/display-an-image-at-device-s-physical-resolution/1150/)).

### 4.2 Re-read DPR on every mousemove

Windows fires `WM_DPICHANGED` to PMv2-aware windows as they cross monitors with different DPI ([Microsoft Learn](https://learn.microsoft.com/en-us/windows/win32/hidpi/wm-dpichanged)). Chromium updates `window.devicePixelRatio` in response. If we cache `dpr` at mousedown, a mid-drag monitor crossing breaks the math. Cheap to re-read each mousemove:

```ts
document.addEventListener("mousemove", (e) => {
    if (!dragging) return;
    const dpr = window.devicePixelRatio || 1;  // fresh
    const tx = initWinX + Math.round((e.screenX - clickScreenX) * dpr);
    ...
});
```

### 4.3 Resnap baseline when DPR changes mid-drag

The principled answer to "robust under any setting": when DPR changes, the accumulated `(e.screenX - clickScreenX) * dpr` is mixed-unit (some delta accumulated at old DPR, rest at new). Detect change and resnap:

```ts
let dragDpr = window.devicePixelRatio || 1;
document.addEventListener("mousemove", async (e) => {
    if (!dragging) return;
    const currentDpr = window.devicePixelRatio || 1;
    if (currentDpr !== dragDpr) {
        // DPR changed mid-drag — resnap baseline at current cursor.
        const pos = await invokeCommand<{x:number;y:number}>("get_window_position");
        initWinX = pos.x;
        initWinY = pos.y;
        clickScreenX = e.screenX;
        clickScreenY = e.screenY;
        dragDpr = currentDpr;
        return;
    }
    const tx = initWinX + Math.round((e.screenX - clickScreenX) * dragDpr);
    ...
});
```

Trade-off: a single mousemove is "lost" during the IPC for `get_window_position`, but DPR changes are rare (once per cross-monitor transition) so this is acceptable.

### 4.4 Verify the host is registered Per-Monitor V2 DPI-aware

The math in §4.1-4.3 assumes PMv2. If the manifest declares anything less (System-DPI-Aware, or unaware), Windows applies bitmap scaling and the coordinate spaces stop behaving as documented. CEF defaults to PMv2 on modern builds, but worth grepping the bundled manifest to confirm. **Code audit, no runtime change.**

Check command:
```bash
grep -r "dpiAware\|PerMonitor\|SetProcessDpi" agentmux-cef/ scripts/
```

### 4.5 Optionally handle `WM_DPICHANGED` on the Rust side

If mid-drag crosses to a different-DPI monitor, the OS sends `WM_DPICHANGED` with a suggested rect in `lParam` for the new monitor. For drag specifically, we want cursor-on-title-bar to win over OS suggestion — but the size component of `lParam` should be honored so the window doesn't look wrong on the new monitor.

**Defer this** until §4.3 shows residual size glitches in field testing. The frontend resnap from §4.3 is the primary mechanism; this is a secondary polish.

### 4.6 Round in JS, not Rust

`Math.round` in JS before IPC serialization keeps the integer-cast behavior obvious. If we serialize floats and let Rust truncate via `as i32`, the rounding becomes implicit and hard to debug.

### 4.7 Test matrix

Per [Electron #14787](https://github.com/electron/electron/issues/14787) — minimum viable matrix:

- **Single monitor:** 100%, 125%, 150%, 175%, 200%
- **Multi-monitor, same scale per monitor:** 100% + 100%, 125% + 125%
- **Multi-monitor, different scales:** 125% laptop + 100% external (the case where Win10 vs Win11 most differs); 200% + 100% (extreme)
- **Drag fully across monitor boundary** (not just to the edge — actually onto the second monitor)
- **Verify drift at end of long drags** (3+ second sustained drag at moderate speed)

## 5. Proposed implementation

### 5.1 Phase 1 — Hotfix (ship same-day)

Apply §4.1 + §4.6 in `useWindowDrag.win32.ts`. Two locations:

```diff
+ const dpr = window.devicePixelRatio || 1;
- const tx = initWinX + (latestScreenX - clickScreenX);
- const ty = initWinY + (latestScreenY - clickScreenY);
+ const tx = initWinX + Math.round((latestScreenX - clickScreenX) * dpr);
+ const ty = initWinY + Math.round((latestScreenY - clickScreenY) * dpr);
```

(at mousedown catch-up, lines ~76-78)

```diff
+ const dpr = window.devicePixelRatio || 1;
- const tx = initWinX + (e.screenX - clickScreenX);
- const ty = initWinY + (e.screenY - clickScreenY);
+ const tx = initWinX + Math.round((e.screenX - clickScreenX) * dpr);
+ const ty = initWinY + Math.round((e.screenY - clickScreenY) * dpr);
```

(at mousemove, lines ~123-124)

Reads DPR fresh on each call (§4.2) — Chromium's `window.devicePixelRatio` getter is cheap.

**Risk:** very low. No Rust changes. Single-monitor users at 100% see byte-identical behavior (`dpr = 1.0` → multiplier is no-op). Users at 125% / 150% / 175% / 200% have their bug fixed.

**Scope:** covers single-monitor + homogeneous multi-monitor. Cross-monitor mid-drag with differential DPI is improved (the per-mousemove read picks up new DPR) but baseline isn't resnapped — small visible glitch at the crossing moment.

### 5.2 Phase 2 — Robust (follow-up PR)

Add §4.3 (resnap on DPR change) and §4.4 (manifest audit). Add §4.7 test matrix as smoke checklist.

### 5.3 Phase 3 — Defer

§4.5 (`WM_DPICHANGED` mid-drag handler) only if Phase 2 leaves residual glitches.

### 5.4 Non-changes (and why)

- **`get_window_position` / `set_window_position` Rust handlers:** unchanged. They correctly use physical pixels. The bug is the frontend's unit conversion, not the IPC contract.
- **`move_window_by` Rust handler:** unchanged. It's still vulnerable to the stale-read race that PR #734 fixed, but isn't on the drag path. (Could be removed entirely in a future cleanup; left for backward compat with any external callers.)
- **Switching to OS-native move loop (`WM_NCLBUTTONDOWN` + `HTCAPTION`):** the comment at `useWindowDrag.win32.ts:7` notes this doesn't work — async IPC roundtrip loses mouse state by the time Rust posts the message. Architectural change, out of scope.
- **`-webkit-app-region: drag` CSS:** disables all DOM events on the affected element ([Electron docs](https://zeke.github.io/electron.atom.io/docs/api/frameless-window/)). Doesn't fit AgentMux's title-bar interaction model (close/min/max buttons + tab clicks). Rejected.

## 6. Why this regressed despite a code freeze

PR #734's code is byte-identical between merge and today — no one reverted the fix. The regression isn't from a code change at all. The fix had a latent unit bug that was tested on Win10 (100% default) but never on Win11 (125% default).

The retro doc at `docs/retro/BUG_WINDOW_DRAG_CURSOR_DRIFT_2026-05-07.md` doesn't mention DPI scaling either, which suggests the author never observed it because they tested at 100%. A test-matrix item for high-DPI would have caught this pre-merge.

## 7. Open questions

1. **Linux behavior?** The hook at `useWindowDrag.linux.ts` likely has the same bug if Linux is run at fractional scaling (HiDPI laptops with `scale-monitor-framebuffer` or fractional scaling enabled). Out of scope for this spec; flag for follow-up.
2. **macOS behavior?** macOS handles HiDPI differently (point-vs-pixel coordinate space). The hook at `useWindowDrag.darwin.ts` likely needs a similar audit. Out of scope; flag for follow-up.
3. **Should the retro be updated?** Yes — once Phase 1 lands, append a "Postscript: DPI regression" section to `docs/retro/BUG_WINDOW_DRAG_CURSOR_DRIFT_2026-05-07.md` documenting that the fix shipped without DPI testing.

## 8. References

- [Chromium Blink Coordinate Spaces](https://www.chromium.org/developers/design-documents/blink-coordinate-spaces/) — authoritative on `use-zoom-for-dsf` and CSS-pixel exposure
- [MDN: Window.devicePixelRatio](https://developer.mozilla.org/en-US/docs/Web/API/Window/devicePixelRatio)
- [Microsoft Learn: High DPI Desktop Application Development](https://learn.microsoft.com/en-us/windows/win32/hidpi/high-dpi-desktop-application-development-on-windows)
- [Microsoft Learn: WM_DPICHANGED](https://learn.microsoft.com/en-us/windows/win32/hidpi/wm-dpichanged)
- [Microsoft Learn: Mixed-Mode DPI Scaling](https://learn.microsoft.com/en-us/windows/win32/hidpi/high-dpi-improvements-for-desktop-applications)
- [Microsoft Learn: LogicalToPhysicalPointForPerMonitorDPI](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-logicaltophysicalpointforpermonitordpi)
- [Marius Bancila: How to build high DPI aware native Windows desktop applications](https://mariusbancila.ro/blog/2021/05/19/how-to-build-high-dpi-aware-native-desktop-applications/)
- [Dan Reynolds: Handling Chrome's DPI Scaling](https://danreynolds.ca/tech/2017/10/15/Variable-Browser-Zoom/)
- [Electron #14787 — Frameless windows break mouse cursor on High DPI](https://github.com/electron/electron/issues/14787)
- [Electron #8533 — Per-monitor DPI awareness](https://github.com/electron/electron/issues/8533)
- [Electron #10659 — Cannot accurately set position when scaling not 100%](https://github.com/electron/electron/issues/10659)
- [Neutralinojs #874 — Multi-monitor differential scaling drag bug](https://github.com/neutralinojs/neutralinojs/issues/874)
- [Alacritty #8448 — Buggy window drag/resize on Windows 11](https://github.com/alacritty/alacritty/issues/8448)
- [Win32 DPI And Monitor Scaling gist (marler8997)](https://gist.github.com/marler8997/9f39458d26e2d8521d48e36530fbb459)

---

## 9. Decision

Recommend: **Ship Phase 1 (§5.1) as a hotfix today**, file a follow-up issue tracking Phase 2 (§5.2). Add §4.7 test matrix to the engineering smoke-test checklist regardless.

Approval:

- [ ] AgentX (author)
- [ ] (Reviewer)
