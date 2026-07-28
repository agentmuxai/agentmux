# SPEC: Floating-pane edge-resize (Win32)

**Date:** 2026-05-29
**Author:** AgentA
**Status:** **Implemented (JS-driven).** The `HTTRANSPARENT` forwarder (Phase 1 below) was built and **abandoned** — it does not work on cef-rs 146. What shipped is the JS-driven approach in §0. Phases 2–3 remain future work.
**Investigation:** `docs/analysis/REPORT_FLOATING_PANE_EDGE_RESIZE_2026_05_29.md`
**Prior art:** #1132 (parked resize), #1159 (maximize-only, deferred this), #1173 (WM_SIZE resizes frontend child)
**Platform:** Windows. Linux/macOS floater resize is a separate platform-parity task (the floater is a Win32 `WS_POPUP`). All new code is `#[cfg(target_os = "windows")]` on the host; the frontend driver is cross-platform-ready (it only needs the `get/set_window_rect` IPC, which currently no-ops off Windows).

---

## §0 — What shipped: JS-driven resize (not the forwarder)

The `HTTRANSPARENT` forwarder (Phase 1) was implemented and **abandoned**: on cef-rs 146 the embedded Chromium child consumes `WM_NCHITTEST` before any child-wndproc subclass sees it, so the forwarder got **zero** hits (confirmed with live `target:"edge-resize"` diagnostics). A follow-on `WM_SYSCOMMAND(SC_SIZE)` attempt also failed — the post succeeds, but the native resize modal loop can't read the mouse because Chromium holds the OS capture from the DOM pointerdown, and the host's `ReleaseCapture` runs on the IPC thread (cross-thread → no-op).

**Shipped instead — JS-driven resize**, mirroring the already-working JS header MOVE:

1. The floater's frontend DOM (`floating-pane-workspace.tsx`) detects a pointerdown within `FLOATER_EDGE_RESIZE_BORDER` (8 CSS px, `frontend/app/workspace/floater-resize.ts`) of an edge/corner.
2. It takes **pointer capture** (so it keeps getting moves after the cursor leaves the window), reads the start rect via `get_window_rect`, and on each move computes the new rect (cursor delta + which of the 8 edges) and calls `set_window_rect` → `SetWindowPos`. Moves coalesce one-IPC-in-flight (last wins). No native loop, no NCHITTEST/capture conflict.
3. New host IPC: `get_window_rect` / `set_window_rect` in `commands/window/motion.rs`.

**Browser floaters** have a second OS child (the web-content window) layered over the frontend DOM in the content region, which would cover the grab band. `use-pane-rect-sync.ts` therefore insets that child by the band depth on the three window-edge sides (left/right/bottom — the top edge is over the 33px header, already frontend), exposing the band. The full-size placeholder div paints the strip, so there's **no native white border** (the failure mode the `WS_THICKFRAME` design hit).

**Note (2026-07-27):** `FLOATER_EDGE_RESIZE_BORDER` shipped at 12px here, was shrunk to 4px in PR #1829 as a side effect of fixing a browser-pane-only visual complaint (the inset above reading as a border around web content), and was restored to 8px after the resulting every-pane-type grab-target regression was diagnosed — see `docs/retro/retro-floating-pane-resize-hit-target-2026-07-27.md`.

The Phase 1 forwarder design below is kept for the record.

---

## Background (one paragraph)

The floater's parent wndproc (`floating_pane.rs::floating_pane_wndproc`) already maps a 6px (`RESIZE_BORDER_CSS`, DPI-scaled) edge border to `HTLEFT/HTRIGHT/HTTOP/HTBOTTOM/HT*CORNER` in `WM_NCHITTEST`, and the window is `WS_THICKFRAME` — so native resize *would* run. It doesn't, because the embedded CEF child HWND(s) fill the client area including that border and receive the edge mouse events first; the parent's `WM_NCHITTEST` zones are never reached. The fix is a child-wndproc that returns `HTTRANSPARENT` over the border so hit-testing falls through to the parent.

---

## Phase 1 — `HTTRANSPARENT` hit-test forwarder (the enabler)

### 1.1 New module: `agentmux-cef/src/floating_pane/resize_forwarder.rs`

(Or a function in `floating_pane.rs`; a small module keeps it isolated.) Models the subclass pattern on `browser_pane/hwnd.rs::install_browser_pane_focus_redirect`.

```rust
// Win32-only.
// Per-child-HWND context: the owning floater's outer HWND, so the hook can
// hit-test against the FLOATER rect (not the child's own rect).
static FLOATING_RESIZE_FORWARDER_CTX: Lazy<Mutex<HashMap<usize /*child hwnd*/, isize /*floater hwnd*/>>> = ...;

/// Subclass `child_hwnd`'s wndproc so cursor positions inside the owning
/// floater's resize border report `HTTRANSPARENT` — falling through to the
/// floater's own WM_NCHITTEST (HT{LEFT,…}) so the native resize loop runs.
/// Everywhere else, delegates to the original wndproc unchanged.
pub(crate) unsafe fn install_floating_resize_forwarder(
    child_hwnd: *mut c_void,
    floater_hwnd: *mut c_void,
);
```

The hook (`wndproc_hook`), on `WM_NCHITTEST`:
1. Look up the floater HWND for `child_hwnd` in the context map (or `GetAncestor(child, GA_ROOT)` and verify class == `AgentMuxFloatingPane`).
2. `GetWindowRect(floater)` → compute the border via the **shared** `RESIZE_BORDER_CSS` (6) scaled by `GetDpiForWindow(floater)` — factor the border math out of `floating_pane_wndproc` into a shared `fn in_resize_border(floater_rect, x, y, dpi) -> bool` so the child strip and the parent strip are pixel-identical.
3. If the cursor (`lparam` screen x/y) is in the border → `return HTTRANSPARENT`.
4. Otherwise → `CallWindowProcW(original, …)` (normal CEF/JS handling — clicks, header-drag, content all unchanged).

All other messages delegate to the original wndproc.

### 1.2 Where to install

- **Frontend browser child** — install right after the floater's browser is embedded. The floater browser is registered under the floater label; install from the floater's `on_after_created` path (where we already have the floater label + can resolve the child HWND via `host.window_handle()` walked up to the direct child of the floater), passing `floater_hwnd` = the outer popup HWND.
- **Web-content child (browser panes)** — when `CreateBrowserPaneTask` parents a web-content browser to a floater (`window_label` starts with `floating-`), install the forwarder on that child too, with the same `floater_hwnd`. For terminal/agent floaters there's only the frontend child.

Both children resolve `floater_hwnd` identically; with both forwarding, an edge over either child falls through to the parent.

### 1.3 Cleanup

Mirror the focus-redirect subclass: remove the context entry on `on_before_close` for the floater (and/or when the child HWND is destroyed). The subclass itself dies with the HWND (we don't need to restore the original wndproc — same as the existing focus-redirect hook). Guard the hook against a missing context entry (delegate to original).

### 1.4 Acceptance

- Edge-hover over a floater (any of the 8 zones) shows the resize cursor; drag resizes the outer window.
- `WM_SIZE` (already in #1173) resizes the frontend child to the new client rect; the frontend reflows and repositions its web-content child below the header.
- Header-drag (JS-driven), content clicks, scroll, and typing are unaffected (only the 6px border returns `HTTRANSPARENT`).

### 1.5 Risk + mitigation (the reason #1132 deferred this)

The forwarder changes child `WM_NCHITTEST` returns — the same surface header-drag and **redock** interact with. Mitigations:
- Transparent return is scoped strictly to the 6px border; interior stays `HTCLIENT`, header region unchanged.
- Redock's `resolve_window_at_cursor` walks **top-level** windows and matches `window_hwnds` — it doesn't depend on child hit-testing, so a child `HTTRANSPARENT` at the border shouldn't change top-level resolution. **Must be smoke-verified**: tear-off → redock still works, and the just-stabilized redock-load path is unaffected.

---

## Phase 2 — keep the reducer's normal-rect honest (`ReportNormalRect`)

**Honest scope:** Phase 1 + the existing button-maximize already compose. `toggle_floating_maximize` reads `GetWindowRect` at click and stashes it as `last_known_normal_rect`, so "resize, then maximize, then restore" returns to the resized size **without** Phase 2. Phase 2 is correctness-hardening for placement state, not a hard dependency.

What Phase 2 adds:
- A `HostCommand::ReportNormalRect { label, rect }` arm that sets `pane_window_states[label].last_known_normal_rect = rect` **only while `placement == Normal`** (never overwrite the restore target while Maximized).
- Dispatched from the floater wndproc on **`WM_EXITSIZEMOVE`** (fires once when the user finishes a resize/move — deterministic, no debounce/timer, aligns with the no-timers rule), resolving the floater label from `window_hwnds` (reverse lookup) and the rect from `GetWindowRect`.
- Optional follow-on `ReportOSPlacementChange` (Win+Up / Aero-Snap maximize, which bypasses `toggle_floating_maximize`): on `WM_SIZE` with `wParam == SIZE_MAXIMIZED/SIZE_RESTORED`, reconcile `placement` so the button icon/state and the reducer agree. This is the `reducer/pane_window.rs` "later phase" made real.

**Why bother:** without it, an OS-driven resize/snap leaves `placement`/`last_known_normal_rect` stale, so a later button-maximize→restore can return to the wrong size, and (future) layout persistence would save the wrong rect. Land it with Phase 1 if we want resize to be fully reducer-consistent; defer if we only need the basic interaction.

---

## Phase 3 — resize dimension overlay (optional polish)

`SPEC_PANE_RESIZE_DIMENSION_OVERLAY_2026_05_26.md` shows a `WxH` badge while a **tile-layout splitter** drags (`layoutModel.isSplitterDragging()`). Extend to floater edge-resize:
- Floater wndproc emits `floating-pane:resizing` (true) on `WM_ENTERSIZEMOVE` and (false) on `WM_EXITSIZEMOVE`, carrying the live size; the frontend sets a signal that mounts the same badge component in the floater.
- Read-only (the spec already guarantees the badge "never affects hit-testing"), so it's orthogonal to Phase 1 and carries no resize/redock risk.

---

## Sequencing & PRs

1. **PR A — Phase 1 forwarder.** Self-contained; delivers the user-visible feature (edge-resize works). Smoke: resize all 8 zones, both pane types; regress redock + header-drag.
2. **PR B — Phase 2 `ReportNormalRect` (+ optional `ReportOSPlacementChange`).** Reducer + `WM_EXITSIZEMOVE` wiring + a reducer test (resize-then-maximize-then-restore returns to the resized rect; maximized state doesn't overwrite the normal rect).
3. **PR C — Phase 3 overlay.** Optional.

Each PR adds a `task changeset -- patch …`. Phase 1 is the one that actually unblocks the user; 2 and 3 are correctness/polish and can follow or be skipped.

## Files touched (summary)

| Phase | Files |
|---|---|
| 1 | `floating_pane.rs` (factor `in_resize_border`, install hook at embed), new `floating_pane/resize_forwarder.rs`, `browser_pane/creation.rs` (install on floater web-content child), `floating_pane.rs` close path (ctx cleanup) |
| 2 | `reducer/mod.rs` (`ReportNormalRect` cmd + DispatchOutput/handler wiring), `reducer/pane_window.rs` (handler), `floating_pane.rs` (`WM_EXITSIZEMOVE` dispatch), `reducer/tests.rs` |
| 3 | `floating_pane.rs` (enter/exit-size-move events), frontend overlay component + a `floaterResizing` signal |
