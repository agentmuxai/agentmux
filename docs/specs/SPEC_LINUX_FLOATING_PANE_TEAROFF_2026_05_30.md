# Linux floating-pane tear-off — implementation spec

**Date:** 2026-05-30
**Repo state:** `main` @ `51c3ba56` (v0.40.0)
**Author:** AgentU-asaf
**Status:** Spec ready to implement (one-file frontend change)
**Pairs with / refines:**
- [`SPEC_MACOS_FLOATING_PANE_TEAROFF_2026_05_29.md`](./SPEC_MACOS_FLOATING_PANE_TEAROFF_2026_05_29.md) — the macOS sibling that just landed in #1182. Linux Phase A is the same shape.
- [`SPEC_FLOATING_PANE_TEAROFF_CROSS_PLATFORM_2026-05-26.md`](./SPEC_FLOATING_PANE_TEAROFF_CROSS_PLATFORM_2026-05-26.md) §3.3 (Linux/GTK recipe).
- [`docs/analysis/ANALYSIS_FLOATING_PANE_LINUX_GAPS_2026-05-28.md`](../analyses/ANALYSIS_FLOATING_PANE_LINUX_GAPS_2026-05-28.md) — analysis from two days ago that called out the missing routing.

---

## Problem (user report)

> "macOS now has refined tab/pane tear-off. Does that apply automatically to Linux?"

**No — and the gap is a one-file frontend change.**

On Linux today, tearing a pane off produces a **full workspace window** (tab bar + widget bar). On Windows and macOS (post-#1182), tearing a pane off produces a **chromeless floating window** — just the pane. Linux still routes pane tear-off through the legacy `openTearOffWindow → openWindowAtPosition` cold path.

---

## Root cause (precise)

Tracing the chain on Linux today:

1. **`frontend/app/drag/CrossWindowDragMonitor.linux.tsx::performTearOff` (lines 135-138)** — the pane branch calls `openTearOffWindow(api, newWsId, screenX, screenY, window.outerWidth, window.outerHeight)`. **It does NOT call `open_floating_pane_window`.**

2. **`frontend/app/drag/tear-off-pool-helper.ts::openTearOffWindow`** — tries the warm pool, falls through to `api.openWindowAtPosition()` (cold path). Neither carries a `floatingPaneId` URL param.

3. **`agentmux-cef/src/commands/drag.rs::open_window_at_position`** — mints `window-<uuid>`, builds URL with `ipc_port + ipc_token + windowLabel + workspaceId`. **No `floatingPaneId`.**

4. **`frontend/app/app.tsx:375-381`** — `IS_FLOATING_PANE = new URLSearchParams(location.search).has("floatingPaneId")`. With no `floatingPaneId`, the render switch falls back to `<Workspace />` (full chrome) instead of `<FloatingPaneWorkspace />` (chromeless).

So the Linux floater opens at the right backend window, just with the wrong frontend render mode. Same exact mechanism as the macOS gap pre-#1182.

---

## What's already in place on Linux (no work needed)

| Layer | Component | Status on Linux today |
|---|---|---|
| Backend window creation | `agentmux-cef/src/commands/floating_pane.rs` non-Windows branch | ✅ **Already implemented.** #1182 widened the non-Windows branch from `"not yet implemented"` to a real implementation: it calls the SAME CEF Views `post_create_window(frameless=true)` path the legacy tear-off uses, but appends `&floatingPaneId=<pane_id>` to the URL. The branch is `#[cfg(not(target_os = "windows"))]` — runs on Linux identically to macOS. |
| Secondary windows are frameless | `agentmux-cef/src/commands/window/creation.rs` — `frameless = true` for tear-off windows | ✅ **Already true on Linux.** Same code path as macOS — both go through `window_create_top_level(... frameless=true)`. |
| Chromeless renderer | `frontend/app/workspace/floating-pane-workspace.tsx` | ✅ **Platform-agnostic.** Zero `cfg`/platform branches. Renders `<TabContent>` only — no `<WindowHeader>`, no `<SystemStatus>`. Switched in via `?floatingPaneId=` URL param. |
| Render switch | `frontend/app/app.tsx::IS_FLOATING_PANE` | ✅ **Platform-agnostic.** |
| Backend tear-off saga | `agentmux-srv/src/server/service.rs::TearOffBlock` | ✅ **Platform-agnostic.** Was already correct before #1182. |
| Source-pane size measurement | `measureSourcePaneSize()` helper | ✅ **Already ported into `.linux.tsx`** in the stashed work from May 28 (`stash@{1}`), and was independently added in this same PR's earlier commit on Linux. |

**Therefore, to put Linux at parity with macOS+Windows: copy the darwin routing change in `CrossWindowDragMonitor.darwin.tsx::performTearOff` (the pane branch) into `CrossWindowDragMonitor.linux.tsx`. No backend work. No new files.**

---

## The decisive insight

This is exactly the same insight as the macOS spec §"The decisive insight" — the chromeless rendering is purely a function of `?floatingPaneId=` in the URL. Linux secondary windows are already frameless via CEF Views (same code path as macOS). The backend already accepts `open_floating_pane_window` on non-Windows. The frontend renderer is already platform-agnostic.

The only missing wire is in `CrossWindowDragMonitor.linux.tsx::performTearOff`.

---

## The change

**One file:** `frontend/app/drag/CrossWindowDragMonitor.linux.tsx`.

Replace the current pane branch (lines ~135-138):

```ts
if (dragType === "pane" && payload.blockId) {
    const newWsId = await WorkspaceService.TearOffBlock(payload.blockId, sourceTabId, sourceWsId, true);
    if (newWsId) await openTearOffWindow(api, newWsId, screenX, screenY, window.outerWidth, window.outerHeight);
}
```

with the macOS-mirror that #1182 landed in `.darwin.tsx`:

```ts
if (dragType === "pane" && payload.blockId) {
    // PANE → chromeless floating window (just the pane: no tab bar, no
    // widget bar). Mirrors the Windows and macOS pane branches.
    // `TearOffBlock` moves the block into a fresh backend workspace+tab;
    // the floating window's `initApp` → `initHostNewWindow` path attaches
    // to it via `?workspaceId=`, and the `?floatingPaneId=` URL param
    // makes the frontend render `<FloatingPaneWorkspace>` (chromeless)
    // instead of `<Workspace>`. Backend creates the frameless top-level
    // via `post_create_window(frameless=true)` — the same CEF Views path
    // the legacy tear-off used — so on Linux today this just changes the
    // URL the new window loads, nothing more.

    // Snapshot the source pane's rendered size BEFORE TearOffBlock —
    // that mutation unmounts the source DOM element.
    const { width: floaterWidth, height: floaterHeight } = measureSourcePaneSize(
        payload.blockId,
    );

    const newWsId = await WorkspaceService.TearOffBlock(
        payload.blockId,
        sourceTabId,
        sourceWsId,
        true,
    );
    if (!newWsId) {
        Logger.error("dnd:cross", "TearOffBlock returned no workspace id", {
            blockId: payload.blockId,
        });
        return;
    }

    // CRITICAL: invoke the IPC FIRST, then mutate the layout on success.
    // If we deleted the layout node up front and the IPC failed (e.g. the
    // H.7 mid-close gate rejects), the pane would be orphaned — still in
    // `blockids` but with no layout node and no floater. Reagent P1 on
    // PR #1073 (Windows path).
    try {
        await invokeCommand<{ window_label: string }>("open_floating_pane_window", {
            pane_id: payload.blockId,
            workspace_id: newWsId,
            x: screenX,
            y: screenY,
            width: floaterWidth,
            height: floaterHeight,
        });
        Logger.info("dnd:cross", "floating pane spawned", {
            blockId: payload.blockId,
            newWsId,
            screenX,
            screenY,
        });
    } catch (err) {
        Logger.error("dnd:cross", "open_floating_pane_window failed", {
            error: String(err),
            blockId: payload.blockId,
            newWsId,
        });
        // Don't try to undo TearOffBlock — the source layout was already
        // mutated server-side. Logging at error level is enough; the user
        // will see a missing pane and can drag it back.
    }
}
```

The `measureSourcePaneSize()` helper already exists in `.linux.tsx` from PR #1137's neighborhood — same constants and DOM selector as the win32 sibling. Verify it's still there before landing.

**Tab branch:** unchanged. Tab tear-off continues to spawn a full top-level instance (with its own taskbar entry) via `openTearOffWindow` — matches macOS and Windows behavior.

---

## What this does NOT do (deliberately)

- **No owned-window lifecycle on Linux yet** (Phase B of the cross-platform spec — Gtk `transient-for` + `skip-taskbar-hint` + `destroy-with-parent`). The Linux floater opens as a regular CEF Views top-level window. It WILL appear in the taskbar/Alt-Tab. Min/restore/destroy with the parent does NOT cascade. **This is the same Phase A scope macOS shipped in #1182** — the Phase B owned-window-lifecycle is tracked separately.
- **No JS-driven header drag.** The pane is shown chromeless; if it had a custom header we'd need to plumb the JS drag through `start_window_drag` (already working on Linux post-#1180). For Phase A, the chromeless `<FloatingPaneWorkspace>` doesn't render its own header — drag is via the standard CEF Views window border. (If we want a draggable in-pane header on Linux later, plumb through `start_window_drag` — that wiring is the same as the macOS Phase B.2 header-drag fix.)
- **No redock onto a window.** That's the macOS Phase C work in #1185 (drop a floater onto a window to merge the pane back). Linux Phase C tracks separately.

---

## Risk audit

| Risk | Mitigation |
|---|---|
| Backend rejects with `"not yet implemented"` on Linux | Already addressed — #1182 widened `#[cfg(not(target_os = "windows"))]` to a real implementation. Confirmed by reading current `agentmux-cef/src/commands/floating_pane.rs`. |
| `measureSourcePaneSize` missing on Linux | Verify import + helper presence in `.linux.tsx` before commit. Helper was added in earlier work (stash `wip: floating-pane linux fix`). If absent, port verbatim from `.win32.tsx`. |
| `floatingPaneId` URL param ignored by Linux frontend | No — `IS_FLOATING_PANE` is read from `URLSearchParams` in `app.tsx`, which is pure DOM API. Works identically across CEF browsers. |
| Window appears at wrong size on Linux due to DPI | macOS spec §"The decisive insight" notes: CEF Views positions/sizes in DIP (logical px); `getBoundingClientRect()` and DOM `screenX/Y` are already in DIP. The Windows-only block above the cross-platform code does cross-monitor DPI scaling for Win32. Linux is fine without it on single-monitor or uniform-DPI setups. Cross-monitor mixed-DPI handoff is the same follow-up macOS has (Phase B). |
| Pool warm window race (the `openTearOffWindow` pool path is Windows-only on the host — `window_pool.rs:454-459` early-returns on non-Windows) | We're switching OFF the pool path on Linux. No regression — Linux was already always cold-pathing today. |
| Source layout not updating after `TearOffBlock` (Bug 2 in the May 28 analysis — "pane stayed in parent") | This change does NOT depend on solving that bug. If it's still present, this PR's symptom is the same as macOS post-#1182 (which presumably has the same risk and was deemed acceptable). Investigation tracked separately. |

---

## Test plan

- [ ] Linux: drag a CPU widget pane out of a tab onto empty desktop. Verify:
  - New window appears at the **pane's** size (not the parent window's outer size).
  - New window is **chromeless** — no tab bar, no widget bar; just the CPU pane content.
  - Host log shows `[ipc] open_floating_pane_window … floatingPaneId=…` and `floating pane spawned`.
- [ ] Linux: drag a tab out of the tab bar. Verify:
  - Behavior unchanged — full top-level workspace window with tab bar + widget bar (tab tear-off, not pane tear-off).
- [ ] Linux: open the new floater's URL bar in devtools (Cmd+Shift+I or remote-debugging-port=9222). Confirm URL contains `?floatingPaneId=<pane_uuid>&workspaceId=<ws_uuid>`.
- [ ] Linux regression: tear off a pane while two windows exist. Verify no crash (relates to the PR #881 / #1137 multi-window territory).
- [ ] macOS: no change expected. `.darwin.tsx` untouched.
- [ ] Windows: no change expected. `.win32.tsx` untouched.

---

## Out of scope (Phase B+, tracked separately)

- Linux owned-window lifecycle (Gtk `transient-for` + `skip-taskbar-hint` + `destroy-with-parent`) — the equivalent of the Win32 `WS_POPUP | WS_EX_TOOLWINDOW` owner-HWND and the macOS Phase B `NSPanel + addChildWindow:ordered:`. Big piece of work, tracked under the original cross-platform spec §3.3.
- Linux redock (drop a floater onto a window to merge). Mirrors macOS Phase C (#1185).
- JS-driven in-pane header drag on Linux. Plumbs through the existing `start_window_drag` IPC (now working post-#1180).
- Cross-monitor DPI handoff. Currently no-ops on Linux; same as macOS Phase A.

---

## References

- `frontend/app/drag/CrossWindowDragMonitor.linux.tsx` — file to change.
- `frontend/app/drag/CrossWindowDragMonitor.darwin.tsx` — reference implementation, just landed in #1182.
- `frontend/app/drag/CrossWindowDragMonitor.win32.tsx` — original reference (Windows pane tear-off path).
- `frontend/app/workspace/floating-pane-workspace.tsx` — chromeless renderer (platform-agnostic).
- `frontend/app/app.tsx::IS_FLOATING_PANE` — render switch (platform-agnostic).
- `agentmux-cef/src/commands/floating_pane.rs` — backend IPC handler. Non-Windows branch is now a real implementation post-#1182.
- `agentmux-cef/src/commands/window/creation.rs::window_create_top_level` — frameless CEF Views window creation (already used for tear-off on macOS+Linux).
- `docs/specs/SPEC_MACOS_FLOATING_PANE_TEAROFF_2026_05_29.md` — sibling macOS spec, deeper background.
- `docs/specs/SPEC_FLOATING_PANE_TEAROFF_CROSS_PLATFORM_2026-05-26.md` — original cross-platform recipes (Linux/GTK section is Phase B+ scope).
- `docs/analysis/ANALYSIS_FLOATING_PANE_LINUX_GAPS_2026-05-28.md` — earlier Linux-gaps analysis, called out exactly this routing.
