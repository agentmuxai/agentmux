// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Browser pane state store — slice #9 Phase 4 of the frontend reducer
 * roadmap. The cell-by-cell migration (Phases 3a-3e) put every
 * per-pane data cell behind a pure reducer; this slot store finishes
 * the slice by externalizing state ownership from
 * `BrowserViewModel` and routing every dispatch through the
 * `recordDispatch` audit ring.
 *
 * Pattern matches slice #4 (`agent-pane-state-store.ts`) — same slot
 * lifecycle (`registerPane` / `dispatch` / `unregisterPane`), same
 * "throw on unregistered dispatch" rule (silent drops defeat the
 * reducer), same projection-on-change discipline.
 *
 * What this enables:
 *   - **Audit ring** — every browser-pane state transition shows up
 *     in `dispatchRecordsAtom`, queryable by the diagnostics panel
 *     (Phase 5) and instantly visible during multi-pane focus or
 *     pool-drift investigations.
 *   - **Click event through reducer** — the focus-routing fix in
 *     PR #760 lives in the IPC handler today; once `PaneClicked`
 *     becomes a reducer command, the side-effect (blur stale main
 *     input + refocus block) flows through dispatch and is recorded
 *     just like every other transition.
 */

import { update } from "./browser-pane-state/reducer";
import {
    BrowserPaneCommand,
    BrowserPaneEvent,
    BrowserPaneState,
    initialState,
} from "./browser-pane-state/types";
import { type CommandSource, recordDispatch } from "./command-source";

/**
 * Setters the slot writes into when reducer state changes. The view
 * (`BrowserViewModel`) owns the underlying SolidJS signals and passes
 * just the setters in via `registerPane`. Readers keep using the
 * model's existing accessors (`urlAtom`, `loadingAtom`, etc.) — only
 * writes are routed through the slot.
 *
 * Mirrors the `_dispatch` projector that lived inline in the model
 * before Phase 4. Each setter is called only when the reducer state
 * field actually changes (referential equality), preserving the
 * Phase 3 discipline that avoided spurious reactive churn on the
 * address-bar typing path PR #737 regressed.
 */
export interface BrowserPaneProjections {
    closed: (next: boolean) => void;
    loading: (next: boolean) => void;
    error: (next: string | null) => void;
    canGoBack: (next: boolean) => void;
    canGoForward: (next: boolean) => void;
    title: (next: string) => void;
    url: (next: string) => void;
    faviconUrl: (next: string) => void;
}

interface Slot {
    state: BrowserPaneState;
    proj: BrowserPaneProjections;
}

const slots = new Map<string, Slot>();

type EventSink = (blockId: string, event: BrowserPaneEvent) => void;
let eventSink: EventSink = (_blockId, _event) => {
    // Default sink is a no-op. The view layer installs a real sink
    // via `setEventSink` to wire `pane-click-blur` (and other
    // future view-effecting events) to the DOM blur + refocus
    // handlers. Tests run with the no-op so the slot store is
    // testable without DOM.
};

export function setEventSink(sink: EventSink): void {
    eventSink = sink;
}

/**
 * Register a pane. Call SYNCHRONOUSLY from the model's constructor,
 * before any IPC handler can dispatch. Re-registering a blockId
 * resets the state cell to initialState — useful for the
 * `BrowserViewModel`'s post-dispose cleanup if a new model gets the
 * same blockId on hot-reload.
 */
export function registerPane(
    blockId: string,
    proj: BrowserPaneProjections,
): void {
    slots.set(blockId, { state: initialState(), proj });
}

export function unregisterPane(blockId: string): void {
    slots.delete(blockId);
}

/**
 * Apply a command. Throws on unregistered blockId — silent drops
 * would defeat the reducer's audit value (same rule as
 * `agent-pane-state-store`).
 *
 * The `closed` invariant — every command after `Disposed` becomes a
 * no-op that emits `post-close-command-dropped` — is enforced inside
 * the pure reducer. The slot store doesn't need to special-case it.
 */
export function dispatch(
    blockId: string,
    command: BrowserPaneCommand,
    source: CommandSource = "system",
): BrowserPaneEvent[] {
    const slot = slots.get(blockId);
    if (!slot) {
        throw new Error(
            `[browser-pane-state] dispatch for unregistered pane ${blockId.slice(0, 7)} (cmd=${command.type}). registerPane must be called synchronously in the BrowserViewModel constructor.`,
        );
    }
    const prev = slot.state;
    const result = update(prev, command);
    slot.state = result.state;

    // Project changes — only call setters when the cell actually
    // changed. Avoids redundant signal writes that could leak into
    // the address-bar typing path (the PR #737 trap that motivated
    // the cell-by-cell discipline).
    if (slot.state.closed !== prev.closed) slot.proj.closed(slot.state.closed);
    if (slot.state.loading !== prev.loading) slot.proj.loading(slot.state.loading);
    if (slot.state.error !== prev.error) slot.proj.error(slot.state.error);
    if (slot.state.canGoBack !== prev.canGoBack) slot.proj.canGoBack(slot.state.canGoBack);
    if (slot.state.canGoForward !== prev.canGoForward) slot.proj.canGoForward(slot.state.canGoForward);
    if (slot.state.title !== prev.title) slot.proj.title(slot.state.title);
    if (slot.state.url !== prev.url) slot.proj.url(slot.state.url);
    if (slot.state.faviconUrl !== prev.faviconUrl) slot.proj.faviconUrl(slot.state.faviconUrl);

    for (const ev of result.events) eventSink(blockId, ev);

    recordDispatch({
        slice: "browser-pane-state",
        key: blockId,
        command,
        events: result.events,
        source,
        at: Date.now(),
    });

    return result.events;
}

/** Snapshot — diagnostics + tests only. */
export function snapshot(blockId: string): BrowserPaneState | null {
    return slots.get(blockId)?.state ?? null;
}

/** Test/dev helper — clears every slot. */
export function __resetAllSlots(): void {
    slots.clear();
}

export type { BrowserPaneCommand, BrowserPaneEvent, BrowserPaneState };
