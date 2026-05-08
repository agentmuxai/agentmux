// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Browser pane state store — slice #9 of the frontend reducer roadmap.
 * Bundles the per-pane URL / title / favicon / loading / history / error
 * state. See docs/specs/browser-pane-reducer.md.
 *
 * Pattern matches agent-pane-state-store.ts: per-blockId slot, atoms as
 * write-only projections, throw on unregistered dispatch. Conventions
 * §4–§5 (frontend-reducer-conventions-2026-05-03.md).
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
 * Per-pane projection setters. Each one corresponds to a Solid signal
 * the BrowserViewModel exposes (urlAtom, titleAtom, etc.). Readers keep
 * using those accessors; writes are routed through the slice and only
 * fire when the field actually changed (referential equality).
 */
export interface BrowserPaneProjections {
    url: (next: string) => void;
    title: (next: string) => void;
    faviconUrl: (next: string) => void;
    loading: (next: boolean) => void;
    canGoBack: (next: boolean) => void;
    canGoForward: (next: boolean) => void;
    error: (next: string | null) => void;
    closed: (next: boolean) => void;
}

interface Slot {
    state: BrowserPaneState;
    proj: BrowserPaneProjections;
}

const slots = new Map<string, Slot>();

type EventSink = (blockId: string, event: BrowserPaneEvent) => void;

/**
 * The default sink is a no-op for non-diagnostic events. The model
 * processes events locally (IPC fan-out) by reading them from the
 * dispatch return value.
 */
let eventSink: EventSink = () => {};

export function setEventSink(sink: EventSink): void {
    eventSink = sink;
}

/**
 * Register a pane. Call SYNCHRONOUSLY from the BrowserViewModel
 * constructor before any IPC subscription can dispatch. Re-registering
 * a blockId resets the state cell to initialState (useful for
 * hot-reload).
 */
export function registerPane(blockId: string, proj: BrowserPaneProjections): void {
    slots.set(blockId, { state: initialState(blockId), proj });
}

export function unregisterPane(blockId: string): void {
    slots.delete(blockId);
}

/**
 * Apply a command. Throws on unregistered blockId — silent drops would
 * defeat the point of the reducer (same rule as agent-document /
 * agent-pane-state stores).
 */
export function dispatch(
    blockId: string,
    command: BrowserPaneCommand,
    source: CommandSource = "system",
): BrowserPaneEvent[] {
    const slot = slots.get(blockId);
    if (!slot) {
        throw new Error(
            `[browser-pane-state] dispatch for unregistered pane ${blockId.slice(0, 7)} (cmd=${command.type}). registerPane must be called synchronously in the component body.`,
        );
    }
    const prev = slot.state;
    const result = update(prev, command);
    slot.state = result.state;

    // Project changes — only call setters for fields that actually
    // changed (referential equality). Avoids redundant signal writes.
    if (slot.state.url !== prev.url) slot.proj.url(slot.state.url);
    if (slot.state.title !== prev.title) slot.proj.title(slot.state.title);
    if (slot.state.faviconUrl !== prev.faviconUrl) slot.proj.faviconUrl(slot.state.faviconUrl);
    if (slot.state.loading !== prev.loading) slot.proj.loading(slot.state.loading);
    if (slot.state.canGoBack !== prev.canGoBack) slot.proj.canGoBack(slot.state.canGoBack);
    if (slot.state.canGoForward !== prev.canGoForward) slot.proj.canGoForward(slot.state.canGoForward);
    if (slot.state.error !== prev.error) slot.proj.error(slot.state.error);
    if (slot.state.closed !== prev.closed) slot.proj.closed(slot.state.closed);

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

/** Test/dev helper. */
export function __resetAllSlots(): void {
    slots.clear();
}

export type { BrowserPaneCommand, BrowserPaneEvent, BrowserPaneState };
