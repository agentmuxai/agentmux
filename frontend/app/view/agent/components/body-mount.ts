// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createEffect, createSignal, onCleanup, untrack, type Accessor } from "solid-js";

/** How long a closed panel keeps its body before letting go of it: well past
 *  the panel's 120 ms collapse, so the animation runs on the real content and
 *  a quick close-then-open (a hold ending, a re-pin) finds it still there. */
export const BODY_LINGER_MS = 1000;

/**
 * Whether a tool panel's body should be mounted: while `open()`, and for
 * `lingerMs` after it closes. A body that was never opened is never built.
 *
 * A collapsed panel is `max-height: 0` with no padding or margin, so dropping
 * its body after the collapse leaves the row's height as it was; keeping it
 * mounted forever kept every live tool call's full output (DOM, layout and
 * reactive state) for the life of the row.
 * docs/specs/SPEC_TOOL_BODY_UNMOUNT_ON_COLLAPSE_2026_10_12.md.
 */
export function createBodyMount(open: Accessor<boolean>, lingerMs = BODY_LINGER_MS): Accessor<boolean> {
    const [mounted, setMounted] = createSignal(untrack(open));
    let timer: ReturnType<typeof setTimeout> | undefined;
    createEffect(() => {
        if (open()) {
            clearTimeout(timer);
            timer = undefined;
            setMounted(true);
        } else if (untrack(mounted) && timer === undefined) {
            timer = setTimeout(() => {
                timer = undefined;
                setMounted(false);
            }, lingerMs);
        }
    });
    onCleanup(() => clearTimeout(timer));
    return mounted;
}
