// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// SPEC_NATIVE_POINTER_DRAG_TEAROFF_2026_07_28 — shared pointer-capture drag
// state machine for tab (and, eventually, pane) tear-off on Windows.
//
// Replaces pragmatic-drag-and-drop's HTML5 `draggable()` — which drives a
// real OLE DoDragDrop session on Windows, the root cause of the un-fixable
// circle-slash cursor (Chromium's own IDropSource::GiveFeedback overrides
// any JS- or host-polling-thread cursor fix faster than it can repaint) —
// with raw Pointer Events + setPointerCapture. Pointer capture keeps
// delivering pointermove/pointerup to this element (with real screenX/screenY,
// even once the cursor has left the window entirely) without ever starting
// an OS-level drag negotiation, so there is no GiveFeedback to fight.
//
// This module owns gesture DETECTION (click vs. reorder vs. tear-off) and
// event plumbing only. All the actual reorder/tear-off mechanics (insertion-
// point math, RPC calls, window positioning) live in the caller's handlers —
// see droppable-tab.tsx for the tab wiring.

const CLICK_THRESHOLD_PX = 4;

export interface DragTrackerHandlers {
    /** Pointer went down and up again without crossing the click threshold. */
    onClick: () => void;
    /** Called exactly once, when movement first crosses the click threshold
     *  (i.e. this is now a real drag, not a click) — before the very first
     *  onReorderUpdate/onTearOffStart call. Mirrors pragmatic-dnd's
     *  onGenerateDragPreview/onDragStart timing for one-time drag setup
     *  (grab-offset capture, arming the cross-window hover hook, etc.).
     *  Cursor position is viewport-relative (the drag hasn't left the
     *  window yet at this point). */
    onDragStart: (cursorX: number, cursorY: number) => void;
    /** Reorder mode: cursor still within the source container. Viewport-
     *  relative (clientX/Y) — reorder math only ever needs in-window position. */
    onReorderUpdate: (cursorX: number, cursorY: number) => void;
    /** Reorder mode, pointerup: commit whatever onReorderUpdate last computed. */
    onReorderCommit: (cursorX: number, cursorY: number) => void;
    /** Reorder mode ended without a commit (Escape / pointercancel). */
    onReorderCancel: () => void;
    /** Cursor crossed the tear-off threshold (still in reorder mode up to
     *  this call). Screen coordinates — valid immediately, unlike clientX/Y,
     *  which become meaningless once the drag leaves the window. The
     *  returned label is the newly-created window now being followed; a
     *  resolved `undefined` means tear-off failed and the tracker falls
     *  back to reorder-cancel semantics. */
    onTearOffStart: (screenX: number, screenY: number) => Promise<string | undefined>;
    /** Tear-off mode, subsequent pointermove — screen coordinates, already
     *  rAF-throttled by the tracker (one in-flight update at a time). */
    onTearOffMove: (screenX: number, screenY: number) => void;
    /** Tear-off mode ended. `committed` is true for a real pointerup
     *  (caller decides standalone vs. cross-window merge — that decision is
     *  driven by the existing WH_MOUSE_LL hook's hover state, not by this
     *  tracker); false for an aborted gesture (Escape / pointercancel), in
     *  which case the caller should cancel-back (close the torn-off window,
     *  restore original position). */
    onTearOffEnd: (committed: boolean) => void;
    /** Called on every pointermove while still in reorder mode, to decide
     *  whether the gesture has now crossed into tear-off territory. */
    isTearOffZone: (cursorX: number, cursorY: number) => boolean;
}

type TrackerState =
    | { kind: "idle" }
    | { kind: "tracking"; pointerId: number; startX: number; startY: number }
    | { kind: "reorder"; pointerId: number }
    | { kind: "tearoff"; pointerId: number; label: string | null };

/**
 * Attaches the pointer-capture drag state machine to `el`. Returns a cleanup
 * function. `canDrag` is re-checked on every pointerdown (matches
 * pragmatic-dnd's draggable() `canDrag` semantics — e.g. "more than one tab,
 * or this isn't the main window").
 */
export function attachNativePointerDragTracker(
    el: HTMLElement,
    handlers: DragTrackerHandlers,
    // Cursor position (viewport-relative) at the pointerdown that's being
    // considered — panes use this to reject drags starting in a resize-handle
    // rejection zone (see TileLayout.win32.tsx); tabs ignore it.
    canDrag: (cursorX: number, cursorY: number) => boolean,
): () => void {
    let state: TrackerState = { kind: "idle" };
    let rafHandle: number | null = null;
    // Raw pointermove can fire far more often than the display refresh rate
    // (full HID polling rate — 125Hz-1000Hz depending on the mouse), much
    // higher than the dragover events this replaced (which Chromium's HTML5
    // DnD pipeline coalesces to roughly frame rate internally). Both the
    // reorder path (computeInsertionPoint does an O(tabs) getBoundingClientRect
    // scan — forced synchronous layout) and the tear-off path (SetWindowPos
    // IPC round trip) are too expensive to run unthrottled at HID rate, so
    // BOTH coalesce through one rAF-gated "latest event wins" slot.
    let pendingMove: { clientX: number; clientY: number; screenX: number; screenY: number } | null = null;

    const flushMove = () => {
        rafHandle = null;
        const pending = pendingMove;
        pendingMove = null;
        if (!pending) return;
        if (state.kind === "reorder") {
            if (handlers.isTearOffZone(pending.clientX, pending.clientY)) {
                beginTearOff(state.pointerId, pending.screenX, pending.screenY);
            } else {
                handlers.onReorderUpdate(pending.clientX, pending.clientY);
            }
        } else if (state.kind === "tearoff" && state.label != null) {
            handlers.onTearOffMove(pending.screenX, pending.screenY);
        }
    };

    const clearRaf = () => {
        if (rafHandle != null) {
            cancelAnimationFrame(rafHandle);
            rafHandle = null;
        }
        pendingMove = null;
    };

    const beginTearOff = (pointerId: number, screenX: number, screenY: number) => {
        state = { kind: "tearoff", pointerId, label: null };
        handlers.onTearOffStart(screenX, screenY).then((label) => {
            // The gesture may have already ended (pointerup/cancel/Escape)
            // by the time this async call resolves — only adopt the result
            // if we're still the active tear-off for this exact pointer.
            if (state.kind !== "tearoff" || state.pointerId !== pointerId) return;
            if (label) {
                state = { kind: "tearoff", pointerId, label };
            } else {
                state = { kind: "idle" };
                clearRaf();
                handlers.onReorderCancel();
            }
        });
    };

    const onPointerDown = (e: PointerEvent) => {
        if (e.button !== 0) return;
        if (state.kind !== "idle") return;
        if (!canDrag(e.clientX, e.clientY)) return;
        // Elements that opt out of dragging (close button, tab name while
        // being renamed) mark themselves draggable="false" — mirrors how
        // native HTML5 drag already respects that attribute on a descendant
        // even when an ancestor is draggable="true".
        if ((e.target as HTMLElement | null)?.closest('[draggable="false"]')) return;
        state = { kind: "tracking", pointerId: e.pointerId, startX: e.clientX, startY: e.clientY };
    };

    const onPointerMove = (e: PointerEvent) => {
        if (state.kind === "idle") return;
        if (e.pointerId !== state.pointerId) return;

        if (state.kind === "tracking") {
            const dx = e.clientX - state.startX;
            const dy = e.clientY - state.startY;
            if (Math.hypot(dx, dy) < CLICK_THRESHOLD_PX) return;
            // Real drag confirmed — take pointer capture now (not at
            // pointerdown) so a plain click never touches capture/preventDefault
            // at all, and stop the default browser drag/text-selection this
            // movement would otherwise start.
            e.preventDefault();
            el.setPointerCapture(state.pointerId);
            state = { kind: "reorder", pointerId: state.pointerId };
            handlers.onDragStart(e.clientX, e.clientY);
        }

        if (state.kind === "reorder" || state.kind === "tearoff") {
            pendingMove = { clientX: e.clientX, clientY: e.clientY, screenX: e.screenX, screenY: e.screenY };
            if (rafHandle == null) rafHandle = requestAnimationFrame(flushMove);
        }
    };

    const endGesture = (
        pointerId: number,
        outcome: "up" | "cancel",
        clientX: number,
        clientY: number,
    ) => {
        const prev = state;
        if (prev.kind === "idle" || prev.pointerId !== pointerId) return;
        state = { kind: "idle" };
        clearRaf();
        if (el.hasPointerCapture(pointerId)) el.releasePointerCapture(pointerId);

        if (prev.kind === "tracking") {
            if (outcome === "up") handlers.onClick();
            return;
        }
        if (prev.kind === "reorder") {
            if (outcome === "up") handlers.onReorderCommit(clientX, clientY);
            else handlers.onReorderCancel();
            return;
        }
        if (prev.kind === "tearoff") {
            // Only report a real end if onTearOffStart had actually landed
            // (adopted a label) — if it's still in flight, its own .then()
            // already sees state reset to idle above and no-ops.
            if (prev.label != null) handlers.onTearOffEnd(outcome === "up");
        }
    };

    const onPointerUp = (e: PointerEvent) => {
        endGesture(e.pointerId, "up", e.clientX, e.clientY);
    };
    const onPointerCancel = (e: PointerEvent) => {
        endGesture(e.pointerId, "cancel", e.clientX, e.clientY);
    };
    const onKeyDown = (e: KeyboardEvent) => {
        if (e.key !== "Escape") return;
        if (state.kind !== "reorder" && state.kind !== "tearoff") return;
        endGesture(state.pointerId, "cancel", 0, 0);
    };

    el.addEventListener("pointerdown", onPointerDown);
    el.addEventListener("pointermove", onPointerMove);
    el.addEventListener("pointerup", onPointerUp);
    el.addEventListener("pointercancel", onPointerCancel);
    window.addEventListener("keydown", onKeyDown, true);

    return () => {
        el.removeEventListener("pointerdown", onPointerDown);
        el.removeEventListener("pointermove", onPointerMove);
        el.removeEventListener("pointerup", onPointerUp);
        el.removeEventListener("pointercancel", onPointerCancel);
        window.removeEventListener("keydown", onKeyDown, true);
        clearRaf();
    };
}
