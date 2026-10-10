// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What is being dragged in this renderer: one session, from the drag's start
 * to its end. docs/specs/SPEC_DRAG_AND_DROP_CONSOLIDATION_2026_09_27.md §5.1.
 *
 * Who may end a session follows today's ordering:
 * - an accepting drop target in this window ends it;
 * - the SOURCE's onDrop only marks it `released` (pragmatic calls it even for
 *   a release outside every target), so the cross-window monitor still sees it;
 * - the cross-window monitor ends a released session once it has handled the
 *   document dragend;
 * - the safety net ends whatever is left (installSafetyNet below).
 * Internal sessions have no inactivity timeout: a drag held over another
 * window sends this renderer no events, and that is normal.
 */

import { createSignal } from "solid-js";

export type DragKind = "files" | "tile" | "window-tab" | "pane-tab" | "doc-tab" | "drone-kind" | "list-item";
/** "unobserved": the safety net saw a pointerdown with the drag still open. */
export type DragEndReason = "drop" | "cancel" | "dragend" | "button-up" | "files-idle" | "unobserved";

export interface DragSource {
    nodeId?: string;
    tabId?: string;
    blockId?: string;
    wsId?: string;
    /** list-item: the dragged entry. doc-tab: the dragged document tab. */
    itemId?: string;
}

/** Kind-specific data captured at drag start that can't be re-derived later. */
export interface DragPayload {
    /** pane-tab: the visible pane's size at start; a background pill has no element to measure at tear-off. */
    paneSize?: { width: number; height: number };
    /** window-tab: false for a lone-tab drag, which only the native strip merge may handle. */
    crossWindow?: boolean;
    /** drone-kind: the node kind a top-bar chip creates. */
    droneKind?: string;
}

export interface DragSession {
    kind: DragKind;
    /** Minted at start, so host events for one drag can be de-duplicated. */
    dragId: string;
    source?: DragSource;
    payload?: DragPayload;
    escaped: boolean;
    released: boolean;
    startedAt: number;
}

export interface SessionEnd {
    session: DragSession;
    reason: DragEndReason;
}

const [current, setCurrent] = createSignal<DragSession | null>(null);
const listeners = new Set<(end: SessionEnd) => void>();

/** The session in progress, or null. Reactive. */
export const session = current;

function mintId(): string {
    const c = globalThis.crypto as Crypto | undefined;
    if (c?.randomUUID) return c.randomUUID();
    return `${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`;
}

export function beginDrag(kind: DragKind, source?: DragSource, payload?: DragPayload): DragSession {
    installSafetyNet();
    // A new drag means any session still here was stranded (its end was lost).
    endDrag("cancel");
    const s: DragSession = {
        kind,
        dragId: mintId(),
        source,
        payload,
        escaped: false,
        released: false,
        startedAt: Date.now(),
    };
    setCurrent(s);
    return s;
}

function update(patch: Partial<DragSession>): void {
    const s = current();
    if (s) setCurrent({ ...s, ...patch });
}

/** The source's onDrop: the drag is over in this window, not necessarily handled. */
export const markReleased = () => update({ released: true });

export const markEscaped = () => update({ escaped: true });

/** Any of `kinds` has started and its source hasn't released it yet. Reactive. */
export function isAnyUnderway(kinds: readonly DragKind[]): boolean {
    const s = current();
    return s != null && kinds.includes(s.kind) && !s.released;
}

/** A `kind` drag has started and its source hasn't released it yet. */
export function isUnderway(kind: DragKind): boolean {
    const s = current();
    return s?.kind === kind && !s.released;
}

/**
 * End the current session. With `dragId`, only if it is still that drag, so a
 * late end from an old drag can't cut short the next one.
 */
export function endDrag(reason: DragEndReason, dragId?: string): void {
    const s = current();
    if (!s || (dragId !== undefined && s.dragId !== dragId)) return;
    setCurrent(null);
    for (const listener of listeners) listener({ session: s, reason });
}

/**
 * The cross-window monitor's document dragend: ends the session if its source
 * has released it, and returns the session as it was, so the caller can still
 * read `escaped`. The tab bar ends tile and window-tab sessions before this
 * runs; what is left is a pane-tab session, or any drag in a window without a
 * tab bar.
 */
export function endReleasedSession(reason: DragEndReason): DragSession | null {
    const s = current();
    if (s?.released) endDrag(reason, s.dragId);
    return s;
}

export function onSessionEnded(listener: (end: SessionEnd) => void): () => void {
    listeners.add(listener);
    return () => listeners.delete(listener);
}

let netInstalled = false;

/**
 * The one safety net (§5.1): ends a drag whose end this window never saw.
 * It goes by real events, never inactivity:
 * - a window `dragend` in the bubble phase, so after pragmatic (window,
 *   capture), the cross-window monitors (document) and each element's own
 *   handler;
 * - a `pointerdown`, which can't happen mid-drag (the button is held), so a
 *   drag still open then ended unobserved.
 * On Windows, CrossWindowDragMonitor.win32's button poll also ends a drag
 * whose dragend was swallowed. "files" sessions are the file-drop
 * controller's, with its own idle watchdog. Each kind's cleanup subscribes
 * with onSessionEnded. Installed on the first drag, so importing this module
 * has no side effects.
 */
function installSafetyNet(): void {
    if (netInstalled || typeof window === "undefined") return;
    netInstalled = true;
    const endStranded = (reason: DragEndReason) => {
        const s = current();
        if (s && s.kind !== "files") endDrag(reason, s.dragId);
    };
    window.addEventListener("dragend", () => endStranded("dragend"));
    window.addEventListener("pointerdown", () => endStranded("unobserved"), true);
}
