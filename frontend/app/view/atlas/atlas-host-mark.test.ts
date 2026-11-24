// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { containerRing, ringVars } from "./atlas-host-mark";
import { gapsAt, MIN_SPREAD } from "./atlas-layout";

const SPREADS = Array.from({ length: 25 }, (_, i) => MIN_SPREAD + (i * (1 - MIN_SPREAD)) / 24);

/** What the stylesheet draws for a plain node (`--ring-gap`, `--ring-w`) and a container agent's (`containerRing`). */
function rings(spread: number) {
    const gaps = gapsAt(spread);
    const vars = ringVars(gaps);
    const gap = parseFloat(vars["--atlas-ring-gap"]);
    const width = parseFloat(vars["--atlas-ring-w"]);
    return { gaps, plain: gap + width, container: containerRing(gaps, gap, width) };
}

describe("the host ring of a node", () => {
    it("never reaches the ring of the node beside it: 1 px is left between them at every zoom", () => {
        for (const spread of SPREADS) {
            const { gaps, plain, container } = rings(spread);
            // Two neighbours' rings (the same kind, the worst case) leave the gap less their two reaches.
            expect(gaps.x - 2 * plain).toBeGreaterThanOrEqual(1);
            expect(gaps.x - 2 * (container.gap + container.width)).toBeGreaterThanOrEqual(1);
            // A container beside a plain node.
            expect(gaps.x - (plain + container.gap + container.width)).toBeGreaterThanOrEqual(1);
        }
    });

    it("stands a container's ring outside its dashed frame, and keeps it at least 1 px wide", () => {
        for (const spread of SPREADS) {
            const { gaps, container } = rings(spread);
            expect(container.gap).toBeGreaterThanOrEqual(gaps.frame - 2);
            expect(container.width).toBeGreaterThanOrEqual(1);
        }
    });

    it("is 1 px wide on a container at the default zoom, where the full 2 px ring met its neighbour's", () => {
        const { gaps, plain, container } = rings(1);
        expect(gaps.x).toBe(14);
        expect(container).toEqual({ gap: 5, width: 1 });
        // The plain node keeps the 2 px ring of the spec.
        expect(plain).toBe(4);
        expect(ringVars(gaps)["--atlas-ring-w"]).toBe("2px");
    });

    it("hands the stylesheet the container's gap and width", () => {
        const vars = ringVars(gapsAt(1));
        expect(vars["--atlas-ring-gap-container"]).toBe("5px");
        expect(vars["--atlas-ring-w-container"]).toBe("1px");
    });
});
