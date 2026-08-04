// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Frontend reducer dispatch + projection layer for launcher typed events.
//
// Refactored in PR-B (slice #6 of the frontend reducer roadmap, 2026-05-03)
// to follow the conventions established in
// docs/specs/frontend-reducer-conventions-2026-05-03.md. The pure reducer
// + types live in `./launcher-event/`; this file owns:
//   - the in-memory state cell (single global slice)
//   - the SolidJS effect that subscribes to the launcherEvent signal
//   - the projection layer (writes derived state into global atoms)
//   - the echo-loop guard (`applyingRemote`)
//   - the public API: startLauncherEventReducer, seedKnownEntriesFromSnapshot,
//     isApplyingRemoteEvent
//
// Behavior is unchanged from the prior in-place dispatch — see
// `launcher-event/reducer.test.ts` for the 19 backfilled tests.
//
// History context (kept for reviewer):
// - Phase B.7.3.1 (PR #602): scaffolding only — logged events, no atom mutation.
// - Phase B.7.3.2 (PR #603): typed events became authoritative for InstancePanel.
// - Phase B.7.3.3 (PR #604): bespoke `window-instances-changed` channel retired.
// - PR-B (this PR): pure refactor + tests. No behavior change.
//
// See docs/specs/SPEC_B_7_3_LAUNCHER_EVENTS_TO_RENDERER_2026_04_29.md.

import { createEffect } from "solid-js";

import { launcherEvent, launcherEventVersion, launcherEventGapSeq } from "@/util/launcher-events";
import { getApi } from "@/store/app-api";
import {
    setOpenWindowEntriesAtom,
    setOpenWindowLabelsAtom,
    setOpenFloatingPaneEntriesAtom,
    setWindowCountAtom,
    type FloatingPaneEntry,
    type WindowEntry,
} from "@/app/store/global";

import { update } from "./launcher-event/reducer";
import {
    initialState,
    isFloatingPaneLabel,
    LauncherEventCommand,
    LauncherEventReducerEvent,
    LauncherEventState,
} from "./launcher-event/types";
import { recordDispatch } from "./command-source";

// ── State cell ─────────────────────────────────────────────────────────

let state: LauncherEventState = initialState();

// ── Floating pane state cell ───────────────────────────────────────────
//
// Tracked separately from the window reducer — floating panes don't need
// the tombstone / seed-race machinery (they can't arrive before their own
// window_opened event and the pre-seed close race doesn't apply to them).
// Same event stream, different label filter: `isFloatingPaneLabel`.

let floatingPanes: Map<string, FloatingPaneEntry> = new Map();

function handleFloatingPaneEvent(evt: { event: string; label?: unknown; window_id?: unknown }): boolean {
    const label = String(evt.label ?? "");
    if (!label || !isFloatingPaneLabel(label)) return false;

    switch (evt.event) {
        case "window_opened":
        case "window_instance_assigned": {
            if (floatingPanes.has(label)) return false;
            floatingPanes = new Map(floatingPanes);
            floatingPanes.set(label, { label, windowId: null });
            return true;
        }
        case "window_closed":
        case "window_instance_released": {
            if (!floatingPanes.has(label)) return false;
            floatingPanes = new Map(floatingPanes);
            floatingPanes.delete(label);
            return true;
        }
        case "backend_window_id_registered": {
            const windowId = typeof evt.window_id === "string" ? evt.window_id : null;
            const existing = floatingPanes.get(label);
            if (existing?.windowId === windowId) return false;
            floatingPanes = new Map(floatingPanes);
            // Ensure an entry exists even if window_opened was missed.
            floatingPanes.set(label, { label, windowId });
            return true;
        }
        case "backend_window_id_unregistered": {
            const existing = floatingPanes.get(label);
            if (!existing || existing.windowId === null) return false;
            floatingPanes = new Map(floatingPanes);
            floatingPanes.set(label, { label, windowId: null });
            return true;
        }
        default:
            return false;
    }
}

function projectFloating(): void {
    setOpenFloatingPaneEntriesAtom(
        [...floatingPanes.values()].sort((a, b) => a.label.localeCompare(b.label)),
    );
}

// ── Echo-loop guard ────────────────────────────────────────────────────

let applyingRemote = false;

/**
 * True while the reducer is mid-apply for a launcher event. Future
 * renderer-emitted commands should check this and skip re-emission to
 * avoid echo loops with the launcher.
 *
 * Currently no commands flow through this bridge (commands still take
 * the host IPC HTTP path), so the flag is forward-compatibility
 * scaffolding — same as before the refactor.
 */
export function isApplyingRemoteEvent(): boolean {
    return applyingRemote;
}

// ── Dispatch + projection ──────────────────────────────────────────────

function dispatch(command: LauncherEventCommand): LauncherEventReducerEvent[] {
    const prev = state;
    const result = update(prev, command);
    state = result.state;
    if (state.instances !== prev.instances) project();
    for (const ev of result.events) onAuditEvent(ev);
    // Source is always "system" — this slice mirrors upstream events
    // (launcher channel) and the snapshot seed; no user-driven path.
    recordDispatch({
        slice: "launcher-event",
        key: null,
        command,
        events: result.events,
        source: "system",
        at: Date.now(),
    });
    return result.events;
}

/**
 * Project derived state to the global atoms. Called only when
 * `instances` changed (referential equality), avoiding redundant atom
 * writes that would re-run subscribers.
 */
function project(): void {
    setOpenWindowLabelsAtom(state.instances.map((e) => e.label));
    setOpenWindowEntriesAtom([...state.instances]);
    setWindowCountAtom(state.instances.length);
}

/**
 * Audit-event sink. Notable variants are logged to console (matching
 * pre-refactor behavior); the rest are silent. The diagnostics-panel
 * surface will hook this once PR-C ships.
 */
function onAuditEvent(event: LauncherEventReducerEvent): void {
    if (event.type === "drift-detected") {
        console.warn("[launcher-event] drift", event.raw);
    } else if (event.type === "saga-event-observed") {
        console.info("[launcher-event]", event.eventName, event.raw);
    }
}

// ── Public API ─────────────────────────────────────────────────────────

/**
 * Seed `knownEntries` from the init RPC snapshot. Called once from
 * `app-init.ts::initInstanceTracking` after `listWindowInstances`
 * returns. The reducer's ApplySeed arm preserves existing entries
 * (codex P1 #603) and skips tombstoned labels (codex P2 #603).
 */
export function seedKnownEntriesFromSnapshot(
    entries: ReadonlyArray<WindowEntry>,
): void {
    dispatch({ type: "ApplySeed", entries });
    // Seed floating panes from the same snapshot. Mirrors ApplySeed semantics:
    // additive only — skip labels already in floatingPanes so a window_opened /
    // backend_window_id_registered event that raced the snapshot RPC is not
    // clobbered with a stale null windowId (same race as codex P1 #603).
    // Called unconditionally at boot; dev-mode guard lives at the call site.
    let changed = false;
    const next = new Map(floatingPanes);
    for (const e of entries) {
        if (isFloatingPaneLabel(e.label) && !next.has(e.label)) {
            next.set(e.label, { label: e.label, windowId: e.windowId });
            changed = true;
        }
    }
    if (changed) {
        floatingPanes = next;
        projectFloating();
    }
}

/**
 * Reconcile knownEntries against a fresh `listWindowInstances` snapshot.
 * Differs from `seedKnownEntriesFromSnapshot` (`ApplySeed`) in that it
 * REPLACES the known set wholesale — labels absent from the snapshot
 * are removed from the panel.
 *
 * Use for periodic refresh paths (e.g. InstancePanel reopens in
 * `task dev` mode where the launcher doesn't push WindowClosed
 * events). Don't use at boot — `ApplySeed` is the right boot path
 * because it's additive against typed events that may have raced
 * the snapshot fetch (codex P1 #603).
 */
export function reconcileKnownEntriesFromSnapshot(
    entries: ReadonlyArray<WindowEntry>,
): void {
    dispatch({ type: "ReconcileFromSnapshot", entries });
    // Wholesale replace the floating panes map with the fresh snapshot.
    const nextFloating = new Map<string, FloatingPaneEntry>();
    for (const e of entries) {
        if (isFloatingPaneLabel(e.label)) {
            nextFloating.set(e.label, { label: e.label, windowId: e.windowId });
        }
    }
    floatingPanes = nextFloating;
    projectFloating();
}

/**
 * Re-pull the authoritative window-instance snapshot and reconcile after the
 * launcher event stream drops one or more events (a detected version gap).
 *
 * The stream is best-effort per-renderer dispatch — a dropped `window_closed`
 * leaves this renderer's `knownEntries` permanently over-counting (the window
 * count "(N)" never decrements; the observed "3 vs 4" desync). `ReconcileFromSnapshot`
 * heals it (wholesale add/remove vs the authoritative `list_window_instances`).
 *
 * Race guard (mirrors the codex P1 #733 concern that gated the InstancePanel
 * reconcile to dev-only): capture the event version BEFORE the async RPC; if a
 * newer typed event lands while the RPC is in flight, that event already
 * advanced state past the snapshot — discard the reconcile rather than clobber
 * fresh state with a stale snapshot. We reconcile ONLY on a detected gap, never
 * unconditionally, so this never fights the normal event flow.
 * See `docs/specs/SPEC_WINDOW_COUNT_STALE_ON_VIEWS_CLOSE_2026_06_22.md` §9.
 */
async function resyncFromAuthorityAfterGap(attempt = 0): Promise<void> {
    const versionAtRequest = launcherEventVersion();
    try {
        const snapshot = await getApi().listWindowInstances();
        if (!Array.isArray(snapshot)) return;
        if (launcherEventVersion() !== versionAtRequest) {
            // A newer event arrived while the RPC was in flight — state advanced past
            // the snapshot. Retry once after a short delay: a busy event stream can
            // race every attempt indefinitely without this, leaving a stale count
            // permanently. One retry is sufficient for the common case.
            // See docs/retro/retro-window-count-stale-post-1701-2026-06-27.md §Gap C.
            if (attempt === 0) setTimeout(() => void resyncFromAuthorityAfterGap(1), 500);
            return;
        }
        reconcileKnownEntriesFromSnapshot(snapshot);
    } catch (e) {
        console.error("[launcher-event] gap resync failed", e);
    }
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
            dispatch({ type: "ApplyEvent", event: evt });
            if (handleFloatingPaneEvent(evt)) projectFloating();
        } finally {
            applyingRemote = false;
        }
    });

    // Resync-on-gap: the launcher event stream is lossy, and a dropped event
    // leaves this renderer's `instances` stale forever. When the tracker detects
    // a version gap it bumps `launcherEventGapSeq`; reconcile against the
    // authoritative snapshot. Seeded at 0 so the initial run is a no-op (only a
    // real gap, seq > prev, triggers a resync).
    createEffect((prevSeq: number) => {
        const seq = launcherEventGapSeq();
        if (seq > prevSeq) void resyncFromAuthorityAfterGap();
        return seq;
    }, 0);

    // Safety-net: periodic reconcile every 30s against the authoritative
    // list_window_instances to catch any gap that went undetected (e.g. no
    // subsequent events arrived after a missed WindowClosed, so gapSeq never
    // bumped). Low cost — one RPC per 30s per renderer. Complements the
    // gap-triggered reconcile; does not replace it.
    // See docs/retro/retro-window-count-stale-post-1701-2026-06-27.md §Gap C.
    // Reducer lives for the renderer's lifetime; interval is reclaimed on unload.
    setInterval(() => void resyncFromAuthorityAfterGap(), 30_000);
}
