// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Whether a capped tool preview scrolls vertically, reported as it settles
 * (SPEC_AGENT_PANE_TOOL_PEEK_ON_LINE_AND_MD_OPEN_IN_EDITOR_2026_10_11.md §3.1).
 * `ToolBlock` shows its taller/shorter toggle only while this is true: a
 * preview that fits has nothing more to show.
 *
 * Not a bare `scrollHeight > clientHeight`. The panel opens by a `max-height`
 * transition, and the height FLIP pins the box's height while it eases, so
 * during either the box is clipped to a fraction of its final height and
 * content that fits would read as overflowing, and the toggle would appear for
 * a moment on every short output. So a reading waits until nothing in the
 * panel is transitioning.
 */

/** Pixels of slack: a fractional layout can leave `scrollHeight` one above `clientHeight` with nothing to scroll. */
const SLACK_PX = 1;

export interface OverflowReporterOptions {
    /** The scrolling box. */
    el: () => HTMLElement | undefined;
    /** True while the box is in a `content-visibility: hidden` subtree, where reading layout forces a render. */
    hidden: () => boolean;
    /** Who to tell; read on each report so a prop that changes is followed. */
    onChange: () => ((overflows: boolean) => void) | undefined;
}

/** Transitions and animations in `panel` that will end, and haven't. An endless one (a blinking cursor) is not waited for. */
function pendingAnimations(panel: Element | null | undefined): Animation[] {
    const all = panel?.getAnimations?.({ subtree: true }) ?? [];
    return all.filter((a) => {
        // A just-created transition is "running" with `pending` set.
        if (a.playState !== "running") return false;
        const end = a.effect?.getComputedTiming?.().endTime;
        return typeof end === "number" && Number.isFinite(end);
    });
}

/** Returns `check`, to call when the box or its content changes size. Calls `onChange` only when the answer changes. */
export function createOverflowReporter(opts: OverflowReporterOptions): { check: () => void } {
    let last: boolean | undefined;
    let waiting = false;

    const check = (): void => {
        const el = opts.el();
        const notify = opts.onChange();
        if (!el || !notify || !el.isConnected || opts.hidden()) return;
        const pending = pendingAnimations(el.closest(".agent-tool-panel"));
        if (pending.length > 0) {
            if (waiting) return;
            waiting = true;
            // Settled, not resolved: a cancelled transition rejects `finished`.
            void Promise.allSettled(pending.map((a) => a.finished)).then(() => {
                waiting = false;
                check();
            });
            return;
        }
        const next = el.scrollHeight - el.clientHeight > SLACK_PX;
        if (next === last) return;
        last = next;
        notify(next);
    };
    return { check };
}
