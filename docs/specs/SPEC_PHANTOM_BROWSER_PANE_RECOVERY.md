# Phantom browser pane recovery

**Status:** Proposed
**Owner:** AgentA
**Date:** 2026-05-09
**Driving incident:** User saw a black browser pane in dev mode after navigating to google.com. Host log showed CEF browser was `BrowserUnregistered` ~4 minutes earlier; the frontend `BrowserViewModel` never noticed and kept dispatching `Navigate` against an orphaned slot. Closing/reopening the pane recovered.

## Symptoms

- Browser pane area renders **black** (or stale paint of last loaded page).
- Address-bar input still focuses but typed URLs never load.
- `[ipc] browser_pane_navigate` returns an error (caught silently by the model's `.catch`); no user-visible signal.
- Dispatch ring (Ctrl+Shift+D) shows ongoing `Navigate` / `LoadStarted` commands against the orphaned `block_id`.
- Slot store snapshot shows `closed: false` even though the host CEF browser is gone.

## Concrete log signature

```
15:36:18  host-reducer: event=BrowserUnregistered label=browser-pane-<bid>-1
15:36:18  Unregistered browser: label=browser-pane-<bid>-1 (remaining: 3)
15:36:18  [on_before_close] no backend window ID registered for label=… — shells may orphan
15:40:26  [fe] [browser-pane:diag][<bid>] navigate(url="…") closed=false
15:40:26  [fe] [browser-pane:diag][<bid>] dispatch type=Navigate src=navigate
   …      (no nav-state event ever returns; the CEF browser is gone)
```

## Root cause

Three sources of truth on browser-pane lifecycle, two of which can drift:

1. **Host's `BrowserPaneManager` registry** (`agentmux-cef/src/browser_pane/`) — knows actual CEF browser state.
2. **Host-reducer state** — emits `BrowserRegistered` / `BrowserUnregistered` events (`agentmux-cef/src/reducer/browsers.rs`).
3. **Frontend slice #9 slot store** (`frontend/app/store/browser-pane-state-store.ts`) — `registerPane` / `unregisterPane` driven by `BrowserViewModel` constructor / `dispose()`.

Today, the host's unregister event (1, 2) does NOT propagate to the frontend (3). Causes of unregister that the frontend doesn't see:

- Renderer process crash (CEF auto-cleans the browser; no DOM event).
- GPU process loss + browser context invalidation.
- Tear-off cleanup paths that drop the source-side browser without disposing the model.
- Programmatic close from another window (less common).

The frontend's `BrowserViewModel` only flips `closed = true` when its own `dispose()` runs — driven by the layout's block-removal saga, which doesn't fire on host-side cleanup.

## Fix design

Three coordinated phases. Each ships independently.

### Phase A — host emits `browser-pane-unregistered` event

Hook into the `BrowserUnregistered` host-reducer state transition. Emit a CEF event over the JS bridge with payload:

```ts
{
    block_id: string,        // extracted from label "browser-pane-<bid>-N"
    label: string,           // full label for diagnosis
    reason: "user-close"     // model's dispose() preceded this
          | "renderer-crash" // CEF callback indicated crash
          | "host-cleanup"   // explicit programmatic close (tearoff, window close, etc.)
          | "unknown",       // catch-all when reason can't be determined
}
```

Implementation: add a new event-emit call in the reducer's `on_apply` for `BrowserUnregistered` events. Filter to `kind: Pane { … }` (don't emit for top-level windows or pool-windows; those are handled separately).

**~30 LOC** in `agentmux-cef/src/state.rs` (event emit) + `agentmux-cef/src/reducer/mod.rs` (label parsing).

### Phase B — frontend listener auto-disposes the slot

`BrowserViewModel` constructor adds a third subscription alongside `browser-pane-nav-state` and `browser-pane-clicked`:

```ts
listenEvent<{ block_id: string; reason: string }>(
    "browser-pane-unregistered",
    (payload) => {
        if (payload.block_id !== this.blockId) return;
        this.diag(`host-unregistered reason=${payload.reason}`);
        // Same code path as user dispose — emits Disposed through
        // the reducer, post-close gate kicks in for any late IPC.
        this.dispose();
    }
);
```

The model's `dispose()` is already idempotent (verified by slot-store tests in PR #764). Calling it from the unregistered handler:
- Dispatches `Disposed` → `state.closed = true` → all subsequent commands gate as `post-close-command-dropped` (visible in dispatch ring).
- Tears down the IPC subscriptions.
- Calls `bpUnregisterPane` which removes the slot.

**~25 LOC** in `frontend/app/view/browser/browser-model.ts`.

### Phase C — view-side error UI

When `state.closed` flips true via `host-unregistered` (not user dispose), the view shows a banner over the pane area:

```
⚠ Browser disconnected (renderer-crash) — close pane to reset
[Close pane]   [Open new browser pane]
```

The banner overlays the (now-stale or black) HWND area. The "Close pane" button calls the existing block-close path; "Open new" creates a fresh block in the same layout slot.

This is the user-recovery UX the spec promised. Without it, users see black and don't know they can recover. **~50 LOC** in `frontend/app/view/browser/browser-view.tsx` + minor reducer state addition (`closeReason: "user" | "host-unregistered"` on the slot state, distinguishing dispose paths).

## Tear-off variant (regression — observed 2026-05-09)

After tearing off a tab containing 2 browser panes:
- New window shows **2 black browsers**.
- DOMs from the source window **flicker out of place** atop the first window (HWNDs not reparented to new window's chrome; render output lands at stale screen coords).

This is a stronger version of the same root cause: tear-off code in `agentmux-cef/src/tearoff/` (and host browser-pane reparenting in `agentmux-cef/src/browser_pane/window_attach.rs`) drops/reattaches the source HWNDs, but the frontend `BrowserViewModel` never re-binds. Symptom matches a class previously fixed (history: hwnd reparenting on tear-off — confirm via `git log --grep tear` once we resume).

Important relationship to Phase A/B/C above: the unregister-event approach **does not cover the tearoff path** if the host is still treating those browsers as live (just attached to a new window). What's needed:

1. **Host side** — on tear-off completion, emit an event (`browser-pane-reparented`) per pane with `{ block_id, new_window_id, new_hwnd_parent }` so the frontend knows where its browser ended up.
2. **Frontend side** — `BrowserViewModel` re-runs the host-attach handshake (`browser_pane_attach`) against the new window context after reparent, so coordinates resolve relative to the right chrome.
3. **Diagnostic** — log the HWND parent delta in host log on every tear-off so this is visible in muxlog.

This is a separate fix from the orphan/recovery flow above, but the spec should track both because they fail in the same way (host-truth and frontend-truth diverged) and should be fixed with the same instinct (host emits a lifecycle event; frontend listens and reconciles).

**Effort:** ~50 LOC additional (~0.5 day) on top of A/B/C.

## Edge cases

- **Race: dispose then unregistered** — user clicks close (dispose runs, slot unregistered), then host emits unregistered. Listener finds no slot for the blockId; safe no-op via `bpSnapshot(this.blockId) == null` guard already in `_dispatch`.
- **Race: unregistered then immediate re-create** — host destroys then re-creates a browser at the same blockId (uncommon; would require a tear-off-cancel-back-to-source flow). The Phase B listener sees the unregister → Disposed; the new register would need to come with a new `BrowserViewModel`. Out of scope: don't try to "recover the same model"; let the layout re-create.
- **Reason can't be determined** — `reason: "unknown"` is the safe default. The view's banner just shows "Browser disconnected" without the reason qualifier.

## Out of scope

- **Auto-recovery** — recreating the CEF browser at the same blockId without user action. Distinct concern; ergonomics + state-restore complexity. Punt unless the close-and-reopen friction is explicitly painful.
- **Pool drift** (launcher mirror count > host count) — different layer (window-level, not pane-level). Tracked separately.
- **Renderer-crash root-cause investigation** — this spec is about graceful recovery, not preventing the crash. The crashes themselves are diagnosed via Crashpad dumps + WER.

## Effort

| Phase | LOC | Days |
|---|---|---|
| A — host emit | ~30 | 0.25 |
| B — frontend listener | ~25 | 0.25 |
| C — error UI | ~50 | 0.5 |
| **Total** | ~105 | **1 day** |

A + B can ship as one PR (small Rust + small TS, both in same flow). C is a follow-up after the lifecycle is correct.

## Cross-references

- Slice #9 Phase 4: `frontend/app/store/browser-pane-state-store.ts` (slot store this disposes through)
- Slice #9 Phase 5: `frontend/app/devtools/diag-panel.tsx` (where the orphan dispatches were visible during the diagnosis)
- `agentmux-cef/src/reducer/browsers.rs` — host-reducer browser register/unregister
- `agentmux-cef/src/state.rs:1113` — current `BrowserUnregistered` log emit (event hook lands alongside)
- Driving log: `~/.agentmux/dev/main/logs/agentmux-host-v0.33.739.log.2026-05-09` (2026-05-09 ~15:36–15:40 window)
