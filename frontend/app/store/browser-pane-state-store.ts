// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Browser pane state store — slice #9.
 *
 * **Phase 1A** (`docs/specs/SPEC_BROWSER_PANE_TABS_2026-05-27.md`) — the cell
 * now owns a tab list. Per-page projection setters (`url`, `loading`,
 * `error`, `canGoBack`, `canGoForward`, `title`, `faviconUrl`) project
 * the ACTIVE tab's values; the model code that consumes them via
 * SolidJS signals doesn't have to change. New projections (`tabs`,
 * `activeTabId`) expose the tab-list shape for Phase 1B view wiring.
 *
 * When `activeTabId` changes, ALL active-tab-derived projections
 * re-emit from the new tab — so the view's existing signals reflect
 * the new tab transparently. When a per-tab field of the active tab
 * changes (e.g. `loading: true` after a Navigate), only that setter
 * fires (projection-on-change discipline, preserved from Phase 3).
 *
 * Pattern matches slice #10 (editor-pane) — same slot lifecycle, same
 * `recordDispatch` audit, same "throw on unregistered dispatch" rule.
 */

import { update } from "./browser-pane-state/reducer";
import {
    BrowserPaneCommand,
    BrowserPaneEvent,
    BrowserPaneState,
    BrowserTab,
    initialState,
} from "./browser-pane-state/types";
import { type CommandSource, recordDispatch } from "./command-source";

/**
 * Setters the slot writes into when reducer state changes. The view
 * (`BrowserViewModel`) owns the underlying SolidJS signals and passes
 * just the setters in via `registerPane`. Per-page setters project
 * the ACTIVE tab's values; tab-list setters project the full list /
 * active id for Phase 1B's tab-strip wiring.
 *
 * Each setter is called only when the projected value actually
 * changes (referential equality), preserving the Phase 3 discipline
 * that avoided spurious reactive churn on the address-bar typing
 * path PR #737 regressed.
 */
export interface BrowserPaneProjections {
    /** Pane-level. */
    closed: (next: boolean) => void;
    /** Active tab's `loading`. */
    loading: (next: boolean) => void;
    /** Active tab's `error`. */
    error: (next: string | null) => void;
    /** Active tab's `canGoBack`. */
    canGoBack: (next: boolean) => void;
    /** Active tab's `canGoForward`. */
    canGoForward: (next: boolean) => void;
    /** Active tab's `title`. */
    title: (next: string) => void;
    /** Active tab's `url`. */
    url: (next: string) => void;
    /** Active tab's `faviconUrl`. */
    faviconUrl: (next: string) => void;
    /** Phase 1A additions — tab-list awareness for Phase 1B view. */
    tabs: (next: BrowserTab[]) => void;
    activeTabId: (next: string | null) => void;
}

interface Slot {
    state: BrowserPaneState;
    proj: BrowserPaneProjections;
}

const slots = new Map<string, Slot>();

type EventSink = (blockId: string, event: BrowserPaneEvent) => void;
let eventSink: EventSink = (_blockId, _event) => {
    // Default sink is a no-op. The view layer installs a real sink
    // via `setEventSink` to wire `pane-clicked` (and other future
    // view-effecting events) to the DOM blur + refocus handlers.
    // Tests run with the no-op so the slot store is testable without
    // DOM.
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
 * Resolve the active tab record for a state cell, or null when no
 * tabs are open / activeTabId is null. Exposed only to the projection
 * differ below — tests use `snapshot` instead.
 */
function activeTab(state: BrowserPaneState): BrowserTab | null {
    if (state.activeTabId == null) return null;
    return state.tabs.find((t) => t.id === state.activeTabId) ?? null;
}

/** Defaults projected when no tab is active (pane in initial /
 *  last-tab-closed state). Matches the historical signal initial
 *  values so the view doesn't observe a regression on empty-pane
 *  rendering. */
const EMPTY_TAB_DEFAULTS = {
    url: "",
    title: "Browser",
    faviconUrl: "",
    loading: false,
    error: null as string | null,
    canGoBack: false,
    canGoForward: false,
};

/**
 * Apply a command. Throws on unregistered blockId — silent drops
 * would defeat the reducer's audit value (same rule as slice #4 /
 * slice #10).
 *
 * The `closed` invariant — every command after `Disposed` becomes a
 * no-op that emits `post-close-command-dropped` — is enforced inside
 * the pure reducer.
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

    // ── Pane-level projections ─────────────────────────────────────
    if (slot.state.closed !== prev.closed) slot.proj.closed(slot.state.closed);

    // ── Tab-list projections ──────────────────────────────────────
    if (slot.state.tabs !== prev.tabs) slot.proj.tabs(slot.state.tabs);
    if (slot.state.activeTabId !== prev.activeTabId) {
        slot.proj.activeTabId(slot.state.activeTabId);
    }

    // ── Active-tab projections ─────────────────────────────────────
    // Compare each per-tab field between the prev and next active
    // tab. Two cases:
    //
    //   1. `activeTabId` changed → re-project EVERY field that
    //      differs between the old active tab and the new active
    //      tab. The view's signals must reflect the new tab's values
    //      without the model code knowing about tabs at all.
    //
    //   2. `activeTabId` unchanged → re-project only the per-tab
    //      fields that changed on the (still-) active tab.
    //
    // The defaults (`EMPTY_TAB_DEFAULTS`) cover the
    // null→tab and tab→null transitions (initial OpenTab; last-tab
    // close) so the projections always have a well-defined value.
    const prevActive = activeTab(prev);
    const nextActive = activeTab(slot.state);

    const prevView = prevActive ?? EMPTY_TAB_DEFAULTS;
    const nextView = nextActive ?? EMPTY_TAB_DEFAULTS;

    if (nextView.url !== prevView.url) slot.proj.url(nextView.url);
    if (nextView.title !== prevView.title) slot.proj.title(nextView.title);
    if (nextView.faviconUrl !== prevView.faviconUrl) slot.proj.faviconUrl(nextView.faviconUrl);
    if (nextView.loading !== prevView.loading) slot.proj.loading(nextView.loading);
    if (nextView.error !== prevView.error) slot.proj.error(nextView.error);
    if (nextView.canGoBack !== prevView.canGoBack) slot.proj.canGoBack(nextView.canGoBack);
    if (nextView.canGoForward !== prevView.canGoForward) slot.proj.canGoForward(nextView.canGoForward);

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
