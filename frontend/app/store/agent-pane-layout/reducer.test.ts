// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import {
    effectiveHeight,
    positions,
    totalSize,
    update,
    windowRange,
} from "./reducer";
import {
    AgentPaneLayoutCommand,
    AgentPaneLayoutState,
    DEFAULT_ROW_PX,
    initialState,
} from "./types";

const apply = (
    s: AgentPaneLayoutState,
    cmd: AgentPaneLayoutCommand,
): AgentPaneLayoutState => update(s, cmd).state;

const applyAll = (
    s: AgentPaneLayoutState,
    cmds: AgentPaneLayoutCommand[],
): AgentPaneLayoutState => cmds.reduce(apply, s);

/** Tiny deterministic LCG so property-test failures reproduce. */
function rng(seed: number): () => number {
    let x = seed >>> 0;
    return () => {
        // numerical recipes LCG
        x = (1664525 * x + 1013904223) >>> 0;
        return x / 0x100000000;
    };
}

describe("agent-pane-layout reducer", () => {
    describe("INV-1 — positions are a flush, non-overlapping prefix-sum", () => {
        const assertPrefixSum = (s: AgentPaneLayoutState): void => {
            const pos = positions(s);
            expect(pos.length).toBe(s.orderedIds.length);
            let cursor = s.scrollMarginPx;
            for (let i = 0; i < pos.length; i++) {
                const p = pos[i];
                // finite, non-negative heights (guarded ingest)
                expect(Number.isFinite(p.height)).toBe(true);
                expect(p.height).toBeGreaterThanOrEqual(0);
                // slot is self-consistent
                expect(p.start).toBe(cursor);
                expect(p.end).toBe(p.start + p.height);
                // flush with the previous row → no overlap, no gap
                if (i > 0) expect(p.start).toBe(pos[i - 1].end);
                cursor = p.end;
            }
            // totalSize equals the accumulated height (independent of margin)
            expect(totalSize(s)).toBe(cursor - s.scrollMarginPx);
        };

        it("holds for a hand-built mixed state", () => {
            let s = initialState();
            s = apply(s, { type: "NodesChanged", orderedIds: ["a", "b", "c", "d"] });
            s = apply(s, { type: "ScrollMarginChanged", px: 12 });
            s = apply(s, { type: "RowMeasured", nodeId: "a", state: "collapsed", cssPx: 23 });
            s = apply(s, { type: "EstimateSet", nodeId: "b", state: "collapsed", cssPx: 40 });
            s = apply(s, { type: "UserExpanded", nodeId: "c" });
            s = apply(s, { type: "RowMeasured", nodeId: "c", state: "expanded", cssPx: 180 });
            // d: no measurement, no estimate → DEFAULT_ROW_PX
            assertPrefixSum(s);
            const pos = positions(s);
            expect(pos.map((p) => p.height)).toEqual([23, 40, 180, DEFAULT_ROW_PX]);
            expect(pos[0].start).toBe(12); // scrollMargin
        });

        it("holds across ANY random command sequence (property test)", () => {
            const ids = ["n0", "n1", "n2", "n3", "n4", "n5"];
            for (let seed = 1; seed <= 40; seed++) {
                const rand = rng(seed);
                const pick = <T>(arr: T[]): T => arr[Math.floor(rand() * arr.length)];
                let s = initialState();
                let measuresApplied = 0;
                s = apply(s, { type: "NodesChanged", orderedIds: ids });

                for (let step = 0; step < 120; step++) {
                    const k = Math.floor(rand() * 11);
                    const id = pick(ids);
                    const st = rand() < 0.5 ? "collapsed" : "expanded";
                    let cmd: AgentPaneLayoutCommand;
                    switch (k) {
                        case 0:
                            cmd = { type: "RowMeasured", nodeId: id, state: st, cssPx: Math.floor(rand() * 400) };
                            break;
                        case 1:
                            cmd = { type: "EstimateSet", nodeId: id, state: st, cssPx: Math.floor(rand() * 400) };
                            break;
                        case 2:
                            cmd = { type: "UserExpanded", nodeId: id };
                            break;
                        case 3:
                            cmd = { type: "UserCollapsed", nodeId: id };
                            break;
                        case 4:
                            cmd = { type: "AutoExpandStarted", nodeId: id };
                            break;
                        case 5:
                            cmd = { type: "AutoExpandHoldExpired", nodeId: id };
                            break;
                        case 6:
                            cmd = { type: "MeasurementInvalidated", nodeId: id };
                            break;
                        case 7:
                            cmd = { type: "Scrolled", scrollTop: Math.floor(rand() * 2000), viewportPx: Math.floor(rand() * 800) };
                            break;
                        case 8:
                            cmd = { type: "ScrollMarginChanged", px: Math.floor(rand() * 60) };
                            break;
                        case 9:
                            cmd = { type: "ZoomChanged", zoom: 0.5 + rand() * 1.5 };
                            break;
                        default: {
                            // occasionally mutate the id set (prepend/truncate)
                            const cut = Math.floor(rand() * ids.length);
                            cmd = { type: "NodesChanged", orderedIds: ids.slice(cut) };
                        }
                    }
                    if (cmd.type === "RowMeasured" || cmd.type === "EstimateSet") measuresApplied++;
                    s = apply(s, cmd);
                    assertPrefixSum(s);
                    // window range is always a valid (possibly empty) slice
                    const w = windowRange(s);
                    expect(w.startIndex).toBeGreaterThanOrEqual(0);
                    expect(w.endIndex).toBeLessThan(s.orderedIds.length);
                }
                // anti-vacuity: the sequence actually exercised measurement
                expect(measuresApplied).toBeGreaterThan(0);
            }
        });

        it("rejects non-finite / negative measurements (keeps the sum clean)", () => {
            let s = initialState();
            s = apply(s, { type: "NodesChanged", orderedIds: ["a"] });
            for (const bad of [NaN, Infinity, -Infinity, -5]) {
                const r = update(s, { type: "RowMeasured", nodeId: "a", state: "collapsed", cssPx: bad });
                expect(r.state).toBe(s); // dropped, same ref
                expect(r.events[0]).toMatchObject({ type: "command-dropped" });
            }
            expect(effectiveHeight(s, "a")).toBe(DEFAULT_ROW_PX);
        });
    });

    describe("INV-2 — zoom is layout-invariant", () => {
        it("ZoomChanged does not alter positions / heights / estimates", () => {
            let s = initialState();
            s = apply(s, { type: "NodesChanged", orderedIds: ["a", "b"] });
            s = apply(s, { type: "RowMeasured", nodeId: "a", state: "collapsed", cssPx: 30 });
            s = apply(s, { type: "RowMeasured", nodeId: "b", state: "collapsed", cssPx: 50 });
            const before = positions(s);
            const heightsRef = s.heights;
            const estimatesRef = s.estimates;
            const idsRef = s.orderedIds;

            const r = update(s, { type: "ZoomChanged", zoom: 0.5 });
            expect(r.events[0]).toMatchObject({ type: "zoom-changed-no-relayout", zoom: 0.5 });
            // layout inputs untouched (same references)
            expect(r.state.heights).toBe(heightsRef);
            expect(r.state.estimates).toBe(estimatesRef);
            expect(r.state.orderedIds).toBe(idsRef);
            // positions identical
            expect(positions(r.state)).toEqual(before);
        });

        it("ZoomChanged to the same value is a no-op (same ref)", () => {
            const s = initialState();
            expect(update(s, { type: "ZoomChanged", zoom: 1 }).state).toBe(s);
        });
    });

    describe("INV-3 — measurements are keyed by expansion state", () => {
        it("an expanded measurement never contaminates the collapsed slot", () => {
            let s = initialState();
            s = apply(s, { type: "NodesChanged", orderedIds: ["x"] });
            s = apply(s, { type: "RowMeasured", nodeId: "x", state: "collapsed", cssPx: 24 });
            expect(effectiveHeight(s, "x")).toBe(24); // collapsed by default

            s = apply(s, { type: "UserExpanded", nodeId: "x" });
            // expanded slot unmeasured → falls back to default
            expect(effectiveHeight(s, "x")).toBe(DEFAULT_ROW_PX);

            s = apply(s, { type: "RowMeasured", nodeId: "x", state: "expanded", cssPx: 200 });
            expect(effectiveHeight(s, "x")).toBe(200);

            // collapse again → original collapsed measurement intact
            s = apply(s, { type: "UserCollapsed", nodeId: "x" });
            expect(effectiveHeight(s, "x")).toBe(24);
        });

        it("effectiveHeight precedence: measured > estimate > default", () => {
            let s = initialState();
            s = apply(s, { type: "NodesChanged", orderedIds: ["x"] });
            expect(effectiveHeight(s, "x")).toBe(DEFAULT_ROW_PX);
            s = apply(s, { type: "EstimateSet", nodeId: "x", state: "collapsed", cssPx: 60 });
            expect(effectiveHeight(s, "x")).toBe(60);
            s = apply(s, { type: "RowMeasured", nodeId: "x", state: "collapsed", cssPx: 47 });
            expect(effectiveHeight(s, "x")).toBe(47);
        });
    });

    describe("expansion semantics — pin outranks auto; hold expiry", () => {
        it("AutoExpandStarted then AutoExpandHoldExpired collapses", () => {
            let s = initialState();
            s = apply(s, { type: "AutoExpandStarted", nodeId: "t" });
            expect(s.expansion.get("t")).toEqual({ open: true, via: "auto" });
            const r = update(s, { type: "AutoExpandHoldExpired", nodeId: "t" });
            expect(r.state.expansion.has("t")).toBe(false); // collapsed === absent
            expect(r.events[0]).toMatchObject({ type: "expansion-changed" });
        });

        it("a user pin survives a hold expiry (no-op, audited)", () => {
            let s = initialState();
            s = apply(s, { type: "AutoExpandStarted", nodeId: "t" });
            s = apply(s, { type: "UserExpanded", nodeId: "t" }); // pin during hold
            expect(s.expansion.get("t")).toEqual({ open: true, via: "pin" });
            const r = update(s, { type: "AutoExpandHoldExpired", nodeId: "t" });
            expect(r.state).toBe(s); // unchanged
            expect(r.events[0]).toMatchObject({ type: "command-dropped", reason: "hold-expired-not-auto" });
        });

        it("AutoExpandStarted does not downgrade an existing pin", () => {
            let s = initialState();
            s = apply(s, { type: "UserExpanded", nodeId: "t" });
            const r = update(s, { type: "AutoExpandStarted", nodeId: "t" });
            expect(r.state).toBe(s);
            expect(s.expansion.get("t")).toEqual({ open: true, via: "pin" });
        });
    });

    describe("NodesChanged — pruning by set membership", () => {
        it("prunes heights/estimates/expansion for removed ids, keeps survivors", () => {
            let s = initialState();
            s = apply(s, { type: "NodesChanged", orderedIds: ["a", "b", "c"] });
            s = apply(s, { type: "RowMeasured", nodeId: "a", state: "collapsed", cssPx: 10 });
            s = apply(s, { type: "RowMeasured", nodeId: "b", state: "collapsed", cssPx: 20 });
            s = apply(s, { type: "UserExpanded", nodeId: "b" });
            s = apply(s, { type: "EstimateSet", nodeId: "c", state: "collapsed", cssPx: 30 });

            const r = update(s, { type: "NodesChanged", orderedIds: ["a", "c"] });
            expect(r.events).toContainEqual({ type: "ids-pruned", removed: 1 });
            // b gone from every map
            expect(r.state.heights.has("b")).toBe(false);
            expect(r.state.expansion.has("b")).toBe(false);
            // survivors keep their measurements
            expect(effectiveHeight(r.state, "a")).toBe(10);
            expect(effectiveHeight(r.state, "c")).toBe(30);
        });

        it("identical id list is a no-op (same ref)", () => {
            let s = initialState();
            s = apply(s, { type: "NodesChanged", orderedIds: ["a", "b"] });
            expect(update(s, { type: "NodesChanged", orderedIds: ["a", "b"] }).state).toBe(s);
        });

        it("a measured row that scrolls out (stays in orderedIds) keeps its height", () => {
            // models the stuck-estimate-after-recycle bug being structurally fixed:
            // scroll-out doesn't change orderedIds, so heights survive.
            let s = initialState();
            s = apply(s, { type: "NodesChanged", orderedIds: ["a", "b", "c"] });
            s = apply(s, { type: "RowMeasured", nodeId: "a", state: "collapsed", cssPx: 18 });
            s = apply(s, { type: "Scrolled", scrollTop: 5000, viewportPx: 400 }); // a is far off-screen
            expect(effectiveHeight(s, "a")).toBe(18); // not reset to estimate/default
        });
    });

    describe("MeasurementInvalidated — content growth forces a clean re-measure", () => {
        it("clears the whole RowHeight for the node", () => {
            let s = initialState();
            s = apply(s, { type: "NodesChanged", orderedIds: ["a"] });
            s = apply(s, { type: "RowMeasured", nodeId: "a", state: "collapsed", cssPx: 24 });
            s = apply(s, { type: "RowMeasured", nodeId: "a", state: "expanded", cssPx: 200 });
            const r = update(s, { type: "MeasurementInvalidated", nodeId: "a" });
            expect(r.state.heights.has("a")).toBe(false);
            expect(r.events[0]).toMatchObject({ type: "measurement-invalidated", nodeId: "a" });
            // invalidating an unmeasured node is a no-op (same ref)
            expect(update(r.state, { type: "MeasurementInvalidated", nodeId: "a" }).state).toBe(r.state);
        });
    });

    describe("windowRange — binary search over the prefix sum", () => {
        const build = (): AgentPaneLayoutState => {
            let s = initialState();
            const ids = Array.from({ length: 20 }, (_, i) => `r${i}`);
            s = apply(s, { type: "NodesChanged", orderedIds: ids });
            // each row 100px tall (collapsed)
            for (const id of ids) {
                s = apply(s, { type: "RowMeasured", nodeId: id, state: "collapsed", cssPx: 100 });
            }
            return apply(s, { type: "ScrollMarginChanged", px: 0 });
        };

        it("returns the rows overlapping the viewport, padded by overscan", () => {
            let s = build(); // 20 rows × 100px = 2000px; overscan default 5
            s = apply(s, { type: "Scrolled", scrollTop: 1000, viewportPx: 300 }); // rows 10..12 visible
            const w = windowRange(s);
            // first visible = 10, last visible = 12; ±5 overscan, clamped
            expect(w.startIndex).toBe(5);
            expect(w.endIndex).toBe(17);
        });

        it("empty region → empty range", () => {
            const s = initialState();
            expect(windowRange(s)).toEqual({ startIndex: 0, endIndex: -1 });
        });

        it("scrolled past the end → empty range", () => {
            let s = build();
            s = apply(s, { type: "Scrolled", scrollTop: 9000, viewportPx: 300 });
            expect(windowRange(s).endIndex).toBeLessThan(windowRange(s).startIndex);
        });
    });

    describe("no-op short-circuits return the same state reference", () => {
        it("Scrolled / ScrollMarginChanged / RowMeasured idempotence", () => {
            let s = initialState();
            s = apply(s, { type: "NodesChanged", orderedIds: ["a"] });
            s = apply(s, { type: "Scrolled", scrollTop: 10, viewportPx: 100 });
            expect(update(s, { type: "Scrolled", scrollTop: 10, viewportPx: 100 }).state).toBe(s);
            s = apply(s, { type: "RowMeasured", nodeId: "a", state: "collapsed", cssPx: 24 });
            expect(update(s, { type: "RowMeasured", nodeId: "a", state: "collapsed", cssPx: 24 }).state).toBe(s);
            expect(update(s, { type: "UserCollapsed", nodeId: "a" }).state).toBe(s); // already collapsed
        });
    });
});
