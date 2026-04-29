// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Phase B.7.3 — frontend reducer for launcher typed events.
//
// Subscribes to the `launcherEvent` signal from
// `frontend/util/launcher-events.ts` and dispatches by `event`
// discriminant.
//
// Phase B.7.3.1 (PR #602): scaffolding only — logged every event,
// no atom mutation.
// Phase B.7.3.2 (PR #603): typed events became AUTHORITATIVE for
// the InstancePanel atoms (`openWindowLabelsAtom`,
// `openWindowEntriesAtom`, `windowCountAtom`). Bespoke channel
// demoted to fallback.
// Phase B.7.3.3 (this PR): bespoke `window-instances-changed`
// channel and its 4 sync emit sites in the host are RETIRED.
// Typed events are the sole path. `task dev` mode (no launcher)
// leaves the panel at its init-RPC-seeded values; live updates
// require the launcher in the loop.
//
// State seed: at init, `app-init.ts::initInstanceTracking` calls
// `seedKnownEntriesFromSnapshot` with the RPC `listWindowInstances`
// result. This populates `knownEntries` so a renderer that joins
// mid-session sees the existing windows. Subsequent typed events
// apply deltas on top.
//
// Echo-loop guard (parent spec §5.5): an `applyingRemote` flag is
// set during each apply so future renderer-emitted commands can
// detect "this state change came from the launcher; don't re-emit."
// Currently no commands flow through this bridge (commands still
// take the host IPC HTTP path), so the flag is forward-compatibility
// scaffolding.
//
// See `docs/specs/SPEC_B_7_3_LAUNCHER_EVENTS_TO_RENDERER_2026_04_29.md`.

import { createEffect } from "solid-js";

import { launcherEvent, launcherEventVersion, type LauncherEvent } from "@/util/launcher-events";
import {
    setOpenWindowEntriesAtom,
    setOpenWindowLabelsAtom,
    setWindowCountAtom,
    type WindowEntry,
} from "@/app/store/global";

let applyingRemote = false;

/**
 * True while the reducer is mid-apply for a launcher event. Future
 * renderer-emitted commands should check this and skip re-emission
 * to avoid echo loops with the launcher.
 */
export function isApplyingRemoteEvent(): boolean {
    return applyingRemote;
}

/**
 * In-memory mirror of label → windowId for top-level + sub windows.
 * Pool labels (`window-pool-*`) and browser-pane labels are excluded —
 * pool windows fire `Event::PoolWindowAdded/Removed` (different
 * event variants we ignore here), and browser-pane HWNDs never
 * appear as `Event::WindowOpened`.
 */
const knownEntries = new Map<string, WindowEntry>();

/**
 * Set to `true` once `seedKnownEntriesFromSnapshot` has run. Until
 * then, close events are recorded as tombstones (see
 * `closedBeforeSeed`) so the seed can skip labels that already
 * closed pre-seed. (codex P2 #603.)
 */
let seedHasHappened = false;

/**
 * Tombstone set: labels for which a `WindowClosed` /
 * `WindowInstanceReleased` arrived BEFORE seed ran. The reducer's
 * close path on an empty `knownEntries` is a no-op delete, so
 * without this set, the seed would re-add the label from the
 * stale RPC snapshot and the InstancePanel would carry a ghost
 * row (codex P2 #603 — the seed-vs-close race).
 *
 * Drained at seed time. Stays `null` after seed (close events
 * apply directly to `knownEntries`).
 */
let closedBeforeSeed: Set<string> | null = new Set();

/**
 * Filter for labels the InstancePanel surfaces. Sub-windows + main
 * + full instances all match `/^window-/` or === "main". Pool
 * (`window-pool-*`) and browser-pane child HWNDs are excluded —
 * pool windows fire `Event::PoolWindowAdded/Removed` (different
 * event variants we don't subscribe to), and browser-pane HWNDs
 * never appear as `Event::WindowOpened`. Defensive filter even so.
 *
 * Exported so `app-init.ts::initInstanceTracking` (the bespoke
 * fallback path + the snapshot-key seed) uses the SAME filter as
 * the typed-event reducer. (reagent P2 #603 — the two filters
 * had diverged on `window-pool-*`.)
 */
export function isInstanceLabel(label: string): boolean {
    if (label === "main") return true;
    if (label.startsWith("window-pool-")) return false;
    if (label.startsWith("browser-pane-")) return false;
    return label.startsWith("window-");
}

function recomputeAtoms(): void {
    const filtered = Array.from(knownEntries.values()).filter((e) => isInstanceLabel(e.label));
    // Stable ordering: "main" pinned first, others alphabetical.
    // Without this, panel rows shuffle across recomputes (Map
    // iteration is insertion-ordered, but the InstancePanel uses
    // array index for naming and special-cases "main").
    filtered.sort((a, b) => {
        if (a.label === "main") return -1;
        if (b.label === "main") return 1;
        return a.label.localeCompare(b.label);
    });
    setOpenWindowLabelsAtom(filtered.map((e) => e.label));
    setOpenWindowEntriesAtom(filtered);
    setWindowCountAtom(filtered.length);
}

/**
 * Seed `knownEntries` from the init RPC snapshot. Called once from
 * `app-init.ts::initInstanceTracking` after `listWindowInstances`
 * returns. Subsequent typed events apply deltas on top.
 *
 * **Important — does NOT clobber existing entries.** The reducer
 * effect is started earlier (in `initWaveWrap`, before
 * `initInstanceTracking` runs the RPC), so typed events can arrive
 * between snapshot-fetch and this seed call. A `BackendWindowIdRegistered`
 * that landed first would have updated `windowEntries[label].windowId`
 * to a real value; the snapshot — taken at an earlier moment — may
 * still show `null` for that label. Clobbering would roll the
 * windowId back to stale `null`, and since `launcherEventsActive()`
 * is now true, the bespoke fallback channel won't recover it. (codex
 * P1 #603.) So this function fills in MISSING labels only; existing
 * entries are left as the typed-event stream wrote them.
 */
export function seedKnownEntriesFromSnapshot(entries: ReadonlyArray<WindowEntry>): void {
    const tombstones = closedBeforeSeed;
    for (const e of entries) {
        if (!isInstanceLabel(e.label)) continue;
        if (knownEntries.has(e.label)) continue;
        // Pre-seed close tombstone — label closed between snapshot
        // fetch and seed; the close arm couldn't delete from an
        // empty map, so we skip the re-add here. (codex P2 #603.)
        if (tombstones && tombstones.has(e.label)) continue;
        knownEntries.set(e.label, { label: e.label, windowId: e.windowId });
    }
    seedHasHappened = true;
    closedBeforeSeed = null;
    recomputeAtoms();
}

let started = false;

/**
 * Start the reducer effect. Idempotent. Called once per renderer
 * after `initWaveWrap` so global state is ready before the first
 * apply touches atoms.
 */
export function startLauncherEventReducer(): void {
    if (started) return;
    started = true;

    createEffect(() => {
        const evt = launcherEvent();
        // Read version too so SolidJS tracks the version signal as a
        // dependency — guarantees the effect re-runs even when two
        // consecutive events have referentially-equal payloads (e.g.
        // a same-shape OffMonitor drift firing twice).
        launcherEventVersion();
        if (!evt) return;

        applyingRemote = true;
        try {
            dispatch(evt);
        } finally {
            applyingRemote = false;
        }
    });
}

function dispatch(evt: LauncherEvent): void {
    switch (evt.event) {
        case "window_opened": {
            const label = String(evt.label ?? "");
            if (!label || !isInstanceLabel(label)) return;
            // Preserve existing windowId if a BackendWindowIdRegistered
            // arrived before WindowOpened (rare, but the launcher's
            // event ordering between separate Commands isn't
            // synchronized w.r.t. renderer apply order — the per-pipe
            // ordering guarantee is the host → renderer hop, not
            // host's emission order across distinct host-side
            // mutations).
            const existing = knownEntries.get(label);
            knownEntries.set(label, { label, windowId: existing?.windowId ?? null });
            recomputeAtoms();
            return;
        }
        case "window_closed": {
            const label = String(evt.label ?? "");
            if (!label) return;
            const deleted = knownEntries.delete(label);
            if (!seedHasHappened && closedBeforeSeed) {
                // Pre-seed: record a tombstone so seed skips this
                // label even if it appears in the snapshot (codex P2
                // #603). The delete above handles the case where
                // WindowOpened arrived first pre-seed — and if it
                // did delete an existing entry, atoms must recompute
                // here too, otherwise the ghost row persists until
                // seed runs (or forever if seed fails). (codex P2
                // #604.)
                closedBeforeSeed.add(label);
                if (deleted) recomputeAtoms();
                return;
            }
            if (deleted) recomputeAtoms();
            return;
        }
        case "window_instance_assigned": {
            // The reducer doesn't track instance numbers — the host's
            // bespoke channel previously carried `count`, but the
            // count is now derived from `knownEntries.size`. We still
            // ensure the entry is in the map (defensive — should
            // already be from WindowOpened).
            const label = String(evt.label ?? "");
            if (!label || !isInstanceLabel(label)) return;
            if (!knownEntries.has(label)) {
                knownEntries.set(label, { label, windowId: null });
                recomputeAtoms();
            }
            return;
        }
        case "window_instance_released": {
            // Released usually pairs with WindowClosed which already
            // deleted the entry; idempotent here.
            const label = String(evt.label ?? "");
            if (!label) return;
            const deleted = knownEntries.delete(label);
            if (!seedHasHappened && closedBeforeSeed) {
                closedBeforeSeed.add(label);
                if (deleted) recomputeAtoms();
                return;
            }
            if (deleted) recomputeAtoms();
            return;
        }
        case "backend_window_id_registered": {
            const label = String(evt.label ?? "");
            const windowId = typeof evt.window_id === "string" ? evt.window_id : null;
            if (!label || !isInstanceLabel(label)) return;
            const existing = knownEntries.get(label);
            // If the entry doesn't exist yet, create it — out-of-order
            // arrival path. WindowOpened, when it lands, preserves
            // this windowId (see preserve-existing-windowId branch
            // in `window_opened` arm).
            knownEntries.set(label, { label, windowId });
            // Skip recompute if windowId actually didn't change AND
            // the entry was already present — avoids a redundant
            // atom write (SolidJS would treat a referentially-new
            // array as a change and re-run subscribers).
            if (existing && existing.windowId === windowId) return;
            recomputeAtoms();
            return;
        }
        case "backend_window_id_unregistered": {
            const label = String(evt.label ?? "");
            if (!label || !isInstanceLabel(label)) return;
            const existing = knownEntries.get(label);
            if (!existing) return;
            if (existing.windowId === null) return;
            knownEntries.set(label, { label, windowId: null });
            recomputeAtoms();
            return;
        }
        case "hwnd_drift_detected":
            // Drift events fire when the launcher's pure reducer
            // detects an off-monitor / hidden / orphan window.
            // Currently logged at warn so they surface in the
            // backend-forwarded `[fe]` log without being silent on
            // the user side. UX wiring (toast / debug panel) is a
            // separate question — see spec §Open questions #3.
            // (reagent P2 #603.)
            console.warn("[launcher-event] drift", evt);
            return;
        case "corrective_window_move":
        case "host_should_quit":
            // Saga events handled host-side. Renderer logs at info
            // for observability — these are rare and meaningful.
            console.info("[launcher-event]", evt.event, evt);
            return;
        default:
            // Forward-compat: unknown variants silently ignored.
            return;
    }
}
