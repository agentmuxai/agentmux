// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createComputed, createRoot } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { SWARM_SETTLE_MS, SwarmOrderController, orderSwarmItems, type SwarmOrderItem } from "./swarm-order";

const local = (key: string, o: Partial<SwarmOrderItem> = {}): SwarmOrderItem => ({
    key,
    remote: false,
    working: false,
    lastActive: 0,
    name: key,
    ...o,
});
const remote = (key: string, working = false): SwarmOrderItem => ({
    key,
    remote: true,
    working,
    lastActive: 0,
    name: key,
});

describe("orderSwarmItems", () => {
    it("puts working agents above idle ones, and idle ones by most recent activity", () => {
        const items = [
            local("idle-old", { lastActive: 100 }),
            local("busy", { working: true, lastActive: 50 }),
            local("idle-new", { lastActive: 900 }),
        ];
        expect(orderSwarmItems(items, new Map([["busy", 1]]))).toEqual(["busy", "idle-new", "idle-old"]);
    });

    it("puts the agent that started working most recently first among the working ones", () => {
        const items = [
            local("a", { working: true, lastActive: 999 }),
            local("b", { working: true, lastActive: 1 }),
            local("c", { working: true, lastActive: 500 }),
        ];
        const since = new Map([
            ["a", 100],
            ["b", 300],
            ["c", 200],
        ]);
        expect(orderSwarmItems(items, since)).toEqual(["b", "c", "a"]);
    });

    it("puts an agent that never produced output last among the idle ones", () => {
        const items = [local("never"), local("seen", { lastActive: 5 })];
        expect(orderSwarmItems(items, new Map())).toEqual(["seen", "never"]);
    });

    it("breaks ties on name, then key, so the order is total", () => {
        const items = [
            local("k3", { name: "bravo", lastActive: 10 }),
            local("k2", { name: "Alpha", lastActive: 10 }),
            local("k1", { name: "alpha", lastActive: 10 }),
        ];
        expect(orderSwarmItems(items, new Map())).toEqual(["k1", "k2", "k3"]);
        expect(orderSwarmItems([...items].reverse(), new Map())).toEqual(["k1", "k2", "k3"]);
    });

    it("lists working remote agents after the local working ones and before every idle one", () => {
        const items = [
            local("idle", { lastActive: 10 }),
            remote("r-idle"),
            remote("r-busy", true),
            local("busy", { working: true }),
        ];
        expect(orderSwarmItems(items, new Map([["busy", 1]]))).toEqual(["busy", "r-busy", "idle", "r-idle"]);
    });

    it("keeps remote agents in the order they came in", () => {
        const items = [remote("r2"), remote("r1"), remote("r3")];
        expect(orderSwarmItems(items, new Map())).toEqual(["r2", "r1", "r3"]);
    });
});

describe("SwarmOrderController", () => {
    beforeEach(() => vi.useFakeTimers());
    afterEach(() => vi.useRealTimers());

    function make(now = () => Date.now(), stillHeld?: (hold: "pointer" | "focus") => boolean) {
        const moves: string[] = [];
        const ctl = new SwarmOrderController(SWARM_SETTLE_MS, now, {
            beforeMove: () => moves.push("before"),
            afterMove: () => moves.push("after"),
            stillHeld,
        });
        return { ctl, moves };
    }

    it("shows the first list at once", () => {
        const { ctl, moves } = make();
        ctl.update([local("a", { lastActive: 1 }), local("b", { lastActive: 2 })]);
        expect(ctl.order()).toEqual(["b", "a"]);
        expect(moves).toEqual([]);
    });

    it("moves rows only after the settle time, once for a burst of changes", () => {
        const { ctl, moves } = make();
        const a = local("a", { lastActive: 2 });
        const busy = local("b", { working: true, lastActive: 3 });
        ctl.update([a, local("b", { lastActive: 1 })]);
        ctl.update([a, busy]);
        expect(ctl.order()).toEqual(["a", "b"]);
        // A change just before the time is up starts the wait again.
        vi.advanceTimersByTime(SWARM_SETTLE_MS - 1);
        ctl.update([a, busy, local("c", { lastActive: 5 })]);
        vi.advanceTimersByTime(SWARM_SETTLE_MS - 1);
        expect(ctl.order()).toEqual(["a", "b", "c"]);
        vi.advanceTimersByTime(1);
        expect(ctl.order()).toEqual(["b", "c", "a"]);
        expect(moves).toEqual(["before", "after"]);
    });

    it("does not start the wait again for a change that leaves the order as it was", () => {
        const { ctl } = make();
        const a = local("a", { lastActive: 2 });
        ctl.update([a, local("b", { lastActive: 1 })]);
        ctl.update([a, local("b", { working: true, lastActive: 3 })]);
        vi.advanceTimersByTime(SWARM_SETTLE_MS - 1);
        // Output from the working agent changes its last activity, not the order.
        ctl.update([a, local("b", { working: true, lastActive: 4 })]);
        vi.advanceTimersByTime(1);
        expect(ctl.order()).toEqual(["b", "a"]);
    });

    it("does not move rows while a hold is on, and moves them once it ends and the settle time passes", () => {
        const { ctl } = make();
        ctl.update([local("a", { lastActive: 2 }), local("b", { lastActive: 1 })]);
        ctl.setHold("pointer", true);
        ctl.update([local("a", { lastActive: 2 }), local("b", { working: true, lastActive: 3 })]);
        vi.advanceTimersByTime(60_000);
        expect(ctl.order()).toEqual(["a", "b"]);
        ctl.setHold("pointer", false);
        vi.advanceTimersByTime(SWARM_SETTLE_MS - 1);
        expect(ctl.order()).toEqual(["a", "b"]);
        vi.advanceTimersByTime(1);
        expect(ctl.order()).toEqual(["b", "a"]);
    });

    it("holds for each reason until the last one ends", () => {
        const { ctl } = make();
        ctl.update([local("a", { lastActive: 2 }), local("b", { lastActive: 1 })]);
        for (const hold of ["pointer", "press", "focus"] as const) ctl.setHold(hold, true);
        ctl.update([local("a", { lastActive: 2 }), local("b", { working: true, lastActive: 3 })]);
        ctl.setHold("pointer", false);
        ctl.setHold("press", false);
        vi.advanceTimersByTime(10_000);
        expect(ctl.order()).toEqual(["a", "b"]);
        expect(ctl.held).toBe(true);
        ctl.setHold("focus", false);
        vi.advanceTimersByTime(SWARM_SETTLE_MS);
        expect(ctl.order()).toEqual(["b", "a"]);
    });

    it("lets a right-click menu hold the list for a bounded time", () => {
        const { ctl } = make();
        ctl.update([local("a", { lastActive: 2 }), local("b", { lastActive: 1 })]);
        ctl.setHold("menu", true);
        ctl.update([local("a", { lastActive: 2 }), local("b", { working: true, lastActive: 3 })]);
        vi.advanceTimersByTime(14_000);
        expect(ctl.order()).toEqual(["a", "b"]);
        vi.advanceTimersByTime(1_000 + SWARM_SETTLE_MS);
        expect(ctl.order()).toEqual(["b", "a"]);
        expect(ctl.held).toBe(false);
    });

    it("removes an agent that leaves at once and adds one that joins at the end, while held", () => {
        const { ctl } = make();
        ctl.update([local("a", { lastActive: 3 }), local("b", { lastActive: 2 }), local("c", { lastActive: 1 })]);
        ctl.setHold("pointer", true);
        ctl.update([
            local("a", { lastActive: 3 }),
            local("c", { lastActive: 1 }),
            local("d", { working: true, lastActive: 9 }),
        ]);
        expect(ctl.order()).toEqual(["a", "c", "d"]);
        ctl.setHold("pointer", false);
        vi.advanceTimersByTime(SWARM_SETTLE_MS);
        expect(ctl.order()).toEqual(["d", "a", "c"]);
    });

    it("dates an agent first seen working by its last output, and one that starts later by now", () => {
        let now = 1_000;
        const { ctl } = make(() => now);
        // x was already working when the list opened (last output 500); y is idle.
        ctl.update([local("x", { working: true, lastActive: 500 }), local("y", { lastActive: 400 })]);
        now = 2_000;
        // y starts working now, so it leaps above x.
        ctl.update([
            local("x", { working: true, lastActive: 1_900 }),
            local("y", { working: true, lastActive: 1_950 }),
        ]);
        vi.advanceTimersByTime(SWARM_SETTLE_MS);
        expect(ctl.order()).toEqual(["y", "x"]);
        // Output from x does not move it back above y.
        now = 3_000;
        ctl.update([
            local("x", { working: true, lastActive: 2_999 }),
            local("y", { working: true, lastActive: 2_000 }),
        ]);
        vi.advanceTimersByTime(SWARM_SETTLE_MS);
        expect(ctl.order()).toEqual(["y", "x"]);
    });

    it("forgets when an agent started once it stops working", () => {
        let now = 1_000;
        const { ctl } = make(() => now);
        ctl.update([local("x", { lastActive: 10 }), local("y", { lastActive: 20 })]);
        now = 2_000;
        ctl.update([local("x", { working: true, lastActive: 2_000 }), local("y", { lastActive: 20 })]);
        vi.advanceTimersByTime(SWARM_SETTLE_MS);
        now = 3_000;
        ctl.update([local("x", { lastActive: 2_900 }), local("y", { working: true, lastActive: 20 })]);
        vi.advanceTimersByTime(SWARM_SETTLE_MS);
        expect(ctl.order()).toEqual(["y", "x"]);
        now = 4_000;
        ctl.update([
            local("x", { working: true, lastActive: 4_000 }),
            local("y", { working: true, lastActive: 3_900 }),
        ]);
        vi.advanceTimersByTime(SWARM_SETTLE_MS);
        // x started again after y, so it is first.
        expect(ctl.order()).toEqual(["x", "y"]);
    });

    it("cancels a pending move when the change undoes itself", () => {
        const { ctl, moves } = make();
        const a = local("a", { lastActive: 2 });
        const b = local("b", { lastActive: 1 });
        ctl.update([a, b]);
        ctl.update([a, { ...b, working: true, lastActive: 3 }]);
        ctl.update([a, b]);
        vi.advanceTimersByTime(SWARM_SETTLE_MS * 3);
        expect(ctl.order()).toEqual(["a", "b"]);
        expect(moves).toEqual([]);
    });

    it("lets go of a pointer or focus hold the page no longer shows, so a lost leave or blur cannot freeze the list", () => {
        let shown = true;
        const { ctl } = make(undefined, () => shown);
        ctl.update([local("a", { lastActive: 2 }), local("b", { lastActive: 1 })]);
        ctl.setHold("pointer", true);
        ctl.setHold("focus", true);
        ctl.update([local("a", { lastActive: 2 }), local("b", { working: true, lastActive: 3 })]);
        vi.advanceTimersByTime(10_000);
        expect(ctl.order()).toEqual(["a", "b"]);
        // The pointer's list was removed under it: no leave ever came.
        shown = false;
        ctl.update([local("a", { lastActive: 2 }), local("b", { working: true, lastActive: 4 })]);
        expect(ctl.held).toBe(false);
        vi.advanceTimersByTime(SWARM_SETTLE_MS);
        expect(ctl.order()).toEqual(["b", "a"]);
    });

    it("lets go of the holds the list gave when the list is removed, but not a right-click menu's", () => {
        const { ctl } = make();
        ctl.update([local("a", { lastActive: 2 }), local("b", { lastActive: 1 })]);
        for (const hold of ["pointer", "press", "focus", "menu"] as const) ctl.setHold(hold, true);
        ctl.update([local("a", { lastActive: 2 }), local("b", { working: true, lastActive: 3 })]);
        ctl.releaseListHolds();
        expect(ctl.held).toBe(true);
        ctl.setHold("menu", false);
        vi.advanceTimersByTime(SWARM_SETTLE_MS);
        expect(ctl.order()).toEqual(["b", "a"]);
    });

    it("is not re-run by the order it publishes, when the view calls it from a computed", () => {
        const { ctl } = make();
        const facts = [local("a", { lastActive: 2 }), local("b", { lastActive: 1 })];
        let runs = 0;
        const dispose = createRoot((d) => {
            createComputed(() => {
                runs++;
                ctl.update(facts);
            });
            return d;
        });
        expect(runs).toBe(1);
        ctl.setHold("pointer", true);
        ctl.update([facts[0], { ...facts[1], working: true }]);
        ctl.setHold("pointer", false);
        vi.advanceTimersByTime(SWARM_SETTLE_MS);
        expect(ctl.order()).toEqual(["b", "a"]);
        expect(runs).toBe(1);
        dispose();
    });

    it("does not restart the wait when a hold that was not on is let go, as every key and pointer-up does", () => {
        const { ctl } = make();
        ctl.update([local("a", { lastActive: 2 }), local("b", { lastActive: 1 })]);
        ctl.update([local("a", { lastActive: 2 }), local("b", { working: true, lastActive: 3 })]);
        vi.advanceTimersByTime(SWARM_SETTLE_MS - 1);
        for (const hold of ["menu", "press", "pointer", "focus"] as const) ctl.setHold(hold, false);
        vi.advanceTimersByTime(1);
        expect(ctl.order()).toEqual(["b", "a"]);
    });

    it("stops its timers when disposed", () => {
        const { ctl } = make();
        ctl.update([local("a", { lastActive: 2 }), local("b", { lastActive: 1 })]);
        ctl.update([local("a", { lastActive: 2 }), local("b", { working: true, lastActive: 3 })]);
        ctl.dispose();
        vi.advanceTimersByTime(SWARM_SETTLE_MS * 2);
        expect(ctl.order()).toEqual(["a", "b"]);
    });
});
