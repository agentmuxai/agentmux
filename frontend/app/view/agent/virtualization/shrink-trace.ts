// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Per-node height-shrink attribution — step 1 of
 * docs/specs/SPEC_CONTENT_RESIZE_CONTRACT_2026_08_31.md.
 *
 * The existing `[wave-scroll-shrink]` diagnostic
 * (`AgentDocumentVirtualList.scrollToTrueBottom`) reports that the pane's
 * `scrollHeight` got smaller between two pin-check calls. It cannot say WHICH
 * content shrank, and that limit is why every conclusion in
 * `FINDINGS_TOOL_CALL_SCROLL_OSCILLATION_2026_08_21.md` and its 08-22 live-data
 * follow-up had to be hedged: a pane-level net delta is consistent with one big
 * collapse, several small ones, or a growth and a shrink cancelling out. The
 * 08-22 doc's last remaining lead (a recurring ~251-252px shrink) was closed on
 * 08-31 without a culprit for exactly this reason — no fixed-height component
 * of that size exists, so the number was almost certainly a sum.
 *
 * This module records each observed row's height as it changes and keeps a
 * bounded log of the SHRINKS only. When the pane-level diagnostic fires, it
 * asks for the shrinks seen in the immediately-preceding window and reports
 * them alongside the pane delta — turning "the pane lost 251px" into "tool node
 * tc-9 went 13400px -> 120px, and 222px is still unaccounted for".
 *
 * The unattributed remainder is the point, not a rounding detail: it is what
 * tells you whether the observed rows explain the pane's shrink or whether
 * something not being observed (the virtualized region, the working-row
 * overlay, the panel's own padding/margin collapse) is responsible. A
 * conclusion drawn from attribution alone, without checking that remainder,
 * would repeat the exact error the 08-21/08-22 findings were corrected for.
 *
 * Deliberately NOT gated on `import.meta.env.DEV`, unlike `perf-probe.ts`: the
 * most useful data so far came from `task package` local builds, where a DEV
 * gate would compile this out entirely. Cost is bounded instead by only
 * recording numbers on resize and only formatting a string when a pane shrink
 * has already been detected.
 */

/** One observed row getting shorter. Growth is not recorded — the pane pin
 *  handles growth invisibly (it teleports to a bottom that is further away,
 *  which is what it already does every frame while streaming). */
export interface RowShrink {
    nodeId: string;
    /** `DocumentNode["type"]` — "tool", "markdown", "thinking", … */
    nodeType: string;
    fromPx: number;
    toPx: number;
    atMs: number;
}

/** Bounded so a long streaming session can't grow this without limit. Sized to
 *  comfortably cover one pin-check interval's worth of resizes (the 08-21 data
 *  showed pin checks ~160ms apart under load; a burst of that length is a
 *  handful of rows, not dozens). */
const RING_SIZE = 64;

/** How far back `attribute()` looks for shrinks to blame a pane delta on.
 *  Pin checks in the 08-21 dataset were ~160ms apart at their closest; this
 *  covers that with margin without reaching back into a previous, unrelated
 *  turn's activity. */
export const ATTRIBUTION_WINDOW_MS = 250;

export interface Attribution {
    shrinks: RowShrink[];
    /** Sum of the per-row shrinks in the window. */
    attributedPx: number;
    /** paneDeltaPx - attributedPx. Positive means observed rows do NOT fully
     *  explain the pane shrink; negative means rows shrank more than the pane
     *  did (something else grew at the same time). Either way, a non-zero
     *  value means "do not stop here". */
    unattributedPx: number;
}

export class ShrinkTrace {
    private heights = new Map<string, number>();
    private ring: RowShrink[] = [];

    /**
     * Record a row's current height. The first observation for a node only
     * establishes a baseline — it is never a shrink, because there is nothing
     * to have shrunk from. (A row mounting at 0px and laying out at its real
     * height would otherwise register as growth on the second call and noise
     * on every subsequent remount.)
     */
    record(nodeId: string, nodeType: string, px: number, atMs: number): void {
        const prev = this.heights.get(nodeId);
        this.heights.set(nodeId, px);
        if (prev === undefined || px >= prev) return;
        this.ring.push({ nodeId, nodeType, fromPx: prev, toPx: px, atMs });
        if (this.ring.length > RING_SIZE) this.ring.shift();
    }

    /**
     * Drop a node's baseline. Called when its row unmounts — without this, a
     * node that leaves the streaming buffer tall and later re-renders short
     * (history reload, cap-advance re-add) would report a fabricated shrink
     * spanning the gap.
     */
    forget(nodeId: string): void {
        this.heights.delete(nodeId);
    }

    /**
     * Explain `paneDeltaPx` using shrinks recorded in the preceding
     * `windowMs`. Consumed entries are removed so the next pane shrink can't
     * be credited to the same rows twice.
     */
    attribute(paneDeltaPx: number, nowMs: number, windowMs: number = ATTRIBUTION_WINDOW_MS): Attribution {
        const cutoff = nowMs - windowMs;
        const shrinks = this.ring.filter((s) => s.atMs >= cutoff);
        this.ring = [];
        const attributedPx = shrinks.reduce((sum, s) => sum + (s.fromPx - s.toPx), 0);
        return { shrinks, attributedPx, unattributedPx: paneDeltaPx - attributedPx };
    }

    /** Test/teardown hook — drops all baselines and pending shrinks. */
    reset(): void {
        this.heights.clear();
        this.ring = [];
    }
}

/**
 * Render an attribution as a single log-line suffix. Node ids are truncated to
 * 7 chars to match the existing `pane=` field's convention in the same line.
 */
export function formatAttribution(a: Attribution): string {
    if (a.shrinks.length === 0) {
        return `attributed: none (no observed row shrank; unattributed=${Math.round(a.unattributedPx)}px)`;
    }
    const parts = a.shrinks.map(
        (s) => `${s.nodeId.slice(0, 7)}(${s.nodeType}) ${Math.round(s.fromPx)}->${Math.round(s.toPx)}px`,
    );
    return `attributed: ${parts.join(" | ")} (sum=${Math.round(a.attributedPx)}px, unattributed=${Math.round(a.unattributedPx)}px)`;
}
