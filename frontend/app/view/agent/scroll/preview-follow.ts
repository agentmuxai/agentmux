// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Follow the latest output of a tool preview (ToolOverlayLog.tsx).
 * SPEC_TOOL_PREVIEW_HEIGHT_THIRD_AND_FOLLOW_LATEST_2026_09_25.md §3, and
 * Phase 3 of SPEC_AGENT_PANE_SCROLL_FOLLOW_STATE_MACHINE_2026_09_24.md: the
 * preview runs the transcript's own follow state machine (follow-scroll.ts).
 *
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

/** The preview's FOLLOWING / DETACHED is the shared reducer's. */
export function createPreviewFollow(opts: PreviewFollowOptions): PreviewFollow {
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
            // A preview that starts at the top mounts on a short "Running…"
            // placeholder with nothing to scroll. Fed to the reducer, that
            // geometry would re-follow it (DETACHED + no overflow) and pin the
            // result to the bottom when it arrives. Until the user or streaming
            // decides, only track the geometry.
            if (byDefault) {
                follow.noteGeometry(geometry(el));
                return;
            }
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
