// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import {
    dropEntry,
    emptyHistory,
    moveTo,
    PANE_HISTORY_CAP,
    recordPane,
    stepHistory,
    type PaneHistory,
    type PaneRef,
} from "./pane-history-core";

const ref = (tabId: string, blockId: string): PaneRef => ({ tabId, blockId });
const B = ref("tab1", "B");
const G = ref("tab4", "G");
const alive = () => true;

function record(...refs: PaneRef[]): PaneHistory {
    return refs.reduce(recordPane, emptyHistory());
}

describe("pane history", () => {
    it("goes Back to B and Forward to G, across tabs", () => {
        let h = record(B, G);
        expect(h.index).toBe(1);

        const back = stepHistory(h, -1, alive);
        expect(back.target).toEqual(B);
        h = moveTo(back.history, back.target!);
        expect(h.index).toBe(0);

        const fwd = stepHistory(h, 1, alive);
        expect(fwd.target).toEqual(G);
        h = moveTo(fwd.history, fwd.target!);
        expect(h.index).toBe(1);
    });

    it("has nowhere to go past either end", () => {
        const h = record(B, G);
        expect(stepHistory(h, 1, alive).target).toBeNull();
        expect(stepHistory(moveTo(h, h.entries[0]), -1, alive).target).toBeNull();
        expect(stepHistory(emptyHistory(), -1, alive).target).toBeNull();
    });

    it("drops what was ahead when a new pane is selected after going Back", () => {
        let h = record(B, G, ref("tab4", "H"));
        h = moveTo(h, h.entries[0]);
        h = recordPane(h, ref("tab2", "C"));
        expect(h.entries.map((e) => e.blockId)).toEqual(["B", "C"]);
        expect(h.index).toBe(1);
    });

    it("collapses a pane selected twice in a row, but not B, G, B", () => {
        expect(record(B, B).entries).toHaveLength(1);
        expect(record(B, G, B).entries).toHaveLength(3);
    });

    it("treats the same block in another tab as another pane", () => {
        expect(record(ref("tab1", "B"), ref("tab2", "B")).entries).toHaveLength(2);
    });

    it("skips entries that no longer exist, in both directions, and removes them", () => {
        const A = ref("tab1", "A");
        const C = ref("tab2", "C");
        const h = record(A, B, G, C);
        const live = (r: PaneRef) => r.blockId !== "B" && r.blockId !== "G";

        const back = stepHistory(moveTo(h, h.entries[3]), -1, live);
        expect(back.target).toBe(h.entries[0]);
        expect(back.history.entries.map((e) => e.blockId)).toEqual(["A", "C"]);
        expect(back.history.index).toBe(1);

        const fwd = stepHistory(moveTo(h, h.entries[0]), 1, live);
        expect(fwd.target).toBe(h.entries[3]);
    });

    it("finds no target when everything that way is dead, and still prunes it", () => {
        const h = record(B, G);
        const r = stepHistory(h, -1, () => false);
        expect(r.target).toBeNull();
        expect(r.history.entries).toEqual([G]);
        expect(r.history.index).toBe(0);
    });

    it("forgets a target that couldn't be selected, keeping the cursor on the current entry", () => {
        const h = record(B, G, ref("tab4", "H"));
        const here = moveTo(h, h.entries[1]);
        const back = stepHistory(here, -1, alive);
        const dropped = dropEntry(here, back.target!);
        expect(dropped.entries.map((e) => e.blockId)).toEqual(["G", "H"]);
        expect(dropped.entries[dropped.index]).toEqual(G);
        const fwd = dropEntry(here, here.entries[2]);
        expect(fwd.entries[fwd.index]).toEqual(G);
    });

    it("keeps the newest entries when over the cap", () => {
        let h = emptyHistory();
        for (let i = 0; i < PANE_HISTORY_CAP + 5; i++) h = recordPane(h, ref("t", `b${i}`));
        expect(h.entries).toHaveLength(PANE_HISTORY_CAP);
        expect(h.entries[0].blockId).toBe("b5");
        expect(h.entries[h.index].blockId).toBe(`b${PANE_HISTORY_CAP + 4}`);
    });
});
