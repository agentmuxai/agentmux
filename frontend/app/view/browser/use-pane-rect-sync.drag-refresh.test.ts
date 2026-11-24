// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { createDragHold } from "./use-pane-rect-sync";

const at = (w: number, h: number) => ({ x: 5, y: 6, width: w, height: h });

describe("createDragHold with the drag's refresh clock", () => {
    it("lets the page catch up with its pane on each tick, and holds between ticks", () => {
        let dragging = true;
        let tick = 0;
        const hold = createDragHold(() => dragging, () => tick);
        expect(hold.apply(at(800, 600))).toEqual(at(800, 600));
        expect(hold.apply(at(900, 600))).toEqual(at(800, 600)); // grown, not yet ticked: held
        tick++;
        expect(hold.apply(at(950, 600))).toEqual(at(950, 600)); // the tick: the page takes the pane's size
        expect(hold.apply(at(990, 600))).toEqual(at(950, 600)); // held again until the next
        expect(hold.sizeCss({ width: 990, height: 600 }, 1)).toEqual({ w: 950, h: 600 });
        dragging = false;
        expect(hold.apply(at(990, 600))).toEqual(at(990, 600));
    });

    it("still follows a pane that shrinks, tick or no tick", () => {
        let tick = 0;
        const hold = createDragHold(() => true, () => tick);
        hold.apply(at(800, 600));
        expect(hold.apply(at(500, 600))).toEqual(at(500, 600));
        tick++;
        expect(hold.apply(at(450, 600))).toEqual(at(450, 600));
    });

    it("holds for the whole drag when there is no clock, as before", () => {
        const hold = createDragHold(() => true);
        hold.apply(at(800, 600));
        expect(hold.apply(at(1200, 900))).toEqual(at(800, 600));
    });
});
