# Spec (v2) — Floating-pane drag — host-side manual loop

> *This file is the landing target for issue #1276. Land this as a docs PR before starting implementation.*

**Status:** Draft v2 — for review
**Date:** 2026-06-05
**Author:** agent2
**Front:** UX-latency umbrella #1161 (windows family of fixes)

**Related:**
- `docs/specs/SPEC_WINDOW_DRAG_MANUAL_MOVE_LOOP_2026_05_29.md` (the canonical pattern this spec extends)
- `docs/analyses/ANALYSIS_FLOATING_PANE_HEADER_DRAG_2026-05-27.md` (Option B = JS-driven drag; chosen for the floater)
- `docs/architecture/ARCHITECTURE_FLOATING_PANE_DOCKING_2026_05_30.md` (canonical map of tear-off / floater / redock / browser-pane lifecycle)
- PR #1181 (smooth Windows title-bar drag via host-side native move loop)
- PR #1177 (edge-resize floaters by dragging edges/corners)
- `frontend/app/workspace/floating-pane-workspace.tsx` (the JS-driven floater drag this spec replaces)
- `frontend/layout/lib/layoutResize.ts` (the JS-driven gutter resize hot path — secondary scope)

## 0. TL;DR

The user wants the latency they removed from the main-window title-bar drag (PR #1181) gone from the floating-pane header drag too. Today the floater runs Option B from `ANALYSIS_FLOATING_PANE_HEADER_DRAG_2026-05-27.md` — JS-driven mousedown + `get_window_position` + `set_window_position`-per-`mousemove` with a one-in-flight + coalesce throttle. The main window used the same pattern until PR #1181 replaced it with a **host-side manual native move loop** (`SetCapture` + `GetMessage` + `SetWindowPos` on the CEF UI thread, zero per-move IPC, no DPR math). We apply the **same fix** to the floater.

Pane-gutter resize is the secondary ask. It can ride the same pattern but has additional design considerations (the per-tick work isn't a single `SetWindowPos`; it's a layout-tree math pass that today runs in the renderer). Floater drag is the primary scope of v1; pane resize is sketched in §4 and split out into a follow-on spec.

## 1. What's in place today (verified against current `main`)

### 1.1 Floater drag — current path

`frontend/app/workspace/floating-pane-workspace.tsx` `onMount` installs a document-level mousedown listener gated by `[data-role="block-header"]`. On qualifying mousedown:

- `preventDefault()` — **load-bearing.** Blocks the HTML5 `dragstart` pragmatic-dnd would have used to initiate a pane tear-off (`TileLayout.win32.tsx:439`). Without it, dragging the floater's header tears the block off into **another** floating window — the "double tear-off" regression. See `ANALYSIS_FLOATING_PANE_HEADER_DRAG_2026-05-27.md` §"Tear-off conflict".
- Linux branch: fires `start_window_drag({label})` and returns. Host runs the compositor-driven drag (CEF Views `BeginWindowDrag` via the patched libcef).
- Windows + macOS branch: races a `get_window_position` IPC against `currentMouseDownId` (to bail on stale press), captures `screenX/Y` + `initWinX/Y`, sets `dragging = true`.
- Per `mousemove` while `dragging`: compute CSS-px delta → multiply by `posScale()` (= `devicePixelRatio` on Windows, `1` on macOS) → `set_window_position` IPC with one-in-flight + `pendingPos` buffer.
- Also per move: throttled (~50ms) `update_floating_redock_hover({source_label, x, y})` drives the **drop-target highlight** on whichever agentmux window the cursor is currently over.
- On `mouseup`: invalidate the in-flight mousedown id, `clear_floating_redock_hover`, and call `tryRedockAtCursor(screenX, screenY)` which queries `resolve_window_at_cursor` and (if there's a target) fires `RedockFloatingPane`.

### 1.2 The host already has the entry point we need

`agentmux-cef/src/commands/window/motion.rs::start_window_drag` is already:

- **Label-aware.** PR #1181's first fix marshaled the HWND lookup onto the caller thread via `resolve_window_hwnd(state, label)` with a `find_own_top_level_window()` fallback. The floater label resolves through `window_hwnds` to the right HWND. So the historical "label-is-silently-dropped" footgun (`ANALYSIS_FLOATING_PANE_HEADER_DRAG` §"the `find_own_top_level_window` problem") is **gone**.
- **Cross-platform.** On Windows it dispatches `post_win32_begin_move` → `Win32BeginMoveTask` (the host-side manual loop). On macOS/Linux it dispatches `post_start_drag` → `StartWindowDragTask` (CEF Views `BeginWindowDrag` via the patched libcef ABI; today guarded by the `patched-libcef` cargo feature).

So the **renderer change is the substantive work**. The host already does the right thing if the renderer just hands it the drag.

### 1.3 What the architecture doc says we mustn't break

From `ARCHITECTURE_FLOATING_PANE_DOCKING_2026_05_30.md` §0 themes 1, 2, 5 and §2.4:

- **Theme 1 — wrong-window HWND resolution.** Fixed for `start_window_drag` (label-aware); we ride that fix.
- **Theme 5 — fire-and-forget IPC coordination.** Floater drag → redock is a sequence of IPCs (`update_floating_redock_hover` during drag, `clear_floating_redock_hover` + `resolve_window_at_cursor` + `RedockFloatingPane` on release). The change must preserve the redock flow end-to-end.
- **§2.4** — `onMouseUp → clear_floating_redock_hover → tryRedockAtCursor → resolve_window_at_cursor → RedockFloatingPane`. The host-side drag must still let the renderer see `mouseup` so this chain still runs.

## 2. The constraint we hit and how we resolve it

When the host's `Win32BeginMoveTask` does `SetCapture(hwnd)` on the OUTER window HWND, the mouse-input pipeline routes WM_MOUSEMOVE/WM_LBUTTONUP into the **host's** message queue, not the CEF renderer's. That is:

- The renderer's `mousemove` listener does **not** fire during the drag.
- The renderer's `mouseup` listener fires **at end of loop** — the host explicitly `DispatchMessageW(&msg)` for `WM_LBUTTONUP` (PR #1181 §5 / reagent P1) so Chromium sees its balancing up.

This has two implications for the floater:

### 2.1 Mouseup still fires → redock-on-release still works

`tryRedockAtCursor(e.screenX, e.screenY)` runs from the dispatched mouseup with the cursor at release position. `resolve_window_at_cursor` + `RedockFloatingPane` are unchanged. **No regression on redock-on-release.**

### 2.2 Mousemove doesn't fire → redock-hover highlight goes dark during drag

`update_floating_redock_hover` is currently called from the renderer's `mousemove` listener while `dragging` is true. With host-owned input that listener doesn't fire → the drop-target highlight overlay never updates → **the user gets no live feedback about which window they'd drop into during the drag**.

This is the one user-visible regression of the naive port. Three ways to handle it:

**Option H1 — Host fires the hover IPC during the move loop (RECOMMENDED).**
`Win32BeginMoveTask` already has `GetCursorPos` per WM_MOUSEMOVE for `SetWindowPos`. Extend the task with a coalesced (10–20 Hz cap, matching the existing 50ms throttle) `update_floating_redock_hover` emission — same shape as the renderer's call. The host has the source label (it was passed as an argument; thread it through `Win32BeginMoveTask` or via a small `state.set_dragging_label(label)` registry). One small host change; no renderer change beyond removing the now-dead per-mousemove call.

*Pro:* preserves the existing UX exactly. The host owns *both* the cursor and the highlight stream — same place, no cross-process coordination, no extra IPC.
*Con:* puts `floating-pane-redock` business logic into a Win32 task. Acceptable — `update_floating_redock_hover` is just an IPC publish, not a redock decision; the decision still lives in `tryRedockAtCursor` post-release.

**Option H2 — Accept the loss; restore the highlight via a different mechanism.**
The user sees no highlight during drag; on `mouseup` the host's `resolve_window_at_cursor` is called and the final hit-test highlights briefly before redock fires. Inferior — the whole point of the live highlight is "before I let go, am I aiming at the right window?"

**Option H3 — Renderer polls cursor position via IPC during the drag.**
A setInterval calls `get_cursor_point` and `update_floating_redock_hover`. Two IPCs at ~50ms cadence. Defeats most of the IPC-reduction win, adds polling complexity.

**Decision: H1.** §3.2 covers the implementation. This is a *small* extension to PR #1181's task — not a redesign.

### 2.3 Esc-cancel still cancels redock correctly

PR #1181 §5.1 / reagent P1: Esc cancels the move (restores start position), keeps looping for renderer balance, dispatches the eventual WM_LBUTTONUP. From the renderer's perspective the mouseup fires at the *original* cursor position (because the host restored the window to its origin and the cursor is still wherever it physically is). The current `tryRedockAtCursor` could redock to a window the user didn't intend.

**Fix:** the host signals "cancelled" to the renderer alongside the dispatched WM_LBUTTONUP — either via a synthetic `cancelled=true` event over the CEF process_message channel, or by setting `dragging = false` from a `start_window_drag_cancelled` IPC fired from the task before its `DispatchMessageW`. The renderer's `onMouseUp` checks `dragging`; if it was reset by the cancel signal, skip `tryRedockAtCursor`. Small renderer change; flag in §3.

## 3. Implementation

### 3.1 Renderer (`frontend/app/workspace/floating-pane-workspace.tsx`)

The `onMount`'s `onMouseDown / onMouseMove / onMouseUp` block (current lines 285–443 — the JS-driven pump) collapses to:

```ts
const onMouseDown = (e: MouseEvent) => {
    if (e.button !== 0) return;
    const target = e.target as HTMLElement | null;
    if (!target) return;
    if (!target.closest(HEADER_SELECTOR)) return;
    if (target.closest(INTERACTIVE_SELECTOR)) return;

    // Load-bearing: block pragmatic-dnd's HTML5 dragstart so the floater
    // doesn't double-tear-off. (Analysis 2026-05-27 §"Tear-off conflict".)
    e.preventDefault();

    // Hand the drag to the host. On Windows: Win32BeginMoveTask manual
    // loop. On macOS/Linux: CefWindow::BeginWindowDrag. The host owns
    // motion + DPR + capture from here.
    invokeCommand("start_window_drag", { label }).catch(() => {});
    dragging = true;
};

const onMouseUp = (e: MouseEvent) => {
    if (!dragging) return;
    dragging = false;
    // Redock-on-release: tryRedockAtCursor still works because the host
    // dispatches WM_LBUTTONUP to the renderer (PR #1181 §5.1 input
    // balance). Cursor is at release position; resolve_window_at_cursor
    // hit-tests against window_hwnds.
    invokeCommand("clear_floating_redock_hover", {}).catch(() => {});
    void tryRedockAtCursor(e.screenX, e.screenY);
};

// Host-driven cancel: when the host's manual-move-loop Esc-cancels, it
// emits a process_message to the renderer; we suppress redock-on-release
// for the next mouseup.
hostBridge.on("window_drag_cancelled", () => { dragging = false; });
```

`onMouseMove` listener: **delete entirely.** Its only job was the per-move `sendPos` and `pushRedockHover`. Both are now host-owned.

**Deleted symbols** (all in this file): `currentMouseDownId`, `clickScreenX/Y`, `initWinX/Y`, `latestScreenX/Y`, `setPosInFlight`, `pendingPos`, `sendPos`, the `pushRedockHover` helper, the entire `mousemove` listener, the entire pre-drag `get_window_position` race + catch-up block. `posScale()` is also used by `tryRedockAtCursor` and the edge-resize block (§"Edge-resize" lines 130–260 in the current file) so it stays.

Net renderer change: ~140 lines removed, ~10 lines added.

### 3.2 Host — extend `Win32BeginMoveTask` with redock-hover emission

`agentmux-cef/src/ui_tasks.rs::Win32BeginMoveTask::execute`'s WM_MOUSEMOVE branch (around line 356 today) adds, *after* `SetWindowPos`:

```rust
// Floater-only: emit a coalesced redock-hover update so the drop-target
// highlight tracks the cursor during host-owned drag. The renderer's
// listener is dark while we hold capture (§2.2 of the spec). 50ms cadence
// matches the renderer's prior pushRedockHover throttle.
if let Some(label) = self.source_label.as_deref() {
    if label.starts_with("floating-")
        && self.last_hover_emit.elapsed() >= Duration::from_millis(50)
    {
        ipc::publish_redock_hover(label, cur.x, cur.y);
        self.last_hover_emit = Instant::now();
    }
}
```

That's the entire host extension. `Win32BeginMoveTask`'s struct gains `source_label: Option<String>` (already trivially passed from the `start_window_drag` handler) and `last_hover_emit: Instant`.

For the Esc-cancel signal (§2.3):

```rust
// WM_KEYDOWN VK_ESCAPE branch (around line 382):
//   …existing: set cancelled=true, SetWindowPos to origin, keep looping…
// Add:
ipc::publish_drag_cancelled(self.source_label.as_deref().unwrap_or("main"));
```

The renderer subscribes via the existing CEF process_message bridge — same mechanism `state.rs` already uses for host→renderer events. New event type: `window_drag_cancelled { label }`. The floater's `onMount` listens; on receipt, sets `dragging = false`.

### 3.3 macOS / Linux

`start_window_drag` on macOS/Linux dispatches `StartWindowDragTask` → `CefWindow::BeginWindowDrag`. Two things to verify (spike F3 in §6):

1. **Capture model.** `BeginWindowDrag` is a CEF Views API; on macOS it ultimately calls `NSWindow performDrag` (or the AppKit equivalent), which has its own event-handling model. **Does the renderer see WM_MOUSEMOVE-equivalents (`mousemove`) during the AppKit drag?** If yes, the renderer's existing `mousemove` listener could keep firing `update_floating_redock_hover` on those platforms (and §2.2 wouldn't apply there). If no, we need the H1 extension on the macOS/Linux task too — but the CEF Views API doesn't expose a per-move callback, so we'd need to poll `get_cursor_point` from a timer started at `BeginWindowDrag` and stopped at the next `mouseup`. Spike confirms.
2. **`patched-libcef` feature.** `StartWindowDragTask` is guarded by the `patched-libcef` cargo feature. Builds without the patch fall back silently (today's warning at `ui_tasks.rs:227`). The Windows path is unaffected — `Win32BeginMoveTask` doesn't depend on the cef-rs patch — but **the floater drag still relies on the legacy JS+IPC path on unpatched macOS/Linux builds**. Either gate the renderer dispatch on a feature-detection IPC, or accept that unpatched builds are dev-only (today's posture).

### 3.4 Coordinate systems — table from the architecture doc

§6 of `ARCHITECTURE_FLOATING_PANE_DOCKING_2026_05_30.md` lays out the units. Post-change:

| Layer | Unit | Before | After |
|---|---|---|---|
| Frontend DOM mousedown | CSS px | screenX/Y captured, multiplied by `posScale()` (= DPR on Win, 1 on macOS) | **Never used** — host reads `GetCursorPos` (physical px on Win) / Views API (DIP on macOS) directly |
| `set_window_position` IPC | physical px on Win, DIP on macOS/Linux | DPR math in renderer | **Path removed** for drag (still used by edge-resize) |
| `Win32BeginMoveTask` | physical px (`SetWindowPos`) | already correct | unchanged |
| `BeginWindowDrag` (macOS/Linux) | CEF Views DIP | already correct | unchanged |
| `update_floating_redock_hover` IPC | physical px on Win, DIP on macOS/Linux | renderer multiplied by `posScale()` | **Host emits** — already in the right unit (the host's coord values are native) |
| `resolve_window_at_cursor` | physical px on Win, DIP on macOS/Linux | renderer multiplied by `posScale()` at `mouseup` | unchanged — mouseup is still renderer-side |

**The renderer's DPR math for the drag path disappears.** `posScale()` is retained for the edge-resize block (PR #1177, lines 130–260) and for `tryRedockAtCursor`'s mouseup hit-test, both of which still operate from renderer screen coords.

## 4. Pane resize (secondary scope; do as follow-on)

Same recipe as floater drag, but the per-tick work is **layout-tree math + airspace clip updates**, not a single `SetWindowPos`. Three options:

| Option | What | Cost | When |
|---|---|---|---|
| A | Port the layout tree to Rust; host owns sizes | Quarter of work | Never (too far) |
| **B** | **Host runs the loop; renderer keeps the tree.** Per WM_MOUSEMOVE the host posts a one-way `pane_resize_tick(handle_id, x, y)` CEF `process_message` to the renderer. Renderer's existing `onResizeMove` runs unchanged, driven by the message instead of by `pointermove`. | Spike-gated; ~2 days if the bridge sustains the rate | **Recommended** |
| C | Keep loop in renderer; rAF-coalesce the per-mousemove work and defer `browser_pane_resize` to release | ~0.5 day | Fallback if B's bridge rate is too slow |

The empirical question is: **can CEF `process_message` sustain ≥60 Hz `host → renderer` events with <16ms median latency?** Spike R1/R2 in §6 of the previous draft answered this with measurements before we commit to plan B. If R2 misses, we ship plan C — still removes per-tick `browser_pane_resize` IPC even if it doesn't move capture to the host.

This is a **separate PR** from the floater drag work — keep them independent so an issue with one doesn't block the other.

## 5. Goals & non-goals

**Goals (v1, floater drag):**

- Floater header drag tracks the cursor at input rate on Windows, matching the main-window title-bar drag from PR #1181.
- Floater drag on macOS / Linux uses the host-side `BeginWindowDrag` path (which `start_window_drag` already dispatches) — already wired; this PR just deletes the JS pump in front of it.
- Redock-hover highlight tracks the cursor during host-owned drag (Option H1 in §2.2).
- Redock-on-release works unchanged.
- Esc-cancel does not redock to a wrong window (§2.3).

**Non-goals (v1):**

- Aero Snap during floater drag (same trade-off as main window — `Win32BeginMoveTask` runs a manual loop, doesn't engage the OS modal move loop).
- Live content updates *during* the drag (mirrors main window — content shows last frame).
- Pane gutter resize (§4 — split into a follow-on spec gated on spike R1/R2).
- Edge-resize on the floater (PR #1177). Different gesture, different IPC (`set_window_rect`). Could be a future candidate for the same pattern; not v1.
- Linux/macOS pane resize (the Views-based path is structurally different from Win32 — separate spec).
- Fixing the 12 px edge-resize band overlap with the 33 px header (existing UX edge case per architecture doc §2.3).

## 6. Spikes (before committing implementation)

### F1 — naive port (1 hour)

Hardcode the Windows + macOS branches of `floating-pane-workspace.tsx::onMouseDown` to take the same shape as the Linux branch (single `start_window_drag` dispatch). Build + run on Windows. Expected behavior: drag is smooth, **redock-hover highlight is dark during drag**. If yes, the host path works for floater labels — proceed to F2. If no, debug label resolution (PR #1181's `resolve_window_hwnd` should handle it).

### F2 — host-side hover emit (3 hours)

Extend `Win32BeginMoveTask` per §3.2: `source_label` field, `update_floating_redock_hover` publish at 50ms cadence. Renderer's `mousemove` listener can be removed at this point. Verify: drag a floater across two agentmux windows → highlight tracks correctly → release → redocks to the right target.

### F3 — macOS/Linux behavior (2 hours)

Confirm whether the renderer's `mousemove` listener fires during `BeginWindowDrag`. If yes, no host change needed on macOS/Linux. If no, decide whether to add a timer-polling fallback to `StartWindowDragTask` or accept the limitation on macOS/Linux for v1.

### F4 — Esc-cancel correctness (1 hour)

Add the `window_drag_cancelled` process_message emission to `Win32BeginMoveTask`'s Esc branch and the renderer subscription. Test: press-drag-Esc-release-with-cursor-over-another-window → no redock. Without the signal, the cursor-at-mouseup would falsely redock to wherever Esc cursor landed.

### R1, R2, R3 — pane-resize spikes

Out of scope for v1. Will move into a separate spec when v1 (floater drag) ships.

## 7. Phases

| Phase | Scope | Estimate | Gate |
|---|---|---|---|
| 0. This spec | Review w/ user + reagent | now | — |
| 1. Floater drag impl + spikes F1–F4 | §3 — renderer simplification + small host extension | 1 day | F1, F2 green |
| 2. macOS / Linux verification | F3 — confirm or add timer polling | 0.5 day if F3 finds a gap | F3 result |
| 3. Pane-resize spike + spec follow-on | §4 — measure bridge rate; new spec if B is viable, else implement C | 1 day for spikes, separate PR | After Phase 1 ships |
| 4. Edge-resize on floaters | PR #1177 — port to same pattern if F1-F2 prove the pattern | future | — |

## 8. Risks & mitigations

| Risk | Mitigation |
|---|---|
| Host's `update_floating_redock_hover` emission misses or duplicates relative to current renderer-side cadence (highlight feels off) | F2 spike measures; the 50ms throttle matches the renderer's prior `HOVER_PUSH_THROTTLE_MS = 50`. |
| Esc-cancel falsely redocks | F4 explicitly tests; the cancel signal IPC is small and reversible. |
| macOS `BeginWindowDrag` has different event semantics → unexpected regression | F3 isolates before committing the macOS dispatch; gate the dispatch on platform until verified. |
| `patched-libcef` feature missing in some builds → silent drag-no-op on macOS/Linux | Existing log line at `ui_tasks.rs:227` covers this; don't add a renderer-side feature gate (it'd grow the surface). Document as a build prerequisite. |
| Redock-on-release breaks because mouseup arrives without a `mousedown` ack in some edge case | The architecture doc §0 theme 5 — fire-and-forget IPC coordination. Mitigation: keep `dragging` flag set by `mousedown`, cleared by `mouseup` OR by `window_drag_cancelled`. Both code paths must clear it. |
| Capture stolen mid-loop by an OS prompt (UAC, virus scanner) leaves dragging=true in the renderer | PR #1181's wake-tick + button-up safety net exits the host loop cleanly; the host publishes the same `WM_LBUTTONUP` → renderer's `onMouseUp` clears `dragging`. Verify in F1. |

## 9. Open questions

1. **`source_label` plumbing in `Win32BeginMoveTask`.** The `start_window_drag` handler in `motion.rs` knows the label; pass it through `post_win32_begin_move(hwnd, label)` instead of the current single-arg call. Trivial signature change.
2. **`ipc::publish_redock_hover` / `ipc::publish_drag_cancelled` channel.** These are host → renderer messages. What's the existing channel mechanism? CEF `process_message` is the obvious answer; confirm during implementation, name the message types consistently with existing events.
3. **`window_drag_cancelled` event payload.** Just `{label}` or also `{reason: 'esc' | 'capture_lost' | …}`? Minimal: `{label}`; renderer treats any cancel as "skip the next redock-on-release". Reagent will probably want the reason for diagnostics.
4. **Pre-existing `dragging` flag in the renderer.** Today it's only set after the awaited `get_window_position`. Post-change we set it synchronously on mousedown — slightly different semantics (a too-fast click-release won't trigger redock-on-release if the host bounces the start, but the host's PR #1181 §0 bail handles fast clicks gracefully). Test in F1.
5. **Touch-pad / drag-from-other-window cases.** The header drag is mouse-only today (no pointer-event handling). Confirm no platform sends `pointermove` without `mousemove` during a floater drag.

## 10. What this spec deliberately does NOT change

- The `floating_pane_wndproc` WndProc (`agentmux-cef/src/floating_pane.rs`) — still owns frameless / DwmExtendFrameIntoClientArea / WM_SIZE child resize.
- The redock-on-release backend flow (`RedockFloatingPane`, the auto-close watcher, the PaneLocation modeling in `ARCHITECTURE_FLOATING_PANE_DOCKING` §9 P3 — that's a separate restructuring).
- `find_main_window` / `find_own_top_level_window` / `resolve_window_hwnd` — already correct for our needs as of PR #1181 + #1165 + #1166.
- The `[data-role="block-header"]` attribute and the targeted-listener approach from `ANALYSIS_FLOATING_PANE_HEADER_DRAG_2026-05-27.md` §"Recommendation". The selection mechanism is unchanged; only what we do after we select is.
- pragmatic-dnd's `draggable()` setup on the pane header (`TileLayout.win32.tsx:439`). `e.preventDefault()` on mousedown still suppresses its `dragstart`.

## 11. Test plan (mirrors PR #1181)

- **Visual smoothness:** drag a floater fast across a 144 Hz display — header tracks cursor without stair-stepping. Confirm on a 4K @ 200% scale monitor (no DPR drift; the host owns physical px).
- **Cross-DPI:** drag a floater from a 100% scale monitor to a 200% one — no jump, no double-speed motion.
- **Esc-cancel + redock interaction:** mid-drag press Esc with cursor over another agentmux window — floater returns to start; **no redock fires**. This is the §2.3 fix; explicit test.
- **Redock-on-release:** drag a floater over another agentmux window's redock zone — highlight tracks, release → block redocks. (Was the §2.2 concern; this confirms H1 restores parity.)
- **Renderer input balance:** mid-drag, hover over an interactive element after release — buttons fire, tooltips render normally. PR #1181's WM_LBUTTONUP dispatch is the prerequisite; verify it works here.
- **Double tear-off does NOT regress:** drag a floater header — the floater moves; no second floater is torn off. The `preventDefault` is unchanged from today.
- **Multiple windows in flight:** drag the main window while a floater is open in another monitor; confirm the main window moves and the floater is untouched. Tests `resolve_window_hwnd` label awareness.
- **macOS / Linux** (after F3): same smoke tests. If F3 finds `mousemove` *does* fire during `BeginWindowDrag`, the renderer's hover IPC is allowed to keep firing on those platforms (no regression). If not, the host-timer fallback kicks in.

## 12. References

- `c63edf18` `feat(window-drag): smooth Windows title-bar drag via host-side native move loop (#1181)` — canonical PR; the pattern this work copies
- `docs/specs/SPEC_WINDOW_DRAG_MANUAL_MOVE_LOOP_2026_05_29.md` — canonical spec for the manual-loop pattern
- `docs/analyses/ANALYSIS_FLOATING_PANE_HEADER_DRAG_2026-05-27.md` — analysis that picked Option B (JS-driven); we keep Option B's listener pattern but move the move-loop to the host
- `docs/architecture/ARCHITECTURE_FLOATING_PANE_DOCKING_2026_05_30.md` — full system map; §0 themes 1/5, §2.4 redock flow, §4 HWND resolution, §6 coord systems, §9 P1 single resolver
- `frontend/app/workspace/floating-pane-workspace.tsx` lines 131–443 (current `onMount`) — the renderer block to simplify
- `agentmux-cef/src/ui_tasks.rs::Win32BeginMoveTask` (lines 248–425) — host-side loop to extend with source_label + hover-emit + cancel-emit
- `agentmux-cef/src/commands/window/motion.rs::start_window_drag` (line 211) — IPC entry; passes label through to the task
- I1-I6 isolation invariants in `agentmux/CLAUDE.md` — any new IPC channel names must embed the `dir_hash` to stay per-(channel, version)
