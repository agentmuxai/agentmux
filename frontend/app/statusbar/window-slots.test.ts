// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it, vi } from "vitest";
import { closedWindowLabel, createWindowSlots, orderBySlots } from "./window-slots";

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

describe("createWindowSlots", () => {
    it("is not ready until srv has answered, and then orders by srv's list", async () => {
        const slots = createWindowSlots();
        expect(slots.ready()).toBe(false);
        await slots.sync(["a", "b", "c"], async () => ["c", "a", "b"]);
        expect(slots.ready()).toBe(true);
        expect(slots.order(["a", "b", "c"], id)).toEqual(["c", "a", "b"]);
    });

    it("puts a window srv has not placed yet last, and follows a gap srv closed", async () => {
        const slots = createWindowSlots();
        await slots.sync(["a", "b", "c"], async () => ["a", "b", "c"]);
        expect(slots.order(["a", "z", "b", "c"], id)).toEqual(["a", "b", "c", "z"]);
        await slots.sync(["a", "c"], async () => ["a", "c"]);
        expect(slots.order(["c", "a"], id)).toEqual(["a", "c"]);
    });

    it("moves a stand-in key's slot to the real key", async () => {
        const slots = createWindowSlots();
        await slots.sync(["a"], async () => ["a"]);
        slots.order(["a", "label:main"], id);
        slots.adopt("label:main", "ws-1");
        slots.adopt("label:main", "ws-1");
        expect(slots.order(["ws-1", "a"], id)).toEqual(["a", "ws-1"]);
    });

    it("says what srv has not been told about, until it has", async () => {
        const slots = createWindowSlots();
        expect(slots.needsSync(["a"])).toBe(true);
        await slots.sync(["a"], async () => ["a"]);
        expect(slots.needsSync(["a"])).toBe(false);
        expect(slots.needsSync(["a", "b"])).toBe(true);
    });

    it("sends nothing for a stand-in alone, and is ready after a failed ask", async () => {
        const slots = createWindowSlots();
        const place = vi.fn(async (k: string[]) => k);
        await slots.sync(["label:main"], place);
        expect(place).not.toHaveBeenCalled();
        await expect(slots.sync(["a"], async () => Promise.reject(new Error("down")))).rejects.toThrow("down");
        expect(slots.ready()).toBe(true);
    });

    it("two panels given the same srv answer draw the same order, whatever they saw first", async () => {
        const first = createWindowSlots();
        const second = createWindowSlots();
        // the first saw the windows open first and the closed one last; the second loaded later in another order
        first.order(["a", "c", "b"], id);
        second.order(["b", "c", "a"], id);
        const srv = async () => ["a", "b", "c"];
        await first.sync(["a", "c", "b"], srv);
        await second.sync(["b", "c", "a"], srv);
        expect(first.order(["a", "c", "b"], id)).toEqual(["a", "b", "c"]);
        expect(second.order(["b", "c", "a"], id)).toEqual(["a", "b", "c"]);
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
