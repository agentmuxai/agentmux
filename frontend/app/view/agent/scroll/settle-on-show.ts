// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createEffect, createSignal, onCleanup, untrack, type Accessor } from "solid-js";
import { useAgentDormant } from "../agent-dormancy";

/**
 * The longest a pane shown again stays covered, counted from the first frame
 * after the show. Under OneWayFlow's SHOW_SETTLE_MS (400), so the cover lifts
 * while layout changes still land at the bottom.
 */
export const SHOW_COVER_MAX_MS = 350;
/** Frames in a row with no change in size before the cover lifts. */
export const SHOW_COVER_QUIET_FRAMES = 2;

/** Cover the transcript on a show when what it holds changed while hidden. */
export interface ShowCover {
    /** Changes identity when the transcript changes (the document's nodes). */
    changes: Accessor<unknown>;
    /** What to watch for size changes while covered (the transcript's containers). */
    observe: () => readonly (Element | undefined)[];
}

/** Timing primitives, injectable for tests. */
export interface ShowCoverDeps {
    frame: (cb: () => void) => number;
    cancelFrame: (id: number) => void;
    now: () => number;
    observer: (onResize: () => void) => { observe(el: Element): void; disconnect(): void } | null;
}

const defaultDeps: ShowCoverDeps = {
    frame: (cb) => requestAnimationFrame(cb),
    cancelFrame: (id) => cancelAnimationFrame(id),
    now: () => performance.now(),
    observer: (onResize) => (typeof ResizeObserver === "undefined" ? null : new ResizeObserver(onResize)),
};

/**
 * When this agent pane is shown again after its tab was hidden, and it follows,
 * the content it got while hidden (laid out but not drawn) commits now: have
 * the one-way flow land each stage of that at the bottom instead of easing to
 * it (`OneWayFlow.settleOnShow`). A pane the user scrolled up keeps its place.
 * SPEC_AGENT_PANE_NO_CATCH_UP_MOTION_2026_10_15.md §3.1.
 *
 * With `cover`, the returned accessor is true while the transcript should stay
 * hidden: from a show where it changed while hidden until its size has held
 * still for SHOW_COVER_QUIET_FRAMES frames (at most SHOW_COVER_MAX_MS), so it
 * appears once, at its latest content, instead of moving up in stages as that
 * content is drawn (§3.7).
 */
export function useSettleOnShow(
    flow: { settleOnShow(): void } | null,
    following: Accessor<boolean>,
    cover?: ShowCover,
    deps: ShowCoverDeps = defaultDeps,
): Accessor<boolean> {
    const [covered, setCovered] = createSignal(false);
    if (!flow) return covered;
    const dormant = useAgentDormant();
    // The previous value by hand: `on(..., { defer: true })` doesn't record it
    // on its skipped first run, so the first show would read "wasn't hidden".
    let wasHidden = untrack(dormant);
    let atHide = cover ? untrack(cover.changes) : undefined;
    let stop: (() => void) | undefined;
    const lift = (): void => {
        stop?.();
        stop = undefined;
        setCovered(false);
    };
    const coverUntilStill = (c: ShowCover): void => {
        let resized = false;
        let quiet = 0;
        let startedAt: number | undefined;
        let raf = 0;
        const ro = deps.observer(() => {
            resized = true;
        });
        for (const el of c.observe()) if (el) ro?.observe(el);
        const tick = (): void => {
            const now = deps.now();
            startedAt ??= now;
            quiet = resized ? 0 : quiet + 1;
            resized = false;
            if (quiet >= SHOW_COVER_QUIET_FRAMES || now - startedAt >= SHOW_COVER_MAX_MS) lift();
            else raf = deps.frame(tick);
        };
        // The first check runs before the first layout: the observer's initial
        // report then counts as a change, so a cover never lifts unlaid out.
        resized = true;
        raf = deps.frame(tick);
        stop = () => {
            deps.cancelFrame(raf);
            ro?.disconnect();
        };
        setCovered(true);
    };
    createEffect(() => {
        const hidden = dormant();
        if (hidden) {
            if (!wasHidden && cover) atHide = untrack(cover.changes);
            lift();
        } else if (wasHidden && untrack(following)) {
            flow.settleOnShow();
            if (cover && untrack(cover.changes) !== atHide) coverUntilStill(cover);
        }
        wasHidden = hidden;
    });
    onCleanup(lift);
    return covered;
}
