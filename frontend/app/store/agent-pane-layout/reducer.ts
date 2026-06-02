// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Pure reducer + selectors for the agent-pane layout slice (#11).
 * See ./types.ts for the invariants and
 * docs/specs/SPEC_AGENT_PANE_LAYOUT_REDUCER_2026_06_02.md.
 *
 * The reducer is total and pure: `(state, command) -> { state, events }`,
 * no I/O, no time, no mutation of inputs. No-ops return the SAME state
 * reference (so the store's projection-on-change differ short-circuits).
 */

import {
    AgentPaneLayoutCommand,
    AgentPaneLayoutEvent,
    AgentPaneLayoutState,
    COLLAPSED,
    DEFAULT_ROW_PX,
    Expansion,
    ExpansionState,
    expansionEq,
    inFlowState,
    ReducerResult,
    RowHeight,
} from "./types";

// ── Internal helpers ─────────────────────────────────────────────────

/** A height is usable iff finite and non-negative — anything else would
 *  poison the prefix-sum (INV-1). */
function isValidPx(n: number): boolean {
    return Number.isFinite(n) && n >= 0;
}

/** Height the layout is CURRENTLY using for (nodeId, state): measured,
 *  else estimate, else default. Used to compute `row-measured` deltas. */
function layoutHeightFor(
    state: AgentPaneLayoutState,
    nodeId: string,
    st: ExpansionState,
): number {
    const measured = state.heights.get(nodeId)?.[st];
    if (measured != null) return measured;
    const est = state.estimates.get(nodeId)?.[st];
    if (est != null) return est;
    return DEFAULT_ROW_PX;
}

/** Set a row's expansion, collapsing === deleting the key (default).
 *  No-op (same state ref) when unchanged. */
function setExpansion(
    state: AgentPaneLayoutState,
    nodeId: string,
    to: Expansion,
): ReducerResult {
    const from = state.expansion.get(nodeId) ?? COLLAPSED;
    if (expansionEq(from, to)) return { state, events: [] };
    const expansion = new Map(state.expansion);
    if (to.open) expansion.set(nodeId, to);
    else expansion.delete(nodeId);
    return {
        state: { ...state, expansion },
        events: [{ type: "expansion-changed", nodeId, from, to }],
    };
}

/** Write one (nodeId, state) slot into a height-ish map immutably. */
function withSlot(
    map: ReadonlyMap<string, RowHeight>,
    nodeId: string,
    st: ExpansionState,
    cssPx: number,
): ReadonlyMap<string, RowHeight> {
    const prev = map.get(nodeId);
    const next: RowHeight = { ...prev, [st]: cssPx };
    return new Map(map).set(nodeId, next);
}

// ── Reducer ──────────────────────────────────────────────────────────

export function update(
    state: AgentPaneLayoutState,
    command: AgentPaneLayoutCommand,
): ReducerResult {
    switch (command.type) {
        case "NodesChanged": {
            const next = command.orderedIds;
            const keep = new Set(next);
            let removed = 0;
            for (const id of state.orderedIds) if (!keep.has(id)) removed++;
            const sameOrder =
                next.length === state.orderedIds.length && removed === 0;
            if (sameOrder) {
                // Same set AND same length → same order (sets equal + lengths
                // equal ⇒ permutation; but ids are unique, so identical order is
                // the only reorder we don't separately track). Treat the common
                // append/no-op case: only short-circuit when the arrays are
                // referentially or element-wise identical.
                let identical = true;
                for (let i = 0; i < next.length; i++) {
                    if (next[i] !== state.orderedIds[i]) {
                        identical = false;
                        break;
                    }
                }
                if (identical) return { state, events: [] };
            }
            // Prune heights/estimates/expansion for ids no longer present.
            const heights = pruneMap(state.heights, keep);
            const estimates = pruneMap(state.estimates, keep);
            const expansion = pruneMap(state.expansion, keep);
            return {
                state: {
                    ...state,
                    orderedIds: next,
                    heights,
                    estimates,
                    expansion,
                },
                events: removed > 0 ? [{ type: "ids-pruned", removed }] : [],
            };
        }

        case "UserExpanded":
            return setExpansion(state, command.nodeId, { open: true, via: "pin" });

        case "UserCollapsed":
            return setExpansion(state, command.nodeId, { open: false });

        case "AutoExpandStarted": {
            const cur = state.expansion.get(command.nodeId);
            // A user pin outranks auto-expand — leave it.
            if (cur?.open && cur.via === "pin") return { state, events: [] };
            return setExpansion(state, command.nodeId, { open: true, via: "auto" });
        }

        case "AutoExpandHoldExpired": {
            const cur = state.expansion.get(command.nodeId);
            if (!cur?.open || cur.via !== "auto") {
                // Pinned (or already collapsed) during the hold → expiry is a
                // no-op. Surface it so the audit shows the suppression.
                return {
                    state,
                    events: [{ type: "command-dropped", reason: "hold-expired-not-auto" }],
                };
            }
            return setExpansion(state, command.nodeId, { open: false });
        }

        case "RowMeasured": {
            // Guard the prefix-sum: a NaN/Infinity/negative height would
            // poison every position downstream (INV-1). Drop, don't store.
            if (!isValidPx(command.cssPx)) {
                return {
                    state,
                    events: [{ type: "command-dropped", reason: "invalid-measure-px" }],
                };
            }
            const prev = state.heights.get(command.nodeId)?.[command.state];
            if (prev === command.cssPx) return { state, events: [] };
            const delta =
                command.cssPx -
                layoutHeightFor(state, command.nodeId, command.state);
            return {
                state: {
                    ...state,
                    heights: withSlot(
                        state.heights,
                        command.nodeId,
                        command.state,
                        command.cssPx,
                    ),
                },
                events: [
                    {
                        type: "row-measured",
                        nodeId: command.nodeId,
                        state: command.state,
                        delta,
                    },
                ],
            };
        }

        case "EstimateSet": {
            if (!isValidPx(command.cssPx)) {
                return {
                    state,
                    events: [{ type: "command-dropped", reason: "invalid-estimate-px" }],
                };
            }
            const prev = state.estimates.get(command.nodeId)?.[command.state];
            if (prev === command.cssPx) return { state, events: [] };
            return {
                state: {
                    ...state,
                    estimates: withSlot(
                        state.estimates,
                        command.nodeId,
                        command.state,
                        command.cssPx,
                    ),
                },
                events: [],
            };
        }

        case "MeasurementInvalidated": {
            if (!state.heights.has(command.nodeId)) return { state, events: [] };
            const heights = new Map(state.heights);
            heights.delete(command.nodeId);
            return {
                state: { ...state, heights },
                events: [
                    { type: "measurement-invalidated", nodeId: command.nodeId },
                ],
            };
        }

        case "Scrolled": {
            if (
                state.scrollTop === command.scrollTop &&
                state.viewportPx === command.viewportPx
            ) {
                return { state, events: [] };
            }
            return {
                state: {
                    ...state,
                    scrollTop: command.scrollTop,
                    viewportPx: command.viewportPx,
                },
                events: [],
            };
        }

        case "ScrollMarginChanged": {
            if (state.scrollMarginPx === command.px) return { state, events: [] };
            return { state: { ...state, scrollMarginPx: command.px }, events: [] };
        }

        case "ZoomChanged": {
            // INV-2: zoom never touches heights/positions. The single
            // ancestor CSS `zoom` re-scales the unzoomed layout at render.
            if (state.zoom === command.zoom) return { state, events: [] };
            return {
                state: { ...state, zoom: command.zoom },
                events: [{ type: "zoom-changed-no-relayout", zoom: command.zoom }],
            };
        }
    }
}

/** Immutable filter of a Map down to the `keep` key set. Returns the SAME
 *  reference when nothing is removed (preserves no-op short-circuiting). */
function pruneMap<V>(
    map: ReadonlyMap<string, V>,
    keep: ReadonlySet<string>,
): ReadonlyMap<string, V> {
    let anyRemoved = false;
    for (const k of map.keys()) {
        if (!keep.has(k)) {
            anyRemoved = true;
            break;
        }
    }
    if (!anyRemoved) return map;
    const next = new Map<string, V>();
    for (const [k, v] of map) if (keep.has(k)) next.set(k, v);
    return next;
}

// ── Selectors (pure projections — the renderer reads these) ──────────

/** In-flow CSS-px height of a row in its current state: measured, else
 *  estimate, else default. */
export function effectiveHeight(
    state: AgentPaneLayoutState,
    nodeId: string,
): number {
    return layoutHeightFor(state, nodeId, inFlowState(state.expansion.get(nodeId)));
}

export interface RowPosition {
    nodeId: string;
    index: number;
    /** Unzoomed CSS px from the scroll-container top, includes scrollMargin. */
    start: number;
    height: number;
    /** === start + height === next row's start (INV-1). */
    end: number;
}

/** Prefix-sum positions over the full virtualized region. O(n).
 *  start[i+1] === end[i] by construction → slots never overlap (INV-1). */
export function positions(state: AgentPaneLayoutState): RowPosition[] {
    const out: RowPosition[] = new Array(state.orderedIds.length);
    let cursor = state.scrollMarginPx;
    for (let i = 0; i < state.orderedIds.length; i++) {
        const nodeId = state.orderedIds[i];
        const height = effectiveHeight(state, nodeId);
        const start = cursor;
        const end = start + height;
        out[i] = { nodeId, index: i, start, height, end };
        cursor = end;
    }
    return out;
}

/** Total scrollable height of the virtualized region (excludes scrollMargin,
 *  which is contributed by the header element above it). */
export function totalSize(state: AgentPaneLayoutState): number {
    let sum = 0;
    for (const id of state.orderedIds) sum += effectiveHeight(state, id);
    return sum;
}

/** Inclusive visible-row index range, padded by overscan. An empty region
 *  returns `{ startIndex: 0, endIndex: -1 }` (start > end ⇒ render nothing). */
export interface WindowRange {
    startIndex: number;
    endIndex: number;
}

export function windowRange(state: AgentPaneLayoutState): WindowRange {
    const pos = positions(state);
    if (pos.length === 0) return { startIndex: 0, endIndex: -1 };

    const top = state.scrollTop;
    const bottom = state.scrollTop + state.viewportPx;

    // First row whose `end > top` (lowest index still visible at the top edge).
    let lo = 0;
    let hi = pos.length - 1;
    let first = pos.length; // sentinel: none visible
    while (lo <= hi) {
        const mid = (lo + hi) >> 1;
        if (pos[mid].end > top) {
            first = mid;
            hi = mid - 1;
        } else {
            lo = mid + 1;
        }
    }
    if (first === pos.length) return { startIndex: 0, endIndex: -1 };

    // Last row whose `start < bottom` (highest index still visible).
    lo = 0;
    hi = pos.length - 1;
    let last = -1;
    while (lo <= hi) {
        const mid = (lo + hi) >> 1;
        if (pos[mid].start < bottom) {
            last = mid;
            lo = mid + 1;
        } else {
            hi = mid - 1;
        }
    }
    if (last < first) return { startIndex: 0, endIndex: -1 };

    return {
        startIndex: Math.max(0, first - state.overscan),
        endIndex: Math.min(pos.length - 1, last + state.overscan),
    };
}
