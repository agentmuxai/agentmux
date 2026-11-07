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

import { createFillPass } from "./fill-pass";
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
    /**
     * Page in older history because the content is too short to scroll into. Called for a FillOlder
     * effect only (I5), at most FILL_MAX_PAGES times in a row.
     */
    fillOlder?(geo: ScrollGeometry): void;
    /** A fill pass ended short (`true`: it hit FILL_MAX_PAGES with the content still short) or started again (`false`). */
    fillStalled?(stalled: boolean): void;
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
    /**
     * Whether the last observed geometry still needs older history to scroll into, and loads it.
     * For the moments the geometry hasn't changed: a page landed, or history became available.
     */
    checkFill(): void;
    /** Begin a new fill pass (the reader asked for more after one gave up). */
    restartFill(): void;
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
    let lastGestureUp = false;
    let pointerHeld = false;
    let scrollbarHeld = false;
    let pointerInside = false; // the last press in the document was on this transcript: scoped scroll keys go to it
    const fill = createFillPass(host);

    /** Runs the reducer; returns true if it asked for a pin the caller makes itself. */
    const dispatch = (event: FollowEvent, pinViaHost: boolean): boolean => {
        const result = followReducer(state, event);
        state = result.state;
        let pin = false;
        for (const effect of result.effects) {
            if (effect.type === "Transition") host.transition(effect.from, effect.to, effect.cause, effect.detail);
            else if (effect.type === "LoadOlder") host.loadOlder(lastGeo());
            else if (effect.type === "FillOlder") fill.run(lastGeo());
            else if (pinViaHost) host.pin(effect.cause);
            else pin = true;
        }
        return pin;
    };

    const checkFill = (): void => {
        dispatch(fillCheck(lastGeo(), host), true);
    };
    const lastGeo = (): ScrollGeometry => last ?? { scrollTop: 0, scrollHeight: 0, clientHeight: 0 };
    const escape = (cause: EscapeCause): void => {
        dispatch({ type: "UserEscape", cause, geo: lastGeo() }, true);
    };
    /** `up`: the gesture moves the transcript up (`afterUpGesture`). */
    const mark = (up = false): void => {
        lastGestureAt = now();
        lastGestureUp = up;
    };

    const onWheel = (e: WheelEvent): void => {
        // Ctrl/⌘+wheel zooms; a sideways-only wheel (a wide table) scrolls nothing the follow depends on.
        if (e.ctrlKey || e.metaKey || e.deltaY === 0) return;
        mark(e.deltaY < 0);
        const target = e.target instanceof Element ? e.target : null;
        const nested = target?.closest(NESTED_SCROLLER);
        // The scroller may be such a box itself (a tool preview): only one inside it defers.
        if (e.deltaY < 0 && !(nested && nested !== scroller && scroller.contains(nested))) escape("wheel-up");
    };
    const onTouch = (): void => mark();
    const onPointerDown = (e: PointerEvent): void => {
        pointerHeld = true;
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
        const up = UP_KEYS.has(e.key) || (e.key === " " && e.shiftKey);
        mark(up);
        if (up) escape("key");
    };
    const onSelectionChange = (): void => {
        if (!pointerHeld || state.kind !== "FOLLOWING") return;
        const sel = document.getSelection();
        if (!sel || sel.isCollapsed || !sel.anchorNode || !scroller.contains(sel.anchorNode)) return;
        // A selection drag: following would drag the selection away (§5.4).
        escape("selection");
    };

    const removeListeners = listenForGestures(scroller, {
        onWheel,
        onTouch,
        onPointerDown,
        onPointerUp,
        held: () => pointerHeld,
        onDocPointerDown,
        onKey,
        onSelectionChange,
    });

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
            fill.observed(geo);
            const pin = dispatch(resizeEvent(state, kind, geo), false);
            checkFill();
            return pin;
        },
        checkFill,
        restartFill: fill.restart,
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
                    input: inputLabel(user, scrollbarHeld, pointerHeld),
                    afterUpGesture: lastGestureUp && now() - lastGestureAt <= INTENT_WINDOW_MS,
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
            fill.restart();
            dispatch({ type: "Reset" }, true);
        },
        dispose: removeListeners,
    };
}

interface GestureHandlers {
    onWheel(e: WheelEvent): void;
    onTouch(): void;
    onPointerDown(e: PointerEvent): void;
    onPointerUp(): void;
    held(): boolean;
    onDocPointerDown(e: Event): void;
    onKey(e: KeyboardEvent): void;
    onSelectionChange(): void;
}

/** What made a scroll batch the user's, for the transition log. */
function inputLabel(user: boolean, scrollbar: boolean, pointer: boolean): string {
    return user ? (scrollbar ? "scrollbar" : pointer ? "pointer" : "gesture") : "none";
}

/**
 * Listens for the gestures that decide whether the reader has left the bottom; returns the removal.
 * A pointer the page never saw released (the button came up over another window or a webview, or the
 * window lost focus) must not stay held: while it does, every scroll the browser makes counts as the
 * user's. A pointer move with no button down, a blur or the page hiding ends it.
 */
function listenForGestures(scroller: HTMLElement, on: GestureHandlers): () => void {
    const passive: AddEventListenerOptions = { passive: true, capture: true };
    const onPointerMove = (e: PointerEvent): void => {
        if (on.held() && e.buttons === 0) on.onPointerUp();
    };
    // Capture sees the blur of every element too (a press on the transcript blurs the composer): only the window's counts.
    const onWindowBlur = (e: Event): void => {
        if (!(e.target instanceof Element)) on.onPointerUp();
    };
    const onVisibility = (): void => {
        if (document.visibilityState === "hidden") on.onPointerUp();
    };
    scroller.addEventListener("wheel", on.onWheel, passive);
    scroller.addEventListener("touchstart", on.onTouch, passive);
    scroller.addEventListener("touchmove", on.onTouch, passive);
    scroller.addEventListener("pointerdown", on.onPointerDown, passive);
    window.addEventListener("pointerup", on.onPointerUp, passive);
    window.addEventListener("pointercancel", on.onPointerUp, passive);
    window.addEventListener("blur", onWindowBlur, passive);
    document.addEventListener("pointermove", onPointerMove, passive);
    document.addEventListener("visibilitychange", onVisibility, passive);
    document.addEventListener("pointerdown", on.onDocPointerDown, passive);
    document.addEventListener("keydown", on.onKey, passive);
    document.addEventListener("selectionchange", on.onSelectionChange);
    return () => {
        scroller.removeEventListener("wheel", on.onWheel, passive);
        scroller.removeEventListener("touchstart", on.onTouch, passive);
        scroller.removeEventListener("touchmove", on.onTouch, passive);
        scroller.removeEventListener("pointerdown", on.onPointerDown, passive);
        window.removeEventListener("pointerup", on.onPointerUp, passive);
        window.removeEventListener("pointercancel", on.onPointerUp, passive);
        window.removeEventListener("blur", onWindowBlur, passive);
        document.removeEventListener("pointermove", onPointerMove, passive);
        document.removeEventListener("visibilitychange", onVisibility, passive);
        document.removeEventListener("pointerdown", on.onDocPointerDown, passive);
        document.removeEventListener("keydown", on.onKey, passive);
        document.removeEventListener("selectionchange", on.onSelectionChange);
    };
}

/** What a ResizeObserver observation means to a pane in `state`: sized again, or the content or viewport changed. */
function resizeEvent(state: FollowState, kind: "content" | "viewport", geo: ScrollGeometry): FollowEvent {
    if (state.kind === "SUSPENDED") return { type: "BecameSized" };
    return { type: kind === "content" ? "ContentResized" : "ViewportResized", geo };
}

/** The check for older history to fill a short pane with, at `geo`. */
function fillCheck(geo: ScrollGeometry, host: FollowScrollHost): FollowEvent {
    return { type: "FillCheck", geo, hasOlderHistory: host.hasOlderHistory(), loadingOlder: host.loadingOlder() };
}
