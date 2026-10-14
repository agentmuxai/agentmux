// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Follow the latest output of a tool preview (ToolOverlayLog.tsx).
 * SPEC_TOOL_PREVIEW_HEIGHT_THIRD_AND_FOLLOW_LATEST_2026_09_25.md §3, and
 * Phase 3 of SPEC_AGENT_PANE_SCROLL_FOLLOW_STATE_MACHINE_2026_09_24.md: with
 * `agent:followcontroller` on, the preview runs the transcript's own follow
 * state machine (follow-scroll.ts); off, its earlier local copy of the same
 * rules (`legacyPreviewFollow`), kept as the kill switch until Phase 4.
 *
 * Either way:
 * - FOLLOWING pins to the bottom on every content or box resize (`resized`,
 *   called from the preview's ResizeObserver).
 * - Only the user can detach: a scroll with no wheel / touch / scrollbar /
 *   scroll-key input in the input window never does, whatever its geometry
 *   (a browser clamp, scroll anchoring, the output cap trimming head lines).
 * - A user scroll that ends within 24 px of the bottom re-attaches.
 * - A preview that starts at the top (documents, content-first answers)
 *   starts DETACHED; if it starts streaming, it follows (`streamingStarted`).
 * Every change logs one `[scroll-follow] tool=<id> …` line.
 */

import { DETACHED, FOLLOWING, intentOf, type ScrollGeometry } from "./follow-controller";
import { createFollowScroll, type FollowScroll } from "./follow-scroll";

export interface PreviewFollow {
    /** Bind to the scroller once mounted; returns the cleanup. */
    attach(el: HTMLElement): () => void;
    following(): boolean;
    /** Still DETACHED only because the preview starts at the top. */
    detachedByDefault(): boolean;
    /** ResizeObserver callback (layout clean): pin while following. */
    resized(): void;
    /** Without ResizeObserver: the pin a rAF after the DOM flush. */
    pin(): void;
    /** The `scroll` handler. */
    scrolled(): void;
    /** A preview that started at the top began streaming: follow it. */
    streamingStarted(): void;
    /** A different tool node took over this slot: start over from `following`. */
    restart(following: boolean): void;
}

export interface PreviewFollowOptions {
    el: () => HTMLElement | undefined;
    /** Inside a content-visibility:hidden panel: read and write no geometry. */
    hidden: () => boolean;
    /** The `tool=` tag for the log line. */
    tag: () => string;
    following: boolean;
}

const logLine = (tag: string, change: string, cause: string, detail?: string): void =>
    console.info("[scroll-follow]", `tool=${tag}`, `${change} cause=${cause}${detail ? ` ${detail}` : ""}`);

/** With the follow controller: the preview's FOLLOWING / DETACHED is the shared reducer's. */
export function createPreviewFollow(opts: PreviewFollowOptions, controller: boolean): PreviewFollow {
    if (!controller) return legacyPreviewFollow(opts);
    let initial = opts.following;
    let byDefault = !initial;
    let follow: FollowScroll | null = null;
    let bound: HTMLElement | null = null;
    const geometry = (el: HTMLElement): ScrollGeometry => ({
        scrollTop: el.scrollTop,
        scrollHeight: el.scrollHeight,
        clientHeight: el.clientHeight,
    });
    // The only writer of scrollTop-to-bottom. Re-checks isConnected: a
    // Switch-branch flip can detach the element, and writing scrollTop on a
    // detached node raised the `replaceChild` race that crashed v0.33.799.
    const pinNow = (): void => {
        const el = opts.el();
        if (!el || !follow?.following() || !el.isConnected || opts.hidden()) return;
        el.scrollTop = el.scrollHeight;
        follow.noteGeometry(geometry(el));
    };
    const bind = (el: HTMLElement): void => {
        follow?.dispose();
        follow = createFollowScroll(
            el,
            {
                // Re-attach and streaming-started pins land on the next frame,
                // like every other pin here: never a layout read in an effect or
                // an input handler.
                pin: () => requestAnimationFrame(pinNow),
                loadOlder: () => {},
                hasOlderHistory: () => false,
                loadingOlder: () => false,
                // Only follow changes, as before: hidden / shown is not one here.
                transition: (from, to, cause, detail) => {
                    if (intentOf(from) === intentOf(to)) return;
                    byDefault = false;
                    const change = intentOf(to) === "FOLLOWING" ? "detached→following" : "following→detached";
                    logLine(opts.tag(), change, cause.replace(/^jump-to-bottom:/, ""), detail);
                },
            },
            initial ? FOLLOWING : DETACHED
        );
    };
    return {
        attach(el) {
            bound = el;
            bind(el);
            return () => {
                follow?.dispose();
                follow = null;
                bound = null;
            };
        },
        following: () => (follow ? follow.following() : initial),
        detachedByDefault: () => byDefault,
        resized() {
            const el = opts.el();
            if (!el || !follow || opts.hidden() || !el.isConnected) return;
            if (follow.observed("content", geometry(el))) pinNow();
        },
        pin: pinNow,
        scrolled() {
            const el = opts.el();
            if (!el || !follow || opts.hidden()) return;
            follow.scrolled(geometry(el), follow.windowOpen() || follow.pointerHeld());
        },
        streamingStarted() {
            follow?.jumpToBottom("streaming-started");
        },
        restart(following) {
            initial = following;
            byDefault = !following;
            if (bound) bind(bound);
        },
    };
}

/** How long after the last user scroll input a scroll still counts as the user's (the pane's window). */
const USER_INPUT_WINDOW_MS = 250;
/** Re-attach within this of the bottom: the pane's REATTACH_PX (pane spec §5.5). */
const REATTACH_PX = 24;
const SCROLL_KEYS = new Set(["PageUp", "PageDown", "Home", "End", "ArrowUp", "ArrowDown", " "]);

/** The preview's own copy of the rules, used with `agent:followcontroller` off. */
function legacyPreviewFollow(opts: PreviewFollowOptions): PreviewFollow {
    let following = opts.following;
    // Detached only because of the document-preview default, not by the user.
    let detachedByDefault = !following;
    let lastScrollTop = 0;
    let lastUserScrollInputAt = Number.NEGATIVE_INFINITY;
    let scrollbarPointerHeld = false;
    const hasRecentUserScrollInput = (): boolean =>
        scrollbarPointerHeld || performance.now() - lastUserScrollInputAt <= USER_INPUT_WINDOW_MS;

    const setFollowing = (next: boolean, cause: string, detail?: string): void => {
        detachedByDefault = false;
        if (next === following) return;
        following = next;
        logLine(opts.tag(), next ? "detached→following" : "following→detached", cause, detail);
    };

    // The only writer of scrollTop-to-bottom. Called from ResizeObserver
    // callbacks (layout is already clean there) or, without RO, from a rAF.
    // Re-checks isConnected: a Switch-branch flip can detach the element, and
    // writing scrollTop on a detached node raised the `replaceChild`
    // reconciliation race that crashed v0.33.799. Never touches a
    // content-visibility:hidden subtree.
    const pinToBottom = (): void => {
        const el = opts.el();
        if (!el || !following || !el.isConnected || opts.hidden()) return;
        el.scrollTop = el.scrollHeight;
        lastScrollTop = el.scrollTop;
    };

    return {
        // Record user scroll input. Wheel and touch on the scroller; a pointer
        // only when pressed on the scroller element itself (its scrollbar —
        // content clicks target a child), held open until release; scroll keys
        // while focus is inside. Target identity, not a hit-test: no layout read
        // in an input handler.
        attach(el) {
            const mark = (): void => {
                lastUserScrollInputAt = performance.now();
            };
            const onPointerDown = (e: PointerEvent): void => {
                if (e.target !== el) return;
                scrollbarPointerHeld = true;
                mark();
            };
            const onPointerUp = (): void => {
                if (!scrollbarPointerHeld) return;
                scrollbarPointerHeld = false;
                mark(); // the drag's trailing scroll events are still the user's
            };
            const onKey = (e: KeyboardEvent): void => {
                if (SCROLL_KEYS.has(e.key)) mark();
            };
            const o: AddEventListenerOptions = { passive: true, capture: true };
            const types = ["wheel", "touchstart", "touchmove"] as const;
            for (const type of types) el.addEventListener(type, mark, o);
            el.addEventListener("pointerdown", onPointerDown, o);
            el.addEventListener("keydown", onKey, o);
            window.addEventListener("pointerup", onPointerUp, o);
            window.addEventListener("pointercancel", onPointerUp, o);
            return () => {
                for (const type of types) el.removeEventListener(type, mark, o);
                el.removeEventListener("pointerdown", onPointerDown, o);
                el.removeEventListener("keydown", onKey, o);
                window.removeEventListener("pointerup", onPointerUp, o);
                window.removeEventListener("pointercancel", onPointerUp, o);
            };
        },
        following: () => following,
        detachedByDefault: () => detachedByDefault,
        resized: pinToBottom,
        pin: pinToBottom,
        scrolled() {
            const el = opts.el();
            if (!el || opts.hidden()) return;
            const top = el.scrollTop;
            const gap = el.scrollHeight - el.clientHeight - top;
            const movedUp = top < lastScrollTop;
            lastScrollTop = top;
            if (!hasRecentUserScrollInput()) return; // not the user: never changes follow
            if (following && movedUp && gap > REATTACH_PX) {
                setFollowing(
                    false,
                    scrollbarPointerHeld ? "user-scroll:scrollbar" : "user-scroll",
                    `gap=${Math.round(gap)}px`
                );
            } else if (!following && gap <= REATTACH_PX) {
                setFollowing(true, "user-scroll-to-bottom");
            }
        },
        streamingStarted() {
            setFollowing(true, "streaming-started");
        },
        restart(next) {
            following = next;
            detachedByDefault = !following;
            lastScrollTop = 0;
        },
    };
}
