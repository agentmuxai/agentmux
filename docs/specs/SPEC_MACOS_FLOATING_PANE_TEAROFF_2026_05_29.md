# macOS floating-pane tear-off — implementation spec

**Date:** 2026-05-29
**Repo state:** `main` @ `7e61fda3` (v0.40.0)
**Author:** AgentO-asaf
**Status:** Spec ready to implement (phased)
**Supersedes/refines:** [`SPEC_FLOATING_PANE_TEAROFF_CROSS_PLATFORM_2026-05-26.md`](./SPEC_FLOATING_PANE_TEAROFF_CROSS_PLATFORM_2026-05-26.md) §3.2 (the macOS "C1" work block) — this spec refines the phasing after discovering that AgentMux's secondary windows are already frameless CEF Views windows, which makes the user-visible fix far cheaper than the NSPanel rewrite that spec assumed.
**Related:** macOS bring-up PRs #1131, #1169, #1170, #1171, #1172, #1175 (all merged); the C1 work block remains open.

---

## Problem (user report)

> "Tearing a pane is supposed to give a floating window, which works on Windows. On macOS, tearing a pane creates a new window **with a tab bar and widgets bar** — we need just the pane."

On Windows, tearing a pane off produces a **chromeless floating window**: just the pane content, no tab bar, no action-widget bar. On macOS, tearing a pane produces a **full workspace window** with the complete title-bar chrome (tab bar + action widgets). The two platforms diverge entirely in the tear-off code path.

---

## Root cause (precise)

The macOS pane tear-off never routes to the floating-pane rendering path. Tracing the chain:

1. **`frontend/app/drag/CrossWindowDragMonitor.darwin.tsx:147-163`** — `performTearOff()` calls `openTearOffWindow()` for **both** panes and tabs. It never calls `open_floating_pane_window`. (Windows branches: panes → `open_floating_pane_window`, tabs → `openTearOffWindow` — see `CrossWindowDragMonitor.win32.tsx:275-355`.)

2. **`frontend/app/drag/tear-off-pool-helper.ts:43-64`** — `openTearOffWindow()` tries the warm pool (Windows-only; the pool init is `#[cfg(not(target_os = "windows"))] return` in `window_pool.rs:454-459`, so on macOS it's a no-op), then falls through to the cold path `api.openWindowAtPosition()`.

3. **`agentmux-cef/src/commands/drag.rs:374-451`** — `open_window_at_position()` mints a `window-<uuid>` label and builds the URL with `ipc_port`, `ipc_token`, `windowLabel`, and (optionally) `workspaceId` — **never `floatingPaneId`**.

4. **`frontend/app/app.tsx:375-381,422-427`** — `IS_FLOATING_PANE = new URLSearchParams(location.search).has("floatingPaneId")`. With no `floatingPaneId`, the render switch falls back to `<Workspace />` (full tab + widget chrome) instead of `<FloatingPaneWorkspace />` (chromeless).

5. **`agentmux-cef/src/commands/floating_pane.rs:155-166`** — even if the darwin monitor *did* call `open_floating_pane_window`, its `#[cfg(not(target_os = "windows"))]` branch returns `"not yet implemented on this platform"`.

So: macOS pane tear-off has **zero routes** to the chromeless rendering. It always lands in the regular-workspace-window pipeline.

---

## The decisive insight (why this is cheaper than the prior spec assumed)

`SPEC_FLOATING_PANE_TEAROFF_CROSS_PLATFORM_2026-05-26.md` §3.2 prescribed a ~400-LOC `agentmux-cef/src/floating_pane_macos.rs` using a raw `NSPanel` + `addChildWindow:ordered:` + `CefWindowInfo::SetAsChild`, mirroring the Windows raw-Win32 floating window. That's because the Windows floating pane **bypasses CEF Views entirely** (raw `CreateWindowExW` with `WS_POPUP | WS_EX_TOOLWINDOW`, custom WNDPROC suppressing the title bar).

But macOS doesn't need that to satisfy the user's actual request, because of two facts the prior spec didn't lean on:

1. **macOS secondary (tear-off) windows are ALREADY frameless.** They're created via CEF Views `window_create_top_level` with `frameless = true` (`commands/window/creation.rs:333` — *"true = frameless: secondary app windows use the same custom title bar as main"*; `app.rs:309-310`, `app.rs:140 fn is_frameless`). There is **no native macOS title bar** on a tear-off window. The chrome the user sees — the tab bar and widget bar — is **pure frontend**: `<Workspace>` renders `<WindowHeader>` (tab bar) + `<SystemStatus>` (action widgets).

2. **The chromeless renderer is 100% platform-agnostic.** `frontend/app/workspace/floating-pane-workspace.tsx` has **zero** `#[cfg]`/platform branching. It renders `<TabContent>` only — no `WindowHeader`, no `SystemStatus` — purely off the `?floatingPaneId=` URL param. The pane/tab branch in the drag monitor, and the backend sagas (`TearOffBlock`, `RedockFloatingPane` in `agentmux-srv`), are also platform-agnostic.

**Therefore: to give the user "just the pane" on macOS, all that's structurally required is for the torn-off window's URL to carry `?floatingPaneId=`.** The window is already frameless; the frontend already knows how to render chromeless. No AppKit. No NSPanel. No new native window primitive.

The NSPanel work the prior spec described is still needed — but only for *behavioral* parity (owned-window lifecycle, JS-driven header drag, redock), not for the user-visible "just the pane" result. This spec splits those apart into Phase A and Phase B.

---

## What's already cross-platform (no macOS work)

| Layer | Component | Status |
|---|---|---|
| Frontend render | `floating-pane-workspace.tsx` (`<FloatingPaneWorkspace>`) | Platform-agnostic; renders chromeless from `floatingPaneId` |
| Frontend switch | `app.tsx` `IS_FLOATING_PANE` → `<Show>` | Platform-agnostic |
| Frontend branch | drag monitor pane-vs-tab decision (`payload.kind === "tile"`) | Platform-agnostic logic; just needs wiring in `.darwin.tsx` |
| Backend RPC | `WorkspaceService.TearOffBlock` (saga `agentmux-srv/src/sagas/tear_off_block.rs`) | Platform-agnostic state mutation |
| Backend RPC | `WorkspaceService.RedockFloatingPane` (saga `redock_floating_pane.rs`) | Platform-agnostic state mutation |
| Host command | `clear_floating_redock_hover` | Already platform-agnostic (`motion.rs:353-369`) |
| Host command | `update_floating_redock_hover` dispatch | Platform-agnostic shell; depends on `resolve_window_at_cursor` for the target |
| Native window | secondary windows via CEF Views `window_create_top_level(frameless=true)` | Already frameless on macOS |

## What's macOS-specific work

| Item | File | Needed for |
|---|---|---|
| Route pane tear-off to a `floatingPaneId` window | `CrossWindowDragMonitor.darwin.tsx` | **Phase A** (the user's ask) |
| `open_floating_pane_window` macOS branch (or `floatingPaneId` injection into the cold path) | `commands/floating_pane.rs` / `commands/drag.rs` | **Phase A** |
| `get_window_position` macOS impl | `commands/window/motion.rs:32-44` (Windows-only; non-win returns `{x:0,y:0}`) | Phase B (drag) |
| `set_window_position` macOS impl | `motion.rs:97-126` (Windows `SetWindowPos`; non-win → `ui_tasks::post_set_window_position`) | Phase B (drag) |
| `resolve_window_at_cursor` macOS impl | `motion.rs:191-293` (Windows Z-order walk; non-win returns nulls) | Phase B (redock) |
| `get_cursor_point` macOS impl | `commands/drag.rs:200-213` (Windows `GetCursorPos`; non-win returns `{0,0}`) | Phase B (drag/redock) |
| Owned-window lifecycle (follows parent, minimize/close cascade) | new — CEF Views parent or `addChildWindow` | Phase B (polish) |

---

## Phase A — "just the pane" (small, AppKit-free, ships the user's request)

**Goal:** macOS pane tear-off produces a frameless window whose content is only the pane (no tab bar, no widget bar). Window is independent (not yet owned/follows-parent), positioned at the drop point, draggable via the OS however a frameless CEF Views window normally is. This is the minimal change that resolves the user report.

### A.1 — Route panes through a `floatingPaneId` window in the darwin monitor

In `CrossWindowDragMonitor.darwin.tsx::performTearOff` (currently `:147-163`), branch on `dragType` mirroring `win32.tsx`:

```ts
async function performTearOff(dragType, payload, sourceWsId, sourceTabId, screenX, screenY) {
    const api = getApi();
    if (dragType === "pane" && payload.blockId) {
        // PANE → chromeless floating window (Phase A: frameless CEF Views + floatingPaneId)
        const newWsId = await WorkspaceService.TearOffBlock(payload.blockId, sourceTabId, sourceWsId, true);
        if (newWsId) {
            await api.openFloatingPaneWindow({
                paneId: payload.blockId,
                workspaceId: newWsId,
                x: screenX, y: screenY,
                width: /* captured pane size */, height: /* captured pane size */,
            });
        }
    } else if (dragType === "tab" && payload.tabId) {
        // TAB → full workspace window (unchanged)
        const newWsId = await WorkspaceService.TearOffTab(payload.tabId, sourceWsId);
        if (newWsId) await openTearOffWindow(api, newWsId, screenX, screenY);
    }
}
```

(The `cef-api.ts` `openFloatingPaneWindow` wrapper already exists for the win32 path — reuse it.)

### A.2 — Implement `open_floating_pane_window` for macOS

Two options for the host side. **Recommended: Option 1** (reuse the proven CEF Views frameless path; defer raw AppKit to Phase B).

**Option 1 — CEF Views frameless window + `floatingPaneId` URL (recommended for Phase A).**
In `commands/floating_pane.rs`, replace the macOS `Err("not yet implemented")` with a path that creates a frameless top-level window (the same `window_create_top_level(frameless=true)` machinery the cold tear-off path already uses) but whose URL carries `&floatingPaneId=<pane_id>&windowLabel=floating-<uuid>&workspaceId=<ws>`. Concretely, factor the URL+window creation out of `open_window_at_position` (`drag.rs:374-451`) into a helper that takes an optional `floating_pane_id`, and have `open_floating_pane_window` call it with the id set. The window is frameless (already true for secondary windows); the frontend renders `<FloatingPaneWorkspace>` because `floatingPaneId` is present. Label `floating-<uuid>` keeps it distinguishable from `window-<uuid>` (consistent with Windows + the orphan-reconciliation exclusion at `window/lifecycle.rs:301`).

- Pros: ~30 lines, no AppKit, reuses a battle-tested path, immediately delivers "just the pane."
- Cons: window is independent (no owned-window cascade) and gets whatever drag/positioning a normal frameless CEF Views window has. Phase B adds the owned-window + header-drag + redock parity.

**Option 2 — raw `NSPanel` + `addChildWindow` (the prior spec's C1; defer to Phase B).**
Build `agentmux-cef/src/floating_pane_macos.rs` per `SPEC_FLOATING_PANE_TEAROFF_CROSS_PLATFORM_2026-05-26.md` §3.2: an `NSPanel` (`[.titled,.closable,.resizable,.utilityWindow,.nonactivatingPanel,.fullSizeContentView]`), `parentWindow.addChildWindow(panel, ordered:.above)`, `CefWindowInfo::SetAsChild(panel.contentView, rect)`. ~400 LOC. This is full parity but unnecessary for the user-visible "just the pane" fix.

> Decision: ship Option 1 as Phase A. It satisfies the user report with minimal risk. Pursue Option 2's owned-window semantics only if/when the independent-window behavior proves insufficient (see Phase B).

### A.3 — Acceptance (Phase A)

- macOS: drag a pane out → a frameless window appears at the drop point showing **only** the pane (its `BlockFrame` header is the sole chrome; no tab bar, no widget bar). `muxlog host` shows the `floating-<uuid>` label and a URL containing `floatingPaneId`.
- macOS: drag a **tab** out → still a full workspace window (unchanged).
- Windows: unchanged (already uses `open_floating_pane_window`).
- Linux: same routing applies (see "Linux" below).

---

## Phase B — behavioral parity (owned window, header drag, redock)

Phase A gives the right *content*. Phase B makes the floater *behave* like the Windows one. Each piece is independent and can land separately.

### B.1 — Owned-window lifecycle (follows parent, minimize/close cascade)

Windows gets this free from `WS_EX_TOOLWINDOW` + owner HWND. macOS options:
- If Option 1 (CEF Views) shipped in Phase A: add `addChildWindow:ordered:` on the underlying `NSWindow` of the CEF Views window (reachable via `cef::Window` → native handle), with `.nonactivatingPanel`-equivalent behavior. May require dropping to AppKit for the `addChildWindow` call even though the window itself is CEF Views.
- If Option 2 (NSPanel) is chosen: owned-window semantics come free from `addChildWindow` + `.nonactivatingPanel` per the xplat spec.

### B.2 — JS-driven header drag

`floating-pane-workspace.tsx:92-386` already implements the drag (mousedown on `[data-role="block-header"]` → `get_window_position` → `set_window_position` with coalescing). It's platform-agnostic JS. It needs the host commands implemented for macOS:

- **`get_window_position`** (`motion.rs:32-44`): macOS — read the window's frame origin. Via CEF Views `cef::Window::bounds()` (cross-platform, already used by `MoveWindowTask` in `ui_tasks.rs`) or raw `[nswindow frame]`. Prefer `cef::Window::bounds()` if the floater is a CEF Views window.
- **`set_window_position`** (`motion.rs:97-126`): macOS — `cef::Window::set_bounds()` (cross-platform) or `[nswindow setFrameOrigin:]`. Note the non-Windows path already routes to `ui_tasks::post_set_window_position` — verify/extend that it actually moves the window on macOS via `set_bounds`.
- **`get_cursor_point`** (`drag.rs:200-213`): macOS — `[NSEvent mouseLocation]` (flip Y: AppKit origin is bottom-left; the redock math expects top-left screen coords — convert via main screen height).

### B.3 — Redock (drop onto another window merges the pane back)

`floating-pane-workspace.tsx:276-372` calls `resolve_window_at_cursor` then `RedockFloatingPane`. The saga is platform-agnostic; the resolver isn't:

- **`resolve_window_at_cursor`** (`motion.rs:191-293`): macOS — enumerate this process's windows front-to-back (`[NSApp orderedWindows]`), hit-test the cursor point against each window's frame (excluding `exclude_label`), return the top-most match's label + backend `window_id`. The Windows version walks Z-order via `WindowFromPoint` + `GetAncestor(GA_ROOT)`; the macOS version is the `orderedWindows` analogue.
- **`update_floating_redock_hover`** already dispatches cross-platform; once `resolve_window_at_cursor` works on macOS, the hover highlight works too.

### B.4 — Resize

Frameless CEF Views windows: confirm edge-resize works on macOS (CEF Views may already provide it; the Windows floater hand-rolls `WM_NCHITTEST` edge bands because raw Win32 has no default). If Option 1 windows don't resize, either enable CEF Views resize or add the macOS edge-hit-test equivalent. Lower priority — note it, don't block on it.

---

## Linux

The same Phase A routing applies to `CrossWindowDragMonitor.linux.tsx` (identical structure to darwin). Linux secondary windows are also frameless CEF Views. The host motion commands have the same Windows-only gating, so Phase B for Linux needs X11/Wayland implementations of `get/set_window_position`, `resolve_window_at_cursor`, `get_cursor_point` — out of scope here but the routing fix (A.1/A.2) is shared. Keep the `open_floating_pane_window` host branch `#[cfg(not(target_os = "windows"))]` so darwin + linux share it where possible, or split `#[cfg(target_os = "macos")]` / `#[cfg(target_os = "linux")]` if the window-creation primitives differ.

---

## Decisions / open questions

1. **Phase A window primitive:** Option 1 (CEF Views frameless + `floatingPaneId`) vs Option 2 (raw NSPanel). **Recommend Option 1** — minimal, AppKit-free, satisfies the user report. Revisit only if independent-window behavior is unacceptable before Phase B lands owned-window semantics.
2. **Can `addChildWindow` attach to a CEF-Views-owned NSWindow?** (B.1) — needs a spike. If not clean, owned-window semantics may force Option 2 for the final form.
3. **`cef::Window::bounds()/set_bounds()` for macOS positioning** (B.2) — verify these move a frameless secondary window on macOS before hand-rolling raw `NSWindow` frame math. `MoveWindowTask` (`ui_tasks.rs`) already uses them, suggesting they're cross-platform.
4. **Pane-size capture:** the darwin monitor must capture the dragged pane's rendered width/height to size the floater (Windows captures `window.outerWidth/Height` and the pane rect — `win32.tsx:293-295`). Port that capture; fall back to a sane default (e.g. the pane's `getBoundingClientRect`).

---

## Acceptance criteria (overall)

**Phase A (the user's request):**
- [ ] macOS: tear a pane → frameless window with **only the pane** (no tab bar, no widget bar).
- [ ] macOS: tear a tab → full workspace window (unchanged).
- [ ] Windows/Linux: tear behavior unchanged from current.

**Phase B (parity):**
- [ ] Floater follows/minimizes/closes with its source window.
- [ ] Dragging the pane header moves the floater (no prohibited-cursor regression — see #1175).
- [ ] Dropping the floater over another AgentMux window redocks the pane into it; dropping on the desktop leaves it standalone.
- [ ] Floater is edge-resizable.

---

## Work-block estimate

| Phase | Scope | Est. |
|---|---|---|
| A.1 + A.2 (Option 1) | darwin monitor branch + `open_floating_pane_window` macOS via CEF-Views-frameless + `floatingPaneId` URL | ~40-80 LOC, 1 PR |
| B.2 | `get/set_window_position` + `get_cursor_point` macOS (CEF Views `bounds` / NSEvent) | ~60 LOC |
| B.3 | `resolve_window_at_cursor` macOS (`orderedWindows` hit-test) | ~80 LOC |
| B.1 | owned-window lifecycle (`addChildWindow` or NSPanel) | ~100-400 LOC depending on Option 1-vs-2 |
| B.4 | resize | spike + ~0-80 LOC |

Phase A is the high-value, low-risk slice and should ship first as its own PR. Phases B.1-B.4 are independent follow-ups.
