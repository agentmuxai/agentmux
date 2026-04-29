# B.7.3 — launcher events to renderer via CEF JS bridge

**Status:** Implementation spec. Written 2026-04-29 after B.9.3 merge (#601).
**Author:** AgentA.
**Parent spec:** `specs/SPEC_WINDOW_PROCESS_STATE_MACHINE_2026_04_27.md` §5.3 + §5.5 (renderer subscription, echo-loop guard).
**Read after:** `docs/retro/phase-b-roadmap.md`, `docs/retro/next-steps-2026-04-29.md` §1.2.

---

## Why this exists

After B.7.1 + B.7.2 (#596 + #597), the renderer subscribes to **one bespoke event** — `window-instances-changed` — emitted by the host. The launcher reducer emits a much richer typed-event stream (`Event::WindowOpened`, `Event::WindowClosed`, `Event::BackendWindowIdRegistered`, `Event::HwndDriftDetected`, `Event::CorrectiveWindowMove`, `Event::HostShouldQuit`, etc.) that **never reaches the renderer directly**. The host's `apply_event_to_shadow` translates SOME launcher events into the bespoke proxy event; most don't make it.

This means:
- Each new launcher event needs new host translation code to surface in the UI.
- WRR drift events (`OffMonitor`, `HiddenSinceOpen`, `OrphanDestroy`, `OrphanInstance`) only land in launcher logs — the user never sees them in-app.
- Frontend's reducer doesn't have a clean "all canonical state" feed; it patches together via a few proxy paths.

**B.7.3 makes the renderer a first-class typed-event subscriber.** The host's CEF JS bridge forwards every launcher event to every active renderer; the renderer registers a single dispatcher that feeds a SolidJS signal; the frontend reducer / atoms / blocks consume the typed stream.

---

## Where the current state of the codebase is

After B.9.3 (#601) merged, the relevant pieces are:

```
LAUNCHER (agentmux-launcher/src)
  reducer.rs::update — emits Event variants per the agentmux-common::ipc::Event enum
  ipc/server.rs::handle_connection — sends events back over the originating pipe
                                     connection only (per-connection reply, no broadcast)

HOST (agentmux-cef/src)
  launcher_ipc.rs::connect_to_launcher — opens the pipe, reads events
  launcher_ipc.rs::apply_event_to_shadow — host's projection layer; updates
                                           shadow_window_meta, shadow_instance_registry,
                                           shadow_backend_window_ids; ALSO re-emits
                                           the bespoke window-instances-changed via
                                           emit_event_all_windows for SOME events

  events.rs::emit_event_all_windows(state, name, payload) — current proxy that
    iterates state.browsers and calls Frame::ExecuteJavaScript on each;
    delivers the bespoke event names

RENDERER (frontend)
  app-init.ts::initInstanceTracking subscribes via getApi().listen("window-instances-changed", ...)
  cef-api.ts::listen wraps the host's bespoke event channel
  app/store/global.ts atoms updated by app-init.ts payload handler
```

**Two reasons to leave most of this in place during the cutover:**
1. The bespoke `window-instances-changed` is consumed by ≥3 frontend sites (`app-init.ts`, action-widgets, command-registry). Removing it requires migrating each.
2. The `task dev` mode (no launcher) still emits the bespoke event directly from `commands::window`/`drag`/`window_pool`. Renderer needs to handle "no launcher" gracefully.

---

## Design

### Wire format

Reuse `agentmux_common::ipc::Event` directly. It's already `serde_json::Serialize`. The host's outbound JS bridge serializes to JSON and dispatches:

```js
// Renderer receives this via window.__agentmux_launcher_event(...)
{
  "event": "window_opened",
  "label": "window-abc123...",
  "kind": "full_instance",
  "parent_label": null,
  "version": 42
}
```

The `event` discriminant matches the snake-case wire form (`#[serde(tag = "event", rename_all = "snake_case")]` already on `Event`). The `version` field carries the reducer's monotonic counter, useful for echo-loop deduping (per parent spec §5.5).

### Host outbound bridge — new module `agentmux-cef/src/launcher_event_bridge.rs`

Single function `dispatch_to_renderers(state, event)` called from `apply_event_to_shadow` after each event is applied to host shadows. Iterates `state.browsers`, calls `Frame::ExecuteJavaScript` per browser:

```rust
pub fn dispatch_to_renderers(state: &Arc<AppState>, event: &agentmux_common::ipc::Event) {
    let json = match serde_json::to_string(event) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("[launcher-event-bridge] serialize failed: {}", e);
            return;
        }
    };
    // Note: the JS string interpolation uses serde_json's escaping —
    // safe against quote/backtick injection from Event payloads.
    let script = format!(
        "if (window.__agentmux_launcher_event) {{ try {{ window.__agentmux_launcher_event({}) }} catch(e) {{ console.error('[launcher-event] dispatch failed', e) }} }}",
        json
    );
    let browsers = state.browsers.lock();
    for (label, b) in browsers.iter() {
        // Skip pool + browser-pane — they have no UI to react.
        if label.starts_with("window-pool-") || label.starts_with("browser-pane-") {
            continue;
        }
        let mut b = b.clone();
        if let Some(frame) = b.main_frame() {
            frame.execute_java_script(
                &cef::CefString::from(&script[..]),
                &cef::CefString::from(""),
                0,
            );
        }
    }
}
```

Filtering rationale:
- **Pool windows skipped** — they're hidden, no user-visible UI. Sending events to them wastes a JS roundtrip per pool window per event; with 3+ pool windows that's 3× the cost.
- **Browser-pane child HWNDs skipped** — same reason.
- All other top-level browsers (main, full-instance tear-offs, sub-windows) get the event.

Cross-platform: `Frame::ExecuteJavaScript` is portable; nothing Windows-specific in the dispatch.

### Renderer subscription — `frontend/util/launcher-events.ts`

New module. At init, registers `window.__agentmux_launcher_event` as the dispatcher. Pushes events into a `solid-js` signal that block-level subscribers can `effect()` over.

```ts
import { createSignal } from "solid-js";

// Mirrors agentmux_common::ipc::Event enum on the wire.
// Internally tagged: { event: "window_opened", label, kind, ..., version }
export interface LauncherEvent {
    event: string;
    version: number;
    [field: string]: any;
}

const [latestEvent, setLatestEvent] = createSignal<LauncherEvent | null>(null);
const [eventVersion, setEventVersion] = createSignal<number>(0);

/** Block subscribers read from this signal in createEffect. */
export const launcherEvent = latestEvent;
export const launcherEventVersion = eventVersion;

let installed = false;
export function installLauncherEventBridge() {
    if (installed) return;
    installed = true;
    (window as any).__agentmux_launcher_event = (evt: LauncherEvent) => {
        // Accept any event with a version; drift / lifecycle / corrective.
        if (typeof evt?.version !== "number") {
            console.warn("[launcher-events] received event without version", evt);
            return;
        }
        setLatestEvent(evt);
        setEventVersion(evt.version);
    };
    console.log("[launcher-events] bridge installed; window.__agentmux_launcher_event ready");
}
```

Called once from `app-init.ts::initApp` BEFORE the first renderer-state-needing operation. Idempotent.

### Frontend reducer — feeding atoms

The renderer's existing atoms (`openWindowLabelsAtom`, `openWindowEntriesAtom`, `windowInstanceNumAtom`, `windowCountAtom`, etc.) currently get fed by `app-init.ts::applyEntries` which is called by the bespoke event listener. After B.7.3, those atoms are fed by typed events.

New file `frontend/app/store/launcher-event-reducer.ts`:

```ts
import { createEffect } from "solid-js";
import { launcherEvent, launcherEventVersion } from "@/util/launcher-events";
import { setOpenWindowLabelsAtom, setOpenWindowEntriesAtom,
         setWindowCountAtom, setWindowInstanceNumAtom } from "@/app/store/global";

export function startLauncherEventReducer() {
    createEffect(() => {
        const evt = launcherEvent();
        const v = launcherEventVersion();
        if (!evt) return;
        // Echo-loop guard (parent spec §5.5): if a local command
        // dispatched this and we're applying it, skip the
        // re-emission. Wire later when we have local commands
        // that hit the launcher.
        switch (evt.event) {
            case "window_opened":
                applyWindowOpened(evt);
                break;
            case "window_closed":
                applyWindowClosed(evt);
                break;
            case "backend_window_id_registered":
                applyBackendWindowIdRegistered(evt);
                break;
            // ... drift events: fire a notification or update a debug panel
            case "hwnd_drift_detected":
                applyHwndDrift(evt);
                break;
            // ... unhandled events are silently ignored (fwd-compat)
        }
    });
}
```

Each `apply*` function reads-modify-writes the corresponding atoms.

### Migration of `window-instances-changed`

**Coexistence period (this PR)**: keep `window-instances-changed` working. Both event channels deliver overlapping data. Frontend prefers typed events when available, falls back to the bespoke event when not.

**Detection**: at init time, if the launcher's bridge has installed itself (`window.__agentmux_launcher_event` is non-null and we've received any event with version > 0), the renderer treats typed events as authoritative. Otherwise (`task dev` mode, no launcher), it continues using the bespoke `window-instances-changed`.

**Retirement (next PR)**: once typed events are validated to be authoritative across all variations (open/close/tear-off/pane), remove the bespoke listener from `app-init.ts` and the sync emit sites in `commands::window::open_window_with_kind`, `commands::drag::tear_off`, `commands::window_pool::*`, `client.rs`. Net delete ~40 LoC across those sites.

### Echo-loop guard

Per parent spec §5.5: when a renderer's local command (e.g. `getApi().openNewWindow()`) results in a launcher-emitted event flowing back to the same renderer, the renderer must not re-dispatch the local command.

Initial design: track an `applying_remote` boolean in the reducer effect. When set, action handlers (e.g. atom-update side effects that would emit a command) check the flag and skip command emission. For B.7.3's first PR we don't have any frontend → launcher commands wired through this bridge yet (commands still go through the host IPC HTTP path), so the guard is a forward-compatibility hook — set it during `applyXxx` so future command emitters see it.

---

## Concretely, what changes — file-by-file

| File | Change |
|---|---|
| **agentmux-cef/src/launcher_event_bridge.rs** (new) | `pub fn dispatch_to_renderers(state, event)` — JSON + ExecuteJavaScript fanout, filters pool/pane labels |
| **agentmux-cef/src/launcher_ipc.rs::apply_event_to_shadow** | At end of every match arm, call `launcher_event_bridge::dispatch_to_renderers(state, event)` |
| **agentmux-cef/src/main.rs** | `mod launcher_event_bridge;` |
| **frontend/util/launcher-events.ts** (new) | Exports `installLauncherEventBridge()`, `launcherEvent` signal |
| **frontend/app/store/launcher-event-reducer.ts** (new) | `startLauncherEventReducer()` — `createEffect` over `launcherEvent`, dispatches by `evt.event` |
| **frontend/app-init.ts** | Call `installLauncherEventBridge()` early in `initApp`. Call `startLauncherEventReducer()` after `initWaveWrap`. Keep `window-instances-changed` listener as fallback when no typed events seen yet. |
| **agentmux-launcher/src/wrr/mod.rs** (no change) | Drift events already typed and emitted; they'll just flow through the new bridge automatically |
| **frontend/types/custom.d.ts** | Declare `window.__agentmux_launcher_event` for TS happiness |

Estimated diff: ~300 LoC new, ~30 LoC modified, ~0 LoC deleted (deletion happens in B.7.3.2 follow-up).

---

## Sub-PR sequence

This spec covers **B.7.3.1** (additive: bridge + renderer subscription + reducer scaffolding). Two follow-ups:

- **B.7.3.2** — feed atoms from typed events as the authoritative path; demote `window-instances-changed` to fallback-only; verify no regressions across all variations.
- **B.7.3.3** — retire `window-instances-changed` and the 4 sync-emit sites in `commands::window`, `drag`, `window_pool`, `client.rs`. Pure deletion. Delete the legacy `applyEntries` / `refreshLabelsViaRpc` paths.

Each sub-PR is independently shippable. B.7.3.1 lands the cable; .2 makes the renderer prefer typed events; .3 burns the old bridge.

---

## Test plan

**B.7.3.1 (this spec)**:
- [ ] Launch portable, open DevTools (Ctrl+Shift+I), confirm `window.__agentmux_launcher_event` is a function.
- [ ] Open a second window via status-bar button. Console should log incoming events: `window_opened`, `backend_window_id_registered`, etc. with valid `version`.
- [ ] Tear off a tab. Same logging pattern; events arrive within ~5ms of close.
- [ ] Close all windows. Process exits cleanly (B.9.3 invariant preserved).
- [ ] `task dev` (no launcher) — renderer init succeeds, `window.__agentmux_launcher_event` is still installed (no events arrive, but no crash). Bespoke `window-instances-changed` fallback continues to drive the InstancePanel.
- [ ] Force a drift event (curl `open_new_window`, see if `hwnd_drift_detected` arrives at the renderer console).

**B.7.3.2**:
- All atoms react correctly to typed events without the bespoke channel firing.
- Event ordering preserved per pipe (B.7.3 inherits the parent spec's per-pipe ordering guarantee).
- `applying_remote` flag set during apply (verify via debug print).

**B.7.3.3**:
- `task package` builds clean after deletion.
- All InstancePanel / window-list UI behaves identically to v0.33.502 (the B.7.3.1 baseline).

---

## Open questions

1. **Per-renderer subscription vs broadcast.** Current design broadcasts to every top-level browser. Should we let renderers register/deregister subscriptions (e.g., a renderer that doesn't show window state opts out)? B.7.3.1 broadcasts; refinement deferred — keep it simple.
2. **Event volume.** Reducer can fire 10–50 events per user action (e.g., open window: WindowOpened + InstanceAssigned + BackendWindowIdRegistered + HwndOpened + HwndForegroundChanged + HwndPositionChanged + ...). Each costs one ExecuteJavaScript per top-level renderer. With 5 renderers and 10 events per action, that's 50 IPC roundtrips per user action. Profiling needed before B.7.3.3 retirement; if it shows up as jank, batch dispatch.
3. **Drift event UX.** Drift events (`OffMonitor`, `HiddenSinceOpen`, etc.) currently only land in logs. Should the renderer surface them as toasts? B.7.3.1 makes them available at the renderer; UX wiring is a separate UX-design question.
4. **`task dev` parity.** Without the launcher, no typed events flow. The renderer falls back to the bespoke channel. Long-term plan (Phase E srv reducer) makes the launcher unconditionally part of the loop, retiring `task dev`'s no-launcher mode. Until then, dual-path stays.

---

## Cross-platform notes

- `Frame::ExecuteJavaScript` is portable across CEF's Windows / macOS / Linux backends. Same bridge code works everywhere.
- The renderer-side dispatcher is plain JavaScript on a CEF V8 context; no platform specifics.
- Phase 7 cross-platform parity work doesn't change anything in B.7.3 — the bridge is OS-agnostic.

---

## What this does NOT do

- **No `GetSnapshot` resync protocol.** Parent spec §5.4. Required for proper reconnect, but B.7.3 assumes the host stays connected for its lifetime (current reality post-B.6). Phase D adds resync.
- **No frontend → launcher commands via this channel.** Commands still flow renderer → host IPC HTTP → launcher pipe. The reverse channel (commands ON the JS bridge) is a Phase E concern.
- **No event ordering across renderers.** Each renderer gets the same events in the same order, but the renderers' apply order isn't synchronized between each other. Per parent spec §5.3, ordering is per-pipe; broadcast doesn't preserve cross-subscriber ordering. Acceptable for AgentMux's use cases.
