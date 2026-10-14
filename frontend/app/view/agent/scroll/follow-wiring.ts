// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The transcript's follow plumbing that doesn't depend on its layout: the
 * `[scroll-follow]` transition log (I4) and the user scroll input of the path
 * without the follow controller. Used by
 * virtualization/AgentDocumentVirtualList.tsx.
 * SPEC_AGENT_PANE_SCROLL_FOLLOW_STATE_MACHINE_2026_09_24.md.
 */

import { createEffect, untrack, type Accessor } from "solid-js";

import { followStateLabel, intentOf, type FollowState } from "./follow-controller";

/** What the transcript asks about the user's scroll input, on either path. */
export interface ScrollInput {
    /** Inside the user-input window, or holding the scrollbar. */
    windowOpen(): boolean;
    /** A pointer is held anywhere on the transcript (a selection or scrollbar drag). */
    pointerHeld(): boolean;
    scrollbarHeld(): boolean;
}

/** Before the scroller mounts: no input yet. */
export const NO_SCROLL_INPUT: ScrollInput = {
    windowOpen: () => false,
    pointerHeld: () => false,
    scrollbarHeld: () => false,
};

/**
 * How long after the last user scroll input (wheel, touch, scroll key,
 * scrollbar release) a scroll batch still counts as the user's. Long enough
 * for a wheel/keyboard gesture's scroll events to land (they are rAF-
 * coalesced), short enough that an unrelated later scroll never inherits it.
 */
const USER_INPUT_WINDOW_MS = 250;

/**
 * User scroll input without the follow controller (follow-scroll.ts has its
 * own). Returns the input and its cleanup.
 *
 * User scroll intent is a short WINDOW, not a flag. Opened by any input
 * that can scroll this container (wheel, touch, scroll keys, or a pointer
 * held on the scroller itself, i.e. its scrollbar) and closed
 * USER_INPUT_WINDOW_MS after the last such event. A batch inside the
 * window reads live geometry, is never attributed to the pin, and is the
 * ONLY kind of batch that can disengage stick-to-bottom.
 *
 * This used to be a boolean consumed by the next scroll batch, however
 * late it came. A click that scrolled nothing (focusing the pane,
 * selecting text, expanding a tool) left it set indefinitely, so the next
 * pin — often minutes later, with new content already landed in the same
 * frame — read as the user scrolling away and silently stopped the
 * follow. Phase 0 of
 * docs/specs/SPEC_AGENT_PANE_SCROLL_FOLLOW_STATE_MACHINE_2026_09_24.md §2.2 (B1).
 */
export function attachLegacyScrollInput(
    el: HTMLElement,
    hooks: { gesture(): void; released(): void }
): ScrollInput & { dispose(): void } {
    let lastUserScrollInputAt = Number.NEGATIVE_INFINITY;
    let scrollbarPointerHeld = false;
    // A pointer held anywhere on the transcript (a selection drag, a scrollbar
    // drag): the one-way path neither compensates nor follows under it.
    let pointerHeld = false;
    // Record user scroll input (see lastUserScrollInputAt). On the scroller:
    // wheel and touch. A pointer counts only when pressed on the scroller
    // element ITSELF — the scrollbar track/thumb (content clicks target a
    // child row, and cannot scroll this container) — and keeps the window
    // open for as long as it is held, so a slow scrollbar drag stays the
    // user's. On the document: the scrolling keys, unless typed into an
    // editable element (typing in the composer never scrolls this container).
    // Target identity, not a clientWidth hit-test: no layout read in an input
    // handler (tools/lint/check-input-handler-layout-reads.sh).
    // The user's own scroll, or a press to select or click, ends any glide
    // at once, so what's under the pointer is where it's drawn.
    const mark = (): void => {
        lastUserScrollInputAt = performance.now();
        hooks.gesture();
    };
    const onPointerDown = (e: PointerEvent): void => {
        hooks.gesture();
        pointerHeld = true;
        if (e.target !== el) return;
        scrollbarPointerHeld = true;
        mark();
    };
    const onPointerUp = (): void => {
        // Let go of the pane: look again shortly (a timer, not a layout
        // read in this handler) in case content arrived meanwhile.
        if (pointerHeld) hooks.released();
        pointerHeld = false;
        if (!scrollbarPointerHeld) return;
        scrollbarPointerHeld = false;
        mark(); // the drag's trailing scroll events are still the user's
    };
    const SCROLL_KEYS = new Set(["PageUp", "PageDown", "Home", "End", "ArrowUp", "ArrowDown", " "]);
    const onKey = (e: KeyboardEvent): void => {
        const t = e.target as HTMLElement | null;
        if (t && (t.isContentEditable || t.tagName === "TEXTAREA" || t.tagName === "INPUT")) return;
        if (SCROLL_KEYS.has(e.key)) mark();
    };
    const opts: AddEventListenerOptions = { passive: true, capture: true };
    const types = ["wheel", "touchstart", "touchmove"] as const;
    for (const type of types) el.addEventListener(type, mark, opts);
    el.addEventListener("pointerdown", onPointerDown, opts);
    window.addEventListener("pointerup", onPointerUp, opts);
    window.addEventListener("pointercancel", onPointerUp, opts);
    document.addEventListener("keydown", onKey, opts);
    return {
        windowOpen: () => scrollbarPointerHeld || performance.now() - lastUserScrollInputAt <= USER_INPUT_WINDOW_MS,
        pointerHeld: () => pointerHeld,
        scrollbarHeld: () => scrollbarPointerHeld,
        dispose() {
            for (const type of types) el.removeEventListener(type, mark, opts);
            el.removeEventListener("pointerdown", onPointerDown, opts);
            window.removeEventListener("pointerup", onPointerUp, opts);
            window.removeEventListener("pointercancel", onPointerUp, opts);
            document.removeEventListener("keydown", onKey, opts);
        },
    };
}

export interface FollowLog {
    /** Publish `cause`, then apply a stickToBottom change; logs it once. */
    transitionFollow(cause: string, detail: string | undefined, apply: () => void): void;
    /** Project a follow-controller transition onto stickToBottom (and log it). */
    project(from: FollowState, to: FollowState, cause: string, detail?: string): void;
}

/**
 * ── Follow-state transition log (spec invariant I4) ──────────────────
 * Every stickToBottom change emits exactly one line:
 *   [scroll-follow] pane=abc1234 following→detached cause=user-scroll gap=412px
 * so a pane that stopped following always says why in
 * `muxlog fe grep scroll-follow`. Bug A (spec §2.1) was invisible for
 * months precisely because its disengage path logged nothing.
 *
 * Call sites go through transitionFollow(), which publishes its cause
 * BEFORE applying the change. Solid may run the effect below either
 * synchronously inside apply() (a setter called outside an update) or
 * later (a setter called from inside another effect); whichever of the
 * two observes the change first logs it, with that cause, exactly once —
 * `lastLoggedFollow` is the dedupe. A change that did NOT come through
 * transitionFollow() has no published cause and logs as cause=external,
 * so no path can be silent. `data-follow-state` on the scroller mirrors
 * the state for CDP / tests. Call during component setup (it creates an effect).
 */
export function createFollowLog(
    paneTag: () => string,
    viewState: { stickToBottom: Accessor<boolean>; engageStickToBottom(): void; disengageStickToBottom(): void }
): FollowLog {
    const followLabel = (on: boolean): string => (on ? "following" : "detached");
    let lastLoggedFollow = untrack(viewState.stickToBottom);
    let pendingFollowCause: { cause: string; detail?: string } | null = null;
    const line = (change: string, cause: string, detail?: string): void =>
        console.info("[scroll-follow]", `pane=${paneTag()}`, change, `cause=${cause}${detail ? ` ${detail}` : ""}`);
    const logFollowIfChanged = (now: boolean): void => {
        if (now === lastLoggedFollow) return;
        const { cause, detail } = pendingFollowCause ?? { cause: "external" };
        line(`${followLabel(lastLoggedFollow)}→${followLabel(now)}`, cause, detail);
        lastLoggedFollow = now;
    };
    const transitionFollow = (cause: string, detail: string | undefined, apply: () => void): void => {
        pendingFollowCause = { cause, detail };
        try {
            apply();
            logFollowIfChanged(untrack(viewState.stickToBottom));
        } finally {
            pendingFollowCause = null;
        }
    };
    console.info("[scroll-follow]", `pane=${paneTag()}`, `mount ${followLabel(lastLoggedFollow)}`);
    createEffect(() => logFollowIfChanged(viewState.stickToBottom()));
    return {
        transitionFollow,
        project(from, to, cause, detail) {
            if (intentOf(from) !== intentOf(to)) {
                const following = intentOf(to) === "FOLLOWING";
                transitionFollow(cause, detail, () =>
                    following ? viewState.engageStickToBottom() : viewState.disengageStickToBottom()
                );
            } else {
                // Hidden / shown: the intent (stickToBottom) is unchanged.
                line(`${followStateLabel(from)}→${followStateLabel(to)}`, cause, detail);
            }
        },
    };
}
