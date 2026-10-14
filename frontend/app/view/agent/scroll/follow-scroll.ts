// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The DOM binding for the follow state machine (follow-controller.ts): turns
 * gestures, scroll batches and ResizeObserver observations of one scroller
 * into reducer events, and runs the effects through its host.
 * SPEC_AGENT_PANE_SCROLL_FOLLOW_STATE_MACHINE_2026_09_24.md §5.1, §5.4.
 *
 * It owns the gesture listeners and the user-intent window. It does not own
 * the ResizeObservers or the pin itself: the host's pin path (the transcript's
 * content/viewport observers, or OneWayFlow) keeps those and reports each
 * observation to `observed()`, so there is still one writer of the bottom
 * scrollTop per path (I3).
 */

import {
    FOLLOWING,
    followReducer,
    intentOf,
    type EscapeCause,
    type FollowEvent,
    type FollowState,
    type ScrollGeometry,
} from "./follow-controller";
import type { ScrollInput } from "./follow-wiring";

/**
 * How long after the last gesture event a scroll batch still counts as the
 * user's (§5.4). A timestamp, not a flag: a click that scrolled nothing can't
 * leave it set for the next pin minutes later (§2.2 B1).
 */
export const INTENT_WINDOW_MS = 250;

const SCROLL_KEYS = new Set(["PageUp", "PageDown", "Home", "End", "ArrowUp", "ArrowDown", " "]);
/** Keys that move the transcript up, so they escape at once. */
const UP_KEYS = new Set(["PageUp", "Home", "ArrowUp"]);

/**
 * Nested boxes that relay the wheel to the pane by hand (scroll-handoff.ts).
 * A wheel-up over one may be used by the box, so it can't escape at once;
 * the scroll it relays (if any) is classified like any other user scroll.
 */
const NESTED_SCROLLER = ".scroll-handoff-box";

export interface FollowScrollHost {
    /** Write scrollTop to the true bottom: a jump, or following again. */
    pin(cause: string): void;
    /** Page in older history from the user's scroll at `geo`. Only ever called for a LoadOlder effect (I2). */
    loadOlder(geo: ScrollGeometry): void;
    /** Every state change, after it is applied (I4). */
    transition(from: FollowState, to: FollowState, cause: string, detail?: string): void;
    hasOlderHistory(): boolean;
    loadingOlder(): boolean;
    /** A gesture started: stop any motion that would fight it. */
    gesture?(): void;
    /** The user let go of a pointer held on the transcript. */
    released?(): void;
    now?(): number;
}

/** `windowOpen()` is the user-intent window above (or a held scrollbar). */
export interface FollowScroll extends ScrollInput {
    state(): FollowState;
    /** The follow intent (FOLLOWING, or SUSPENDED from FOLLOWING). */
    following(): boolean;
    /**
     * A ResizeObserver observation, layout clean. Returns true when the caller
     * should pin now (it is the pin's only caller for resizes).
     */
    observed(kind: "content" | "viewport", geo: ScrollGeometry): boolean;
    /** Geometry after one of our own scrollTop writes. */
    noteGeometry(geo: ScrollGeometry): void;
    /** A scroll batch. `user` comes from `windowOpen()` / `pointerHeld()` when the batch ran. */
    scrolled(geo: ScrollGeometry, user: boolean): void;
    jumpToBottom(reason: string): void;
    jumpToNode(nodeId: string): void;
    reset(): void;
    dispose(): void;
}

export function createFollowScroll(
    scroller: HTMLElement,
    host: FollowScrollHost,
    initial: FollowState = FOLLOWING
): FollowScroll {
    const now = host.now ?? (() => performance.now());
    let state = initial;
    let last: ScrollGeometry | null = null;
    let lastGestureAt = Number.NEGATIVE_INFINITY;
    let pointerHeld = false;
    let scrollbarHeld = false;
    /** The last press in the document was on this transcript: scoped scroll keys go to it. */
    let pointerInside = false;

    /** Runs the reducer; returns true if it asked for a pin the caller makes itself. */
    const dispatch = (event: FollowEvent, pinViaHost: boolean): boolean => {
        const result = followReducer(state, event);
        state = result.state;
        let pin = false;
        for (const effect of result.effects) {
            if (effect.type === "Transition") host.transition(effect.from, effect.to, effect.cause, effect.detail);
            else if (effect.type === "LoadOlder") host.loadOlder(lastGeo());
            else if (pinViaHost) host.pin(effect.cause);
            else pin = true;
        }
        return pin;
    };

    const lastGeo = (): ScrollGeometry => last ?? { scrollTop: 0, scrollHeight: 0, clientHeight: 0 };
    const escape = (cause: EscapeCause): void => {
        dispatch({ type: "UserEscape", cause, geo: lastGeo() }, true);
    };
    const mark = (): void => {
        lastGestureAt = now();
        host.gesture?.();
    };

    const onWheel = (e: WheelEvent): void => {
        // Ctrl/⌘+wheel zooms the pane; it never scrolls.
        if (e.ctrlKey || e.metaKey) return;
        mark();
        const target = e.target instanceof Element ? e.target : null;
        const nested = target?.closest(NESTED_SCROLLER);
        if (e.deltaY < 0 && !(nested && scroller.contains(nested))) escape("wheel-up");
    };
    const onTouch = (): void => mark();
    const onPointerDown = (e: PointerEvent): void => {
        pointerHeld = true;
        host.gesture?.();
        // Only a press on the scroller itself (its scrollbar) can scroll it;
        // a press on content (focus, a tool header, a link) can't (§2.2 B1).
        if (e.target !== scroller) return;
        scrollbarHeld = true;
        mark();
    };
    const onPointerUp = (): void => {
        const was = pointerHeld;
        pointerHeld = false;
        if (scrollbarHeld) {
            scrollbarHeld = false;
            mark(); // the drag's trailing scroll events are still the user's
        }
        if (was) host.released?.();
    };
    const onDocPointerDown = (e: Event): void => {
        pointerInside = e.target instanceof Node && scroller.contains(e.target);
    };
    const onKey = (e: KeyboardEvent): void => {
        if (!SCROLL_KEYS.has(e.key)) return;
        const t = e.target instanceof HTMLElement ? e.target : null;
        if (t && (t.isContentEditable || t.tagName === "TEXTAREA" || t.tagName === "INPUT" || t.tagName === "SELECT"))
            return;
        // Only keys that scroll THIS transcript: focus inside it, or nothing
        // focused and the last press was on it (Chromium scrolls that one).
        const unfocused = !t || t === document.body || t === document.documentElement;
        if (!(t && scroller.contains(t)) && !(unfocused && pointerInside)) return;
        mark();
        if (UP_KEYS.has(e.key) || (e.key === " " && e.shiftKey)) escape("key");
    };
    const onSelectionChange = (): void => {
        if (!pointerHeld || state.kind !== "FOLLOWING") return;
        const sel = document.getSelection();
        if (!sel || sel.isCollapsed || !sel.anchorNode || !scroller.contains(sel.anchorNode)) return;
        // A selection drag: following would drag the selection away (§5.4).
        escape("selection");
    };

    const passive: AddEventListenerOptions = { passive: true, capture: true };
    scroller.addEventListener("wheel", onWheel, passive);
    scroller.addEventListener("touchstart", onTouch, passive);
    scroller.addEventListener("touchmove", onTouch, passive);
    scroller.addEventListener("pointerdown", onPointerDown, passive);
    window.addEventListener("pointerup", onPointerUp, passive);
    window.addEventListener("pointercancel", onPointerUp, passive);
    document.addEventListener("pointerdown", onDocPointerDown, passive);
    document.addEventListener("keydown", onKey, passive);
    document.addEventListener("selectionchange", onSelectionChange);

    return {
        state: () => state,
        following: () => intentOf(state) === "FOLLOWING",
        windowOpen: () => scrollbarHeld || now() - lastGestureAt <= INTENT_WINDOW_MS,
        pointerHeld: () => pointerHeld,
        scrollbarHeld: () => scrollbarHeld,
        observed(kind, geo) {
            if (geo.clientHeight <= 0) {
                dispatch({ type: "BecameZeroSize" }, false);
                return false;
            }
            last = geo;
            if (state.kind === "SUSPENDED") return dispatch({ type: "BecameSized" }, false);
            return dispatch({ type: kind === "content" ? "ContentResized" : "ViewportResized", geo }, false);
        },
        noteGeometry(geo) {
            last = geo;
        },
        scrolled(geo, user) {
            const prev = last ?? geo;
            last = geo;
            dispatch(
                {
                    type: "Scroll",
                    user,
                    prev,
                    geo,
                    scrollbar: scrollbarHeld,
                    hasOlderHistory: host.hasOlderHistory(),
                    loadingOlder: host.loadingOlder(),
                },
                true
            );
        },
        jumpToBottom(reason) {
            dispatch({ type: "JumpToBottom", reason }, true);
        },
        jumpToNode(nodeId) {
            dispatch({ type: "JumpToNode", nodeId }, true);
        },
        reset() {
            dispatch({ type: "Reset" }, true);
        },
        dispose() {
            scroller.removeEventListener("wheel", onWheel, passive);
            scroller.removeEventListener("touchstart", onTouch, passive);
            scroller.removeEventListener("touchmove", onTouch, passive);
            scroller.removeEventListener("pointerdown", onPointerDown, passive);
            window.removeEventListener("pointerup", onPointerUp, passive);
            window.removeEventListener("pointercancel", onPointerUp, passive);
            document.removeEventListener("pointerdown", onDocPointerDown, passive);
            document.removeEventListener("keydown", onKey, passive);
            document.removeEventListener("selectionchange", onSelectionChange);
        },
    };
}
