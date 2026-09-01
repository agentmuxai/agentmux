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
 * This module diffs a SNAPSHOT of the rendered rows' heights against the
 * previous snapshot, and reports which rows shrank plus how much of the pane
 * delta they fail to explain.
 *
 * ## Why snapshot-diffing rather than a ResizeObserver
 *
 * The first version of this fed a `ResizeObserver` into a time-windowed ring
 * buffer. That is broken for the primary case, and codex caught it on PR #2887:
 * the pin effect defers `scrollToTrueBottom()` with `queueMicrotask`, and
 * reading `scrollHeight` there forces a layout flush — so the PANE shrink is
 * detected during the microtask checkpoint, while `ResizeObserver` callbacks
 * are not delivered until later in the rendering steps. A tool going
 * running->terminal would therefore log as wholly unattributed, and its row
 * shrink would still be sitting in the ring afterwards to be miscredited to
 * some unrelated later pane delta. Systematically wrong on the one case the
 * instrumentation exists for, and wrong in the direction that invents
 * false attributions.
 *
 * Sampling synchronously from the caller removes the ordering dependency
 * instead of trying to schedule around it: both numbers are then read from the
 * same layout-clean instant, and the row window is exactly the pane window
 * (previous pin check -> this one) rather than a 250ms approximation of it.
 * The caller reads `scrollHeight` immediately before sampling, so layout is
 * already flushed and the per-row reads cost no extra reflow.
 *
 * Heights are `offsetHeight`, deliberately, to match the pane's `scrollHeight`:
 * both are unzoomed under an ancestor CSS `zoom`, whereas
 * `getBoundingClientRect().height` is scaled by it. Mixing the two would make
 * every attribution wrong by the zoom factor at non-100% pane zoom.
 *
 * Deliberately NOT gated on `import.meta.env.DEV`, unlike `perf-probe.ts`: the
 * most useful data so far came from `task package` local builds, where a DEV
 * gate would compile this out entirely.
 */

/** One row's height at sample time, as read by the caller from the DOM. */
export interface RowSample {
    id: string;
    /** `DocumentNode["type"]` — "tool", "markdown", "thinking", … */
    type: string;
    px: number;
}

/** One observed row that got shorter since the previous sample. Growth is not
 *  reported — the pin handles growth invisibly (it teleports to a bottom that
 *  moved further away, which it already does every frame while streaming). */
export interface RowShrink {
    nodeId: string;
    nodeType: string;
    fromPx: number;
    toPx: number;
}

export interface Attribution {
    shrinks: RowShrink[];
    /** Sum of the per-row shrinks. */
    attributedPx: number;
    /** paneDeltaPx - attributedPx. Positive means the observed rows do NOT
     *  fully explain the pane shrink (something unobserved did — the
     *  virtualized region, the working-row overlay, the panel's own
     *  padding/margin collapse). Negative means rows shrank more than the pane
     *  did, i.e. something grew at the same time. Either way a non-zero value
     *  means "do not stop here" — reporting it is what keeps this diagnostic
     *  from manufacturing the same over-confident conclusions the 08-21/08-22
     *  findings had to be corrected for. */
    unattributedPx: number;
}

export class ShrinkTrace {
    private heights = new Map<string, number>();

    /**
     * Diff `samples` against the previous call and return the rows that got
     * shorter. Ids absent from `samples` are dropped, so a row that unmounted
     * (a streaming-buffer cap-advance retiring it) cannot later appear to have
     * shrunk across the gap when the same node re-renders at a new height.
     *
     * The first sample of a node only establishes a baseline — it is never a
     * shrink, because there is nothing to have shrunk from.
     */
    sample(samples: RowSample[]): RowShrink[] {
        const shrinks: RowShrink[] = [];
        const next = new Map<string, number>();
        for (const s of samples) {
            const prev = this.heights.get(s.id);
            next.set(s.id, s.px);
            if (prev !== undefined && s.px < prev) {
                shrinks.push({ nodeId: s.id, nodeType: s.type, fromPx: prev, toPx: s.px });
            }
        }
        this.heights = next;
        return shrinks;
    }
}

/** Stateless — the sample diff already scoped these to the right window. */
export function attribute(paneDeltaPx: number, shrinks: RowShrink[]): Attribution {
    const attributedPx = shrinks.reduce((sum, s) => sum + (s.fromPx - s.toPx), 0);
    return { shrinks, attributedPx, unattributedPx: paneDeltaPx - attributedPx };
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
