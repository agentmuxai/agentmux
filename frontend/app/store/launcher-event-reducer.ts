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
// Phase B.7.3.2 (this PR): typed events become AUTHORITATIVE for
// the InstancePanel atoms (`openWindowLabelsAtom`,
// `openWindowEntriesAtom`, `windowCountAtom`). The reducer
// maintains an in-memory `knownEntries` map of `label →
// {label, windowId}`, applies deltas from typed events, and
// recomputes the atoms after each apply. The bespoke
// `window-instances-changed` listener in `app-init.ts` is gated
// by `!launcherEventsActive()` — it only runs when the launcher
// isn't in the loop (e.g. `task dev` mode).
// Phase B.7.3.3: retire the bespoke channel + 4 sync emit sites.
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
 * `isInstanceLabel` mirrors the legacy filter in
 * `app-init.ts::initInstanceTracking`. Sub-windows + main + full
 * instances all match `/^window-/` or === "main". `window-pool-*`
 * also matches, but pool windows don't fire `WindowOpened`, so
 * they never enter `knownEntries`. Defensive belt anyway: filter
 * out `window-pool-` here too in case the host's launcher
 * implementation ever changes.
 */
function isInstanceLabel(label: string): boolean {
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
 * Idempotent w.r.t. repeated calls: each call clears + re-seeds.
 * Repeated typed-event apply on the same label is also idempotent
 * (`Map.set` overwrites; `Map.delete` of a missing key is a no-op).
 */
export function seedKnownEntriesFromSnapshot(entries: ReadonlyArray<WindowEntry>): void {
    knownEntries.clear();
    for (const e of entries) {
        if (isInstanceLabel(e.label)) {
            knownEntries.set(e.label, { label: e.label, windowId: e.windowId });
        }
    }
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
            if (knownEntries.delete(label)) {
                recomputeAtoms();
            }
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
            if (knownEntries.delete(label)) {
                recomputeAtoms();
            }
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
        case "corrective_window_move":
        case "host_should_quit":
            // WRR / saga events. Logged only — UX wiring deferred.
            return;
        default:
            // Forward-compat: unknown variants silently ignored.
            return;
    }
}
