// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Follow (tail) mode of the Large text viewer: while it is on, a log that grows
 * keeps its last line on screen. The viewer already reads the file again when it
 * changes on disk; this only moves the view to the end when the file got longer.
 * Scrolling away from the end turns it off, as `tail -f` is left.
 * docs/specs/SPEC_EDITOR_REVAMP_UNIFIED_VIEWERS_2026_10_15.md §4.4.6.
 */

import { createEffect, createSignal, on, type Accessor } from "solid-js";
import { maxFirstRow } from "../hex/hex-layout";
import type { LineScroll } from "./line-scroll";

export function createFollow(lines: Accessor<number>, scroll: LineScroll) {
    const [following, setFollowing] = createSignal(false);
    const toEnd = () => scroll.showLine(lines());
    // The file got longer: show its new end.
    createEffect(
        on(
            lines,
            (n, was) => {
                if (following() && n > (was ?? 0)) toEnd();
            },
            { defer: true }
        )
    );
    // The reader moved off the end: they are reading, not following.
    createEffect(
        on(
            scroll.first,
            (first) => {
                if (following() && first < maxFirstRow(lines(), scroll.viewPx())) setFollowing(false);
            },
            { defer: true }
        )
    );
    return {
        following,
        toggle: () => {
            const next = !following();
            setFollowing(next);
            if (next) toEnd();
        },
    };
}

export type Follow = ReturnType<typeof createFollow>;
