// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The agent transcript's jump to a row it has not measured yet (SPEC_AGENT_PANE_SEARCH_AND_DIM_MODE_2026_11_06
 * §4.3), wired to the list: where the aim comes from (the layout slice for the virtualised region, the page for
 * the streaming tail) and when the layout is ready. The loop itself is scroll/settle-on-node.ts.
 */

import { snapshot as snapshotLayout, type LayoutView } from "@/app/store/agent-pane-layout-store";
import { onCleanup, type Accessor } from "solid-js";
import { scrollTranscriptTo } from "../scroll/one-way-flow";
import { createNodeSettler, type NodeSettler } from "../scroll/settle-on-node";

/** What the jump reads of the list's props. */
export interface JumpProps {
    blockId?: string;
    layoutView?: Accessor<LayoutView | null>;
    zoomFactor?: Accessor<number>;
    viewState: { indexOf: (nodeId: string) => number };
}

/** What the jump reads and holds of the list's state. */
export interface JumpHost {
    scrollRef: HTMLDivElement;
    partition: Accessor<{ splitIndex: number }>;
    input: { windowOpen: () => boolean; pointerHeld: () => boolean };
    settleNode: NodeSettler | null;
}

/**
 * The layout has seen where the transcript is (its scroll position is the page's, clamped as the page clamps it)
 * and has a height for every row of its window: a jump that holds its aim now has landed, not just not heard yet.
 */
function layoutIsReady(props: JumpProps, s: JumpHost): boolean {
    const state = props.blockId ? snapshotLayout(props.blockId) : null;
    const view = props.layoutView?.();
    if (!state || !view || !s.scrollRef) return true;
    if (Math.abs(state.scrollTop - s.scrollRef.scrollTop) >= 1) return false; // perf:allow-layout-read — a jump the person asked for
    for (let i = view.window.startIndex; i <= view.window.endIndex; i++) {
        const row = view.rows[i];
        if (row && !state.heights.has(row.nodeId)) return false;
    }
    return true;
}

/**
 * A jump to a row outside the window aims by the estimate, then re-aims as the rows near it are measured
 * (scroll/settle-on-node.ts). The target is read from the layout slice by node id each frame, so rows that arrive
 * or are measured meanwhile move the aim and not the person's place.
 */
export function createJumpSettler(props: JumpProps, s: JumpHost): { start: (nodeId: string) => void } {
    let target = "";
    const aimAt = (nodeId: string): number | null => {
        if (!s.scrollRef) return null;
        const idx = props.viewState.indexOf(nodeId);
        if (idx >= s.partition().splitIndex) {
            // The streaming tail is drawn outside the virtualiser: its rows sit where the page puts them, after the
            // virtualised region's height, which changes as rows above are measured.
            const el = s.scrollRef.querySelector<HTMLElement>(`[data-node-id="${nodeId}"]`);
            if (!el) return null;
            return Math.max(0, el.offsetTop - s.scrollRef.clientHeight / 2 + el.clientHeight / 2); // perf:allow-layout-read — a jump the person asked for, at most a few frames
        }
        const row = props.layoutView?.()?.rows[idx];
        if (!row) return null;
        return Math.max(0, row.start - s.scrollRef.clientHeight / 2 + row.height / 2); // perf:allow-layout-read — same
    };
    s.settleNode = createNodeSettler({
        aim: () => aimAt(target),
        write: (top) => s.scrollRef && scrollTranscriptTo(s.scrollRef, top),
        userActive: () => s.input.windowOpen() || s.input.pointerHeld(),
        viewKey: () => `${props.zoomFactor?.() ?? 1}:${s.scrollRef?.clientHeight ?? 0}`, // perf:allow-layout-read — same
        ready: () => layoutIsReady(props, s),
        frame: (cb) => requestAnimationFrame(cb),
        cancelFrame: (id) => cancelAnimationFrame(id),
        now: () => performance.now(),
    });
    onCleanup(() => s.settleNode?.cancel());
    return {
        start: (nodeId) => {
            target = nodeId;
            s.settleNode?.start();
        },
    };
}
