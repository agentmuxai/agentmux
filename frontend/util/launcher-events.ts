// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Phase B.7.3.1 — renderer-side subscriber for launcher typed events.
// Phase E.6 — multi-source dispatcher with version tracking + saga buffering.
//
// Mirrors `agentmux_common::ipc::Event` on the wire. The host's CEF
// JS bridge (`agentmux-cef/src/launcher_event_bridge.rs`) calls
// `window.__agentmux_launcher_event(<json>)` once per top-level
// renderer per launcher event.
//
// Phase B.7.3.3 — typed events are the SOLE path for InstancePanel
// state. The bespoke `window-instances-changed` channel and its 4
// sync emit sites in the host are gone.
//
// Phase E.6 — dispatcher backed by the shared `PerSourceTracker`.
// Same version-monotonicity + saga-buffer semantics as the srv pipe.
// See `util/event-buffer.ts` for the contract.
//
// See `docs/specs/SPEC_B_7_3_LAUNCHER_EVENTS_TO_RENDERER_2026_04_29.md`.

import { createSignal } from "solid-js";
import { PerSourceTracker, type EventCallback, type VersionedEvent } from "./event-buffer";

/**
 * Wire-format launcher event. Matches the JSON serialization of
 * `agentmux_common::ipc::Event` (`#[serde(tag = "event", rename_all = "snake_case")]`).
 */
export type LauncherEvent = VersionedEvent;

const [latestEvent, setLatestEvent] = createSignal<LauncherEvent | null>(null);
const [eventVersion, setEventVersion] = createSignal<number>(0);
const [seenAnyEvent, setSeenAnyEvent] = createSignal<boolean>(false);
const [gapSeq, setGapSeq] = createSignal<number>(0);

/** Latest typed event delivered by the launcher. Null until the first event. */
export const launcherEvent = latestEvent;

/** Monotonic version of the most recent event. 0 until the first event. */
export const launcherEventVersion = eventVersion;

/**
 * Monotonic counter, bumped each time the launcher event stream detects a
 * VERSION GAP (one or more events dropped on the wire). The per-renderer
 * incremental `instances` state cannot self-heal from a dropped event — a
 * missed `window_closed` leaves the window count permanently over-counting
 * (the "3 vs 4" desync). The launcher-event reducer reacts to this signal by
 * re-pulling the authoritative `list_window_instances` snapshot and
 * reconciling. 0 until the first gap.
 * See `docs/specs/SPEC_WINDOW_COUNT_STALE_ON_VIEWS_CLOSE_2026_06_22.md` §9.
 */
export const launcherEventGapSeq = gapSeq;

/**
 * True once we've received at least one typed event. Kept as a
 * forward-compat utility — B.7.3.3 retired its only consumer (the
 * bespoke-channel gate in `app-init.ts`), but Phase D's `GetSnapshot`
 * resync flow may need it again.
 */
export const launcherEventsActive = seenAnyEvent;

const tracker = new PerSourceTracker<LauncherEvent>(
    {
        source: "launcher",
        onVersionGap: (gap, prev, next) => {
            // Keep the diagnostic warning (matches the default handler)…
            console.warn(
                `[launcher-events] version gap: expected ${prev + 1}, got ${next} (${gap} event${gap === 1 ? "" : "s"} possibly dropped); scheduling authoritative resync`,
            );
            // …and signal the reducer to reconcile against the authority.
            setGapSeq((n) => n + 1);
        },
    },
    {
        setLatest: setLatestEvent,
        setVersion: setEventVersion,
        setSawAny: setSeenAnyEvent,
    },
);

/**
 * Per-event subscriber for the launcher pipe. Same contract as
 * `subscribeSrvEvent`: every event in source order, even during
 * saga buffer flushes. Returns an unsubscribe function.
 */
export function subscribeLauncherEvent(cb: EventCallback<LauncherEvent>): () => void {
    return tracker.subscribe(cb);
}

let installed = false;

// Drift-storm guard (v0.33.655 smoke): the host fanned a single
// `hwnd_drift_detected` event ~600× to the renderer in 3 seconds —
// V8 stack exhaustion → renderer crash. The launcher-side reset
// (`handle_report_pool_window_promoted` setting
// `foregrounded_since_open=true`) stops the storm at source; this
// per-key cache is the renderer-side defence-in-depth so a future
// upstream regression can't crash us. Keyed by
// `(event, label, hwnd)`; max version observed wins. PerSourceTracker
// already drops by global monotonic version, but a per-key check
// catches duplicates that the global counter advances past.
//
// See `docs/specs/ANALYSIS_DRIFT_STORM_RENDERER_CRASH_2026-05-06.md`.
const MAX_DEDUP_KEYS = 1024;
const dedupSeen = new Map<string, number>();
let dedupSuppressedCount = 0;

function dedupKey(evt: LauncherEvent): string {
    const e = evt as LauncherEvent & { label?: unknown; hwnd?: unknown };
    const label = typeof e.label === "string" ? e.label : "";
    const hwnd = typeof e.hwnd === "number" ? e.hwnd : "";
    return `${evt.event}|${label}|${hwnd}`;
}

/**
 * Per-key (event-kind, label, hwnd) version-monotonicity guard.
 * Returns true if the event should be dispatched, false if it's a
 * duplicate or stale re-emission for the same logical entity.
 *
 * **Launcher restart pass-through (codex P1, this PR round 2):** when
 * the launcher process restarts, its `event_version` resets to 1
 * (`agentmux-launcher/src/state.rs::default`). `PerSourceTracker.deliver`
 * has explicit handling for the v=1 sentinel — it resets `lastVersion`
 * so post-restart events aren't dropped as stale. This bridge-level
 * dedup runs BEFORE the tracker, so without an equivalent reset the
 * sentinel would be suppressed against a cached key from the prior
 * incarnation (e.g. a long-lived label like `main`) and the tracker
 * would never see v=1, leaving subsequent low-version events
 * permanently stuck behind `lastVersion` from the dead launcher.
 * Heuristic mirrors the tracker's: `evt.version === 1` AND we've
 * recorded at least one prior version → clear the cache and admit.
 */
export function shouldDispatchLauncherEvent(evt: LauncherEvent): boolean {
    // Defensive shape check (codex P2, this PR round 3): if the event
    // is malformed (null, non-object, missing required fields), defer
    // to PerSourceTracker.deliver — it has the canonical
    // log-and-discard path. Touching `evt.event` / `evt.version` here
    // on a null/non-object would throw out of __agentmux_launcher_event,
    // breaking the bridge for subsequent events.
    if (
        evt == null ||
        typeof evt !== "object" ||
        typeof (evt as { version?: unknown }).version !== "number" ||
        typeof (evt as { event?: unknown }).event !== "string"
    ) {
        return true;
    }
    if (evt.version === 1 && hasSeenAnyVersionAbove(0)) {
        dedupSeen.clear();
        // Don't reset suppressed counter — it's a cumulative diag.
    }
    const k = dedupKey(evt);
    const seenVersion = dedupSeen.get(k);
    if (seenVersion !== undefined && evt.version <= seenVersion) {
        dedupSuppressedCount++;
        return false;
    }
    dedupSeen.set(k, evt.version);
    if (dedupSeen.size > MAX_DEDUP_KEYS) {
        // Bound memory. Map iteration order is insertion order → drop
        // the oldest entry. Acceptable trade-off: an evicted key's
        // next arrival is admitted unconditionally (whatever its
        // version), but the worst case is one duplicate event slipping
        // through after a 1024-key eviction window. The
        // `PerSourceTracker` behind this still drops by global
        // monotonic version, so a same-version evicted-key duplicate
        // is still caught one layer down.
        const firstKey = dedupSeen.keys().next().value;
        if (firstKey !== undefined) dedupSeen.delete(firstKey);
    }
    return true;
}

function hasSeenAnyVersionAbove(threshold: number): boolean {
    for (const v of dedupSeen.values()) {
        if (v > threshold) return true;
    }
    return false;
}

/** Diagnostic accessor for the dedup guard — used by tests + diag tools. */
export function launcherEventDedupStats(): { tracked: number; suppressed: number } {
    return { tracked: dedupSeen.size, suppressed: dedupSuppressedCount };
}

/**
 * Reset the dedup cache. Test-only by convention — the `__` prefix is
 * the project's marker for non-production APIs (see `__snapshot` /
 * `__resetState` in `app/store/launcher-event-reducer.ts`). Production
 * code MUST NOT call this; the only legitimate cache reset is the
 * launcher-restart sentinel inside `shouldDispatchLauncherEvent`.
 */
export function __resetDedupForTests(): void {
    dedupSeen.clear();
    dedupSuppressedCount = 0;
}

/**
 * Register `window.__agentmux_launcher_event` as the dispatcher.
 * Idempotent — safe to call multiple times. Called once from
 * `app-init.ts::initApp` BEFORE the first state-needing operation
 * so events that arrive during init aren't dropped.
 */
export function installLauncherEventBridge(): void {
    if (installed) return;
    installed = true;
    (window as any).__agentmux_launcher_event = (evt: LauncherEvent) => {
        if (!shouldDispatchLauncherEvent(evt)) return;
        tracker.deliver(evt);
    };
    console.log("[launcher-events] bridge installed; window.__agentmux_launcher_event ready");
}
