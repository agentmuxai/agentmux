// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Agent-pane layout state machine — Slice #11.
 *
 * Owns the deterministic layout model for the agent document's
 * VIRTUALIZED region: per-row in-flow heights (measured + estimated,
 * keyed by expansion state), unified expansion state, zoom, and the
 * scroll viewport. Rendered positions are a PURE prefix-sum projection,
 * so consecutive row slots are flush by construction and overlap is
 * impossible — the bug class behind #1231 / #1233 / #1235.
 *
 * Spec: docs/specs/SPEC_AGENT_PANE_LAYOUT_REDUCER_2026_06_02.md.
 * Conventions: docs/specs/frontend-reducer-conventions-2026-05-03.md.
 *
 * Phase 0 (this file's slice): pure core + store + tests, NO render-path
 * wiring and NO behaviour change. Phases 1–4 wire it in (see spec §6).
 *
 * Invariants (proven by reducer.test.ts):
 *   INV-1  positions() is a prefix-sum of in-flow heights → start[i+1] === end[i].
 *   INV-2  heights/positions are unzoomed CSS px; `ZoomChanged` never relayouts.
 *   INV-3  measurements are keyed by (nodeId, ExpansionState) — an expanded
 *          measurement never contaminates the collapsed slot.
 */

/** In-flow footprint state — the only thing that drives the prefix-sum.
 *  Hover-peek "overlay" is presentational (absolute layer, summary stays
 *  in flow), so it is NOT a layout input. Layout is binary. */
export type ExpansionState = "collapsed" | "expanded";

/** Why a row is open:
 *  - `pin`     — user explicitly pinned it open. Outlives an auto-expand:
 *                a hold expiry collapses an `auto` row but is a no-op on a pin.
 *  - `auto`    — auto-expanded because the tool is running / in its
 *                post-completion hold (mirrors today's `autoExpanded()`).
 *  - `default` — open by KIND default (agent_message, normal markdown / user
 *                message, subagent link, open section) rather than by a user or
 *                lifecycle action. Like `auto`/`pin` for height, but a hold
 *                expiry never touches it (it has no hold). Added in Phase 1 so
 *                default-open kinds are modeled (Phase 0 assumed default-closed).
 *  Collapsed rows are stored as ABSENT from the expansion map, keeping it small. */
export type Expansion =
    | { open: false }
    | { open: true; via: "pin" | "auto" | "default" };

export const COLLAPSED: Expansion = { open: false };

/** Measured/estimated CSS-px height per in-flow state (INV-3).
 *  `undefined` = not yet measured/estimated for that state. */
export interface RowHeight {
    collapsed?: number;
    expanded?: number;
}

export interface AgentPaneLayoutState {
    /** INV-2: layout is zoom-INVARIANT; this only scales the final render. */
    readonly zoom: number;
    /** FULL virtualized-region node ids, in order (not just the visible window). */
    readonly orderedIds: ReadonlyArray<string>;
    /** Membership index over `orderedIds`, kept in lockstep by `NodesChanged`.
     *  O(1) guard so a late `RowMeasured` for an already-removed id is dropped
     *  instead of re-entering the heights map and surviving the next prune. */
    readonly idSet: ReadonlySet<string>;
    /** Open rows only (collapsed === absent). */
    readonly expansion: ReadonlyMap<string, Expansion>;
    /** Measured heights, per state, unzoomed CSS px. */
    readonly heights: ReadonlyMap<string, RowHeight>;
    /** Estimator output, per state — fallback when unmeasured. */
    readonly estimates: ReadonlyMap<string, RowHeight>;
    /** Unzoomed CSS px — windowing input. */
    readonly scrollTop: number;
    /** Scroll-container clientHeight ÷ zoom (unzoomed CSS px). */
    readonly viewportPx: number;
    /** Header offset above the virtualized region (unzoomed CSS px). */
    readonly scrollMarginPx: number;
    /** Extra rows rendered beyond the viewport, each side. */
    readonly overscan: number;
}

/** Height used when a row has neither a measurement nor an estimate for its
 *  current state. Matches the historical `estimateSize` fallback. */
export const DEFAULT_ROW_PX = 32;
const DEFAULT_OVERSCAN = 5;

export const initialState = (): AgentPaneLayoutState => ({
    zoom: 1,
    orderedIds: [],
    idSet: new Set(),
    expansion: new Map(),
    heights: new Map(),
    estimates: new Map(),
    scrollTop: 0,
    viewportPx: 0,
    scrollMarginPx: 0,
    overscan: DEFAULT_OVERSCAN,
});

export type AgentPaneLayoutCommand =
    // ── document shape ──────────────────────────────────────────────
    | { type: "NodesChanged"; orderedIds: ReadonlyArray<string> }
    // ── expansion (unified; replaces the scattered component signals) ─
    | { type: "UserExpanded"; nodeId: string }
    | { type: "UserCollapsed"; nodeId: string }
    | { type: "AutoExpandStarted"; nodeId: string }
    | { type: "AutoExpandHoldExpired"; nodeId: string }
    // Set a row's expansion to a fully-resolved value. The Phase-1 wiring
    // computes `currentExpansion(node, documentState)` (which already encodes
    // the pin > auto > default precedence per kind) and pushes it here, so the
    // slice mirrors the rendered open/closed state from one source. The
    // semantic commands above remain for Phase-2 transient timing (the hold).
    | { type: "ExpansionResolved"; nodeId: string; to: Expansion }
    // ── measurement ingest (normalized ÷zoom at the boundary) ───────
    | { type: "RowMeasured"; nodeId: string; state: ExpansionState; cssPx: number }
    | { type: "EstimateSet"; nodeId: string; state: ExpansionState; cssPx: number }
    | { type: "MeasurementInvalidated"; nodeId: string }
    // ── viewport / zoom ─────────────────────────────────────────────
    | { type: "Scrolled"; scrollTop: number; viewportPx: number }
    | { type: "ScrollMarginChanged"; px: number }
    | { type: "ZoomChanged"; zoom: number };

export type AgentPaneLayoutEvent =
    // `delta` is SLOT-SPECIFIC: `cssPx - priorHeightFor(nodeId, state)` for the
    // measured `state` only. It equals the row's change in prefix-sum
    // contribution ONLY when `state === inFlowState(expansion.get(nodeId))`
    // (i.e. the measured slot is the one currently rendered). A consumer must
    // NOT add `delta` to `totalSize` unconditionally — recompute from the
    // selectors, or gate on the current in-flow state first.
    | { type: "row-measured"; nodeId: string; state: ExpansionState; delta: number }
    | { type: "expansion-changed"; nodeId: string; from: Expansion; to: Expansion }
    | { type: "measurement-invalidated"; nodeId: string }
    | { type: "zoom-changed-no-relayout"; zoom: number }
    | { type: "ids-pruned"; removed: number }
    | { type: "command-dropped"; reason: string };

export interface ReducerResult {
    state: AgentPaneLayoutState;
    events: AgentPaneLayoutEvent[];
}

/** Pure: current in-flow footprint of a row from its expansion value. */
export const inFlowState = (e: Expansion | undefined): ExpansionState =>
    e?.open ? "expanded" : "collapsed";

/** Pure: are two expansion values equal? */
export function expansionEq(a: Expansion, b: Expansion): boolean {
    if (!a.open && !b.open) return true;
    return a.open && b.open && a.via === b.via;
}
