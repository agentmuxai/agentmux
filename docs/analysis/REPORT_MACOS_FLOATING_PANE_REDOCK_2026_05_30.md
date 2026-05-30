# Report — macOS floating-pane REDOCK (drop a floater back onto a window)

**Date:** 2026-05-30
**Repo state:** `main` @ `c13ea931` (v0.40.0)
**Author:** AgentO-asaf
**Status:** Analysis + implementation plan (no code in this PR — report only)
**Builds on:** PR #1182 (macOS floating-pane tear-off — chromeless floater + draggable header), `docs/specs/SPEC_MACOS_FLOATING_PANE_TEAROFF_2026_05_29.md` (this is the Phase B.3 deep-dive that spec deferred)
**Related Windows work:** PR #1166 (deterministic redock-onto-main), the floating-pane suite #1073/#1077/#1159/#1166/#1177

---

## The gap (user report)

> "We also need to redock. Windows does it."

On **Windows**, after tearing a pane into a floating window you can drag that floater over another AgentMux window and drop it — the pane merges into that window's active tab and the (now-empty) floater auto-closes. A live highlight overlay shows the drop zone during the drag.

On **macOS**, redock does nothing: the floater drops wherever you release it and stays a standalone window. This report maps the full Windows redock flow, answers whether it integrates with the global reducer system, and specs exactly what macOS needs.

---

## Does it integrate with the global reducer system?

**Yes — across three cleanly separated layers. The *mutation* is fully reducer-integrated and already cross-platform; the *window resolution* reads host-side projections that aren't populated on macOS yet.**

### Layer 1 — Backend (agentmux-srv) reducer + WaveObject graph: the actual redock effect ✅ cross-platform

The redock *mutation* is a saga over the backend reducer:

- `WorkspaceService.RedockFloatingPane(sourceBlockId, sourceTabId, sourceWsId, targetTabId, targetWsId)` (frontend RPC, `frontend/app/store/services.ts`).
- → `agentmux-srv/src/sagas/redock_floating_pane.rs::run` → `run_saga("redock_floating_pane", …)` → `run_inner` dispatches a single atomic `Command::MoveBlock { block_id, src_tab_id, dst_tab_id, dst_index }` to the reducer.
- The reducer removes `block_id` from `source_tab.block_ids` and inserts it into `target_tab.block_ids`. Pure state mutation, no OS APIs.

The frontend also *reads* the same global object graph to resolve the target tab: `tryRedockAtCursor` (`frontend/app/workspace/floating-pane-workspace.tsx`) does `WOS.reloadWaveObject("window", window_id)` → `WaveWindow.workspaceid` → `WOS.reloadWaveObject("workspace", …)` → `Workspace.activetabid`. That's the WaveObject store — the global reducer's projected object graph.

**This layer has zero platform-specific code and already works on macOS.** Tearing off (PR #1182) already drives `TearOffBlock` through the same backend reducer; redock's `MoveBlock` is the symmetric operation.

### Layer 2 — Host (agentmux-cef) reducer: window lifecycle

Window creation/registration flows through `crate::reducer::HostCommand` (e.g. `EnqueuePendingWindowCreation`, `BrowserRegistered`). PR #1182's floating-window creation already dispatches `EnqueuePendingWindowCreation` here. Redock doesn't add host-reducer commands — it *reads* host state (next layer).

### Layer 3 — Launcher-event → host shadow projection: the macOS gap ⚠️

`resolve_window_at_cursor` (the host command that maps a cursor point → target window) returns `{ label, window_id }`, where `window_id` comes from `state.backend_window_id(label)`:

```rust
// agentmux-cef/src/state.rs:1143
pub fn backend_window_id(&self, label: &str) -> Option<String> {
    self.shadow_backend_window_ids.lock().get(label).cloned()
}
```

`shadow_backend_window_ids` is a host-side **projection of the launcher's authoritative `backend_window_ids` map**, fed exclusively by `Event::BackendWindowIdRegistered` through `launcher_ipc::apply_event_to_shadow`. The launcher is **Windows-only** (`task dev` on macOS/Linux invokes the host directly — see CLAUDE.md). On macOS:

- `register_backend_window` (`commands/window/meta.rs`) calls `report_backend_window_id_registered`, which sends a `Command` to `COMMAND_TX` — but `COMMAND_TX` is the launcher channel and is `None` without a launcher, so the call no-ops.
- `apply_event_to_shadow` is currently **dead code on macOS** (confirmed: `cargo build` warns `function 'apply_event_to_shadow' is never used`, `function 'apply_shadow_projection' is never used`).
- Therefore `shadow_backend_window_ids` is **always empty on macOS**, and `backend_window_id(label)` always returns `None`.

**Consequence:** even a perfect macOS `resolve_window_at_cursor` returning the right *label* would return `window_id: null`. The frontend then bails:

```ts
// floating-pane-workspace.tsx, tryRedockAtCursor
if (!target.label || !target.window_id) {
    // Cursor over desktop, external app, or our own floater — leave floater.
    return;
}
```

So macOS redock needs **both** a native hit-test *and* a working `label → backend window_id` lookup.

---

## The full Windows redock flow (reference)

### A. Frontend — `frontend/app/workspace/floating-pane-workspace.tsx`

- **During drag** (`onMouseMove` → `pushRedockHover`, ~50 ms throttle): sends cursor in **physical px** (`screenX × devicePixelRatio`) to `update_floating_redock_hover { source_label, x, y }`.
- **On drop** (`onMouseUp` → `tryRedockAtCursor`):
  1. `clear_floating_redock_hover {}` (tear down the highlight).
  2. `resolve_window_at_cursor { x: screenX×dpr, y: screenY×dpr, exclude_label: ourLabel }` → `{ label, window_id }`. `exclude_label` is our own floater (it follows the cursor, so it's always topmost at the drop point).
  3. If `label && window_id`: load target `WaveWindow` → `Workspace` → `activetabid`/`oid` via WaveObject; read source `sourceBlockId`/`sourceTabId`/`sourceWsId`; call `RedockFloatingPane(...)`.
  4. The `MoveBlock` empties the floater's source tab → the `createEffect` auto-close watcher closes the floater.

### B. Host — `resolve_window_at_cursor` (`agentmux-cef/src/commands/window/motion.rs`, Windows branch)

1. Snapshot `state.window_hwnds` (`HashMap<label, HWND isize>`), build a reverse `HWND → label` map.
2. Resolve `exclude_hwnd` from `exclude_label`; resolve `main_hwnd` via `find_main_window()` (cache-independent, fixes the redock-onto-main startup race — PR #1166).
3. Walk top-level windows front-to-back: `GetTopWindow` + `GetWindow(GW_HWNDNEXT)`. For each visible, same-PID, non-`WS_EX_TRANSPARENT` window whose `GetWindowRect` contains `(x,y)`:
   - if in the reverse map → return `{ label, window_id: backend_window_id(label) }`;
   - else if it's `main_hwnd` → return `{ "main", backend_window_id("main") }`;
   - else continue.
4. No hit → `{ label: null, window_id: null }`.

All coordinates are **physical px** (Win32 `GetWindowRect`).

### C. Host — hover broadcast (`update_floating_redock_hover` / `clear_floating_redock_hover`)

`update_floating_redock_hover` calls `resolve_window_at_cursor` internally, then `emit_event_to_top_level_windows("floating-redock:hover-state", { target_label, source_label, cursor_x, cursor_y })` (cursor in **physical px**). `clear_…` emits `{ target_label: null }` as a teardown sentinel.

### D. Frontend — highlight receiver (`frontend/app-init.ts`, `installFloatingRedockHoverListener`)

Each window listens for `floating-redock:hover-state`. If `target_label === myLabel`, it converts the cursor back to client CSS px (**`cursor_x / dpr − window.screenX`**), `document.elementFromPoint` → nearest `[data-blockid]` leaf, `determineDropDirection` → drop zone, and renders a placeholder overlay. Else clears.

### E. The `label → HWND → window_id` chain

- `window_hwnds[label] = HWND`: floaters register their outer HWND at creation (`floating_pane.rs`); main/tear-off windows register on "ready" (`capture_hwnd_for_label` in `window/lifecycle.rs`).
- `backend_window_id(label)`: from `shadow_backend_window_ids`, fed by launcher `Event::BackendWindowIdRegistered` (Layer 3 above).

---

## What macOS needs

Three pieces. None require new backend-reducer or saga work — the redock mutation (Layer 1) is already cross-platform.

### Gap 1 — `resolve_window_at_cursor` macOS hit-test (the headline)

Windows walks HWND Z-order; macOS has no `window_hwnds` (it's Windows-only — macOS uses `state.windows: HashMap<String, cef::Window>`, populated by `app.rs::on_window_created` keyed by label, **not** `#[cfg]`-gated away). Implement a macOS branch that:

1. Reads the cursor point in the same space the frontend sends. **Recommendation: send DIP** from the frontend on macOS (mirror PR #1182's `posScale()` — Windows physical px, macOS/Linux DIP `1:1`) so the host compares against CEF Views DIP bounds directly.
2. Enumerates this process's top-level windows **front-to-back**. Options:
   - **CEF Views**: iterate `state.windows`, read each `cef::Window::bounds()` (DIP) on the **UI thread** (same blocking-channel pattern PR #1182 added for `get_window_position` — see `ui_tasks::get_window_position_blocking`), hit-test the point, return the top-most match excluding `exclude_label`. Z-order among CEF Views windows: query `cef::Window::is_always_on_top`/focus, or fall back to AppKit ordering.
   - **AppKit**: `[NSApp orderedWindows]` is already front-to-back; map each `NSWindow` back to a label. Mapping NSWindow→label needs a registry (the CEF Views window's native handle, or a label↔NSWindow map maintained at creation). This gives true Z-order for free but adds an AppKit registry.
   - **Pragmatic first cut**: iterate `state.windows`, hit-test bounds, and if multiple match, prefer the most-recently-focused (track a `last_focused_label`). Good enough for the common case (windows rarely overlap exactly); refine to true Z-order if needed.
3. Returns `{ label, window_id: backend_window_id(label) }` — which requires Gap 2.

Reuse the existing `exclude_label` and `find_main_window`-style main fallback semantics. The UI-thread bounds read is the same primitive PR #1182 introduced, so this is a known pattern.

### Gap 2 — `label → backend window_id` on macOS (no launcher)

`shadow_backend_window_ids` is launcher-fed and empty on macOS. The frontend already hands the host the mapping via `register_backend_window(label, window_id)` (verified in PR #1182's logs: `[window] registered backend window ID label=floating-… window_id=…`). The fix: on non-Windows, have `register_backend_window` populate `shadow_backend_window_ids` **directly** (the host is the sole authority when there's no launcher), so `backend_window_id(label)` returns the id. ~5 lines, `#[cfg(not(target_os = "windows"))]`, in `commands/window/meta.rs`. Also remove the dead-code warnings by gating `apply_event_to_shadow`/`apply_shadow_projection` to Windows or wiring the non-Windows direct path.

### Gap 3 — hover-highlight coordinate space

The frontend highlight receiver (`app-init.ts`) converts the broadcast cursor via `cursor_x / dpr − window.screenX`, assuming the host emits **physical px**. On macOS the host would emit **DIP** (Gap 1 recommendation), so the `/ dpr` is wrong there. Make the receiver platform-aware (the inverse of `posScale()` — divide by `dpr` only on Windows; on macOS/Linux use the cursor as-is, `cursor − window.screenX`). Same one-line platform factor as PR #1182. Without this, the drop-zone highlight lands in the wrong place on Retina even once Gap 1 resolves.

### Already cross-platform (no work)

- `RedockFloatingPane` saga + `Command::MoveBlock` reducer (Layer 1).
- `update_floating_redock_hover` / `clear_floating_redock_hover` dispatch shells (they call `resolve_window_at_cursor` and broadcast — they work the moment Gap 1 returns a real target).
- The frontend `tryRedockAtCursor` + auto-close watcher (platform-agnostic).
- WaveObject target-tab resolution.

---

## Coordinate-space summary (the recurring trap)

| Hop | Windows | macOS (recommended) |
|---|---|---|
| Frontend → host cursor | CSS px × `devicePixelRatio` (physical) | CSS px × 1 (DIP) — via `posScale()` |
| Host window bounds | `GetWindowRect` (physical px) | `cef::Window::bounds()` (DIP, UI-thread read) |
| Host → frontend hover cursor | physical px | DIP |
| Frontend hover receiver | `cursor / dpr − screenX` | `cursor − screenX` (no `/dpr`) |

The single consistent rule: **Windows is physical px end-to-end; macOS/Linux is DIP end-to-end.** PR #1182 established this for the header drag (`posScale()` + `get_window_position` DIP); redock must extend it to `resolve_window_at_cursor` + the hover broadcast/receiver.

---

## Implementation plan (suggested PR, separate from this report)

| Step | File | Est. |
|---|---|---|
| Gap 2: `register_backend_window` populates `shadow_backend_window_ids` on non-Windows | `commands/window/meta.rs` | ~10 LOC |
| Gap 1: `resolve_window_at_cursor` macOS hit-test (UI-thread bounds read of `state.windows`, exclude_label, main fallback) | `commands/window/motion.rs` + `ui_tasks.rs` | ~80–120 LOC |
| Gap 1: frontend sends DIP cursor on macOS (`posScale()` in `tryRedockAtCursor` + `pushRedockHover`) | `floating-pane-workspace.tsx` | ~6 LOC |
| Gap 3: hover receiver platform-aware cursor conversion | `app-init.ts` | ~4 LOC |
| (optional) true Z-order via `[NSApp orderedWindows]` + a label↔NSWindow registry | new `floating_pane_macos.rs` or `state.windows` ordering | ~60 LOC |

**Phasing:** ship Gaps 1–3 as a first PR for working redock (prefer-most-recently-focused for overlap). The true-Z-order refinement is an optional follow-up — overlapping AgentMux windows at the exact drop point is the only case it changes.

---

## Acceptance criteria (for the implementation PR)

- [ ] macOS: tear a pane → drag the floater over another AgentMux window → a drop-zone highlight appears on the target → release → the pane merges into the target's active tab and the floater auto-closes.
- [ ] macOS: drop over the desktop / an external app → floater stays put (no-op), no errors.
- [ ] macOS: highlight overlay lands under the cursor on a Retina display (Gap 3 verified).
- [ ] macOS: dropping onto the **main** window works deterministically (main-fallback parity with PR #1166).
- [ ] Windows: unchanged.
- [ ] No new backend-reducer or saga code (Layer 1 already cross-platform).

---

## Appendix — why no NSPanel / addChildWindow is needed for redock

PR #1182 showed the macOS floater is a frameless CEF Views window (not a raw NSPanel), and that the chromeless render + drag work without AppKit. Redock is the same story: it's a **read** (hit-test) + the existing cross-platform **mutation** (MoveBlock saga). The only genuinely macOS-native question is Z-order resolution among overlapping windows (Gap 1, option C/optional) — and even that has a pragmatic non-AppKit first cut (most-recently-focused). The owned-window lifecycle (NSPanel + `addChildWindow`) from the older cross-platform spec remains a separate, optional polish (the floater following/minimizing with its parent), unrelated to redock.
