// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import {
    dropEntry,
    emptyHistory,
    moveTo,
    PANE_HISTORY_CAP,
    locatePane,
    recordPane,
    replacePlace,
    samePlace,
    sameView,
    setViewAt,
    stepHistory,
    type PaneHistory,
    type PaneRef,
} from "./pane-history-core";

const ref = (tabId: string, blockId: string): PaneRef => ({ tabId, blockId });
const B = ref("tab1", "B");
const G = ref("tab4", "G");
const alive = (r: PaneRef): PaneRef | null => r;

function record(...refs: PaneRef[]): PaneHistory {
    return refs.reduce(recordPane, emptyHistory());
}

describe("locatePane", () => {
    const tabs = [
        { tabId: "tab1", blockIds: ["A", "B"] },
        { tabId: "tab2", blockIds: ["C"] },
        { tabId: "tab3", blockIds: null },
    ];

    it("keeps a pane where its tab still holds it", () => {
        expect(locatePane(ref("tab1", "B"), tabs)).toEqual(ref("tab1", "B"));
    });

    it("finds the tab a pane was moved to", () => {
        expect(locatePane(ref("tab1", "C"), tabs)).toEqual(ref("tab2", "C"));
    });

    it("finds a pane whose old tab was closed", () => {
        expect(locatePane(ref("tab9", "C"), tabs)).toEqual(ref("tab2", "C"));
    });

    it("says nothing about a pane that is in no tab of the window", () => {
        expect(locatePane(ref("tab1", "Z"), tabs)).toBeNull();
        expect(locatePane(ref("tab9", "Z"), tabs)).toBeNull();
    });

    it("trusts a tab that is not loaded, since selecting it settles the question", () => {
        expect(locatePane(ref("tab3", "X"), tabs)).toEqual(ref("tab3", "X"));
    });
});

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
        const live = (r: PaneRef) => (r.blockId !== "B" && r.blockId !== "G" ? r : null);

        const back = stepHistory(moveTo(h, h.entries[3]), -1, live);
        expect(back.target).toBe(h.entries[0]);
        expect(back.history.entries.map((e) => e.blockId)).toEqual(["A", "C"]);
        expect(back.history.index).toBe(1);

        const fwd = stepHistory(moveTo(h, h.entries[0]), 1, live);
        expect(fwd.target).toBe(h.entries[3]);
    });

    it("finds no target when everything that way is dead, and still prunes it", () => {
        const h = record(B, G);
        const r = stepHistory(h, -1, () => null);
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

    it("follows a pane moved to another tab, Back and Forward, and keeps the cursor on the current entry", () => {
        const A = ref("tab1", "A");
        const h = record(A, B, G);
        // B moved from tab1 to tab2.
        const where = (r: PaneRef): PaneRef | null => (r.blockId === "B" ? ref("tab2", "B") : r);
        const back = stepHistory(h, -1, where);
        expect(back.target).toEqual(ref("tab2", "B"));
        expect(back.history.entries).toEqual([A, ref("tab2", "B"), G]);
        expect(back.history.entries).toContain(back.target);
        expect(back.history.index).toBe(2);
        const atB = moveTo(back.history, back.target!);
        expect(atB.index).toBe(1);
        // Forward from A reaches the moved entry too.
        const fwd = stepHistory(moveTo(h, h.entries[0]), 1, where);
        expect(fwd.target).toEqual(ref("tab2", "B"));
        expect(fwd.history.entries.map((e) => e.tabId)).toEqual(["tab1", "tab2", "tab4"]);
        // An entry that did not move is left as it was, not copied.
        expect(stepHistory(h, -1, alive).history).toBe(h);
    });

    it("skips a closed pane but follows a moved one further along", () => {
        const A = ref("tab1", "A");
        const h = record(A, B, G);
        const r = stepHistory(h, -1, (e) => (e.blockId === "G" ? e : e.blockId === "B" ? null : ref("tab3", "A")));
        expect(r.target).toEqual(ref("tab3", "A"));
        expect(r.history.entries).toEqual([ref("tab3", "A"), G]);
        expect(r.history.index).toBe(1);
    });

    it("keeps the newest entries when over the cap", () => {
        let h = emptyHistory();
        for (let i = 0; i < PANE_HISTORY_CAP + 5; i++) h = recordPane(h, ref("t", `b${i}`));
        expect(h.entries).toHaveLength(PANE_HISTORY_CAP);
        expect(h.entries[0].blockId).toBe("b5");
        expect(h.entries[h.index].blockId).toBe(`b${PANE_HISTORY_CAP + 4}`);
    });
});

// The trail of places: a pane and where its view stands inside it (SPEC_BACK_FORWARD_FOLLOW_THE_WHOLE_TRAIL_2026_11_26.md).
describe("the trail of places", () => {
    const K = (task?: string, board = "work"): PaneRef => ({ tabId: "tab1", blockId: "K", view: task ? { board, task } : { board } });
    const A = ref("tab1", "A");

    it("the operator's trail: pane A, a task in the Kanban, an agent's pane, pane B; Back walks it in reverse and Forward replays", () => {
        const agent = ref("tab1", "agent");
        const h = record(A, K("x"), agent, B);
        const names: string[] = [];
        let cur = h;
        for (let i = 0; i < 3; i++) {
            const r = stepHistory(cur, -1, alive);
            names.push(`${r.target?.blockId}:${JSON.stringify(r.target?.view ?? {})}`);
            cur = moveTo(r.history, r.target!);
        }
        expect(names).toEqual(['agent:{}', 'K:{"board":"work","task":"x"}', "A:{}"]);
        const fwd = stepHistory(cur, 1, alive);
        expect(fwd.target?.blockId).toBe("K");
    });

    it("another card in the same pane is a new place, the same card is not", () => {
        let h = record(A, K("x"));
        expect(recordPane(h, K("x"))).toBe(h);
        h = recordPane(h, K("y"));
        expect(h.entries.map((e) => e.view?.task)).toEqual([undefined, "x", "y"]);
    });

    it("compares places by their data, not by the order the keys were written in", () => {
        const a: PaneRef = { tabId: "t", blockId: "K", view: { board: "w", task: "x" } };
        const b: PaneRef = { tabId: "t", blockId: "K", view: { task: "x", board: "w" } };
        expect(samePlace(a, b)).toBe(true);
        expect(samePlace(a, { tabId: "t", blockId: "K" })).toBe(false);
        // A key with no value is no key; a pane with no place and one with an empty place are the same.
        expect(sameView({ task: undefined }, undefined)).toBe(true);
    });

    it("a place recorded after Back drops what was ahead, view places included", () => {
        let h = record(A, K("x"), B);
        const back = stepHistory(h, -1, alive);
        h = moveTo(back.history, back.target!);
        h = recordPane(h, K("z"));
        expect(h.entries.map((e) => e.view?.task ?? e.blockId)).toEqual(["A", "x", "z"]);
    });

    it("replacing stands for a push made a moment ago, and an entry that would repeat the one before goes", () => {
        let h = record(A, K("x"));
        h = replacePlace(h, K("y"));
        expect(h.entries.map((e) => e.view?.task ?? e.blockId)).toEqual(["A", "y"]);
        expect(replacePlace(h, K("y"))).toBe(h);
        // Replacing the current entry with the pane before it leaves one entry of it.
        const back = replacePlace(h, A);
        expect(back.entries).toEqual([A]);
        expect(back.index).toBe(0);
        expect(replacePlace(emptyHistory(), A).entries).toEqual([A]);
    });

    it("an entry keeps its place when its pane is followed to another tab", () => {
        const h = record(K("x"), B);
        const r = stepHistory(h, -1, (e) => (e.blockId === "K" ? ref("tab9", "K") : e));
        expect(r.target).toEqual({ tabId: "tab9", blockId: "K", view: { board: "work", task: "x" } });
    });

    it("a view place can be cleared when the view says it is gone, and read back after a restore", () => {
        let h = record(A, K("x"));
        h = setViewAt(h, 1, undefined);
        expect(h.entries[1].view).toBeUndefined();
        expect(setViewAt(h, 1, undefined)).toBe(h);
        h = setViewAt(h, 1, { board: "work", task: "q" });
        expect(h.entries[1].view).toEqual({ board: "work", task: "q" });
        expect(h.index).toBe(1);
    });

    it("when an agent moved the person (the trail did not record it), Back goes to the entry at the cursor first", () => {
        const h = record(A, B);
        const elsewhere = ref("tab1", "C");
        // The person is on C; the cursor entry is B: Back lands on B, not on A.
        expect(stepHistory(h, -1, alive, elsewhere).target).toEqual(B);
        // Forward from there goes to what is after the cursor: nothing here.
        expect(stepHistory(h, 1, alive, elsewhere).target).toBeNull();
        // Where the person is the cursor entry, Back is a step back as ever.
        expect(stepHistory(h, -1, alive, B).target).toEqual(A);
        // And with the place unknown, as before.
        expect(stepHistory(h, -1, alive, null).target).toEqual(A);
    });

    it("the cursor entry itself may be dead when an agent moved the person: it is removed and the cursor stays behind it", () => {
        const h = record(A, B);
        const r = stepHistory(h, -1, (e) => (e.blockId === "B" ? null : e), ref("tab1", "C"));
        expect(r.target).toEqual(A);
        expect(r.history.entries).toEqual([A]);
        expect(r.history.index).toBe(0);
    });
});
