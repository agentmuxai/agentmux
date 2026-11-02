// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { beforeEach, describe, expect, it } from "vitest";
import { adoptSlot, closedWindowLabel, inSlotOrder, orderBySlots, resetWindowSlots } from "./window-slots";

const id = (s: string) => s;

describe("orderBySlots", () => {
    it("places a first-seen set in the order given", () => {
        expect(orderBySlots(["a", "b", "c"], id, [])).toEqual({ rows: ["a", "b", "c"], slots: ["a", "b", "c"] });
    });

    it("keeps each window in its slot whatever order the rows come in", () => {
        const first = orderBySlots(["a", "b", "c"], id, []);
        // b closed: the lists now give open windows first, then the closed one
        expect(orderBySlots(["a", "c", "b"], id, first.slots).rows).toEqual(["a", "b", "c"]);
    });

    it("puts a window met later after the ones already placed", () => {
        const first = orderBySlots(["a", "b"], id, []);
        expect(orderBySlots(["z", "a", "b"], id, first.slots).rows).toEqual(["a", "b", "z"]);
    });

    it("lists a window once even when two rows have its key", () => {
        expect(orderBySlots(["a", "a", "b"], id, []).rows).toEqual(["a", "b"]);
    });

    it("lets a window that is not listed now leave a gap, and keeps its slot for when it returns", () => {
        const first = orderBySlots(["a", "b", "c"], id, []);
        const without = orderBySlots(["a", "c"], id, first.slots);
        expect(without.rows).toEqual(["a", "c"]);
        expect(orderBySlots(["c", "b", "a"], id, without.slots).rows).toEqual(["a", "b", "c"]);
    });

    it("drops the oldest unlisted slots past the cap, never a listed window", () => {
        const many = Array.from({ length: 205 }, (_, i) => `w${i}`);
        const placed = orderBySlots(many, id, []);
        const later = orderBySlots(["w204", "w0"], id, placed.slots);
        expect(later.slots.length).toBeLessThanOrEqual(200);
        expect(later.rows).toEqual(["w0", "w204"]);
    });
});

describe("inSlotOrder", () => {
    beforeEach(resetWindowSlots);

    it("remembers the slots between calls", () => {
        expect(inSlotOrder(["a", "b", "c"], id)).toEqual(["a", "b", "c"]);
        expect(inSlotOrder(["c", "a"], id)).toEqual(["a", "c"]);
        expect(inSlotOrder(["b", "c", "a"], id)).toEqual(["a", "b", "c"]);
    });

    it("gives the same order for the same rows", () => {
        const rows = ["x", "y", "z"];
        expect(inSlotOrder(rows, id)).toEqual(inSlotOrder(rows, id));
    });
});

describe("adoptSlot", () => {
    beforeEach(resetWindowSlots);

    it("moves a stand-in key's slot to the real key", () => {
        inSlotOrder(["label:main", "label:w2", "ws-9"], id);
        adoptSlot("label:main", "ws-1");
        adoptSlot("label:w2", "ws-2");
        expect(inSlotOrder(["ws-9", "ws-2", "ws-1"], id)).toEqual(["ws-1", "ws-2", "ws-9"]);
    });

    it("does nothing when the real key already has a slot or the stand-in has none", () => {
        inSlotOrder(["ws-1", "label:main"], id);
        adoptSlot("label:main", "ws-1");
        adoptSlot("label:other", "ws-5");
        expect(inSlotOrder(["label:main", "ws-5", "ws-1"], id)).toEqual(["ws-1", "label:main", "ws-5"]);
    });
});

describe("closedWindowLabel", () => {
    it("reads the name and how many agents run, as in Wizzy · 4 agents running", () => {
        expect(closedWindowLabel("Wizzy", 4)).toBe("Wizzy · 4 agents running");
        expect(closedWindowLabel("Wizzy", 1)).toBe("Wizzy · 1 agent running");
    });

    it("is just the name when none run, and Window when it has none", () => {
        expect(closedWindowLabel("Wizzy", 0)).toBe("Wizzy");
        expect(closedWindowLabel("  ", 0)).toBe("Window");
        expect(closedWindowLabel("", 2)).toBe("Window · 2 agents running");
    });
});
