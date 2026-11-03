// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { CLOSING_HOLD_MS, nextHeld, type HeldRow } from "./window-slot-keys";

const set = (...k: string[]) => new Set(k);
const names = (...pairs: [string, string][]) => new Map(pairs);

describe("nextHeld", () => {
    it("holds a window that was open and is now in neither list", () => {
        const held = nextHeld(names(["ws-1", "Alpha"], ["ws-2", "Bravo"]), set("ws-1"), set(), [], 100);
        expect(held).toEqual([{ key: "ws-2", name: "Bravo", since: 100 }]);
    });

    it("holds nothing for a window still open, or already listed as closed", () => {
        expect(nextHeld(names(["ws-1", "Alpha"], ["ws-2", "Bravo"]), set("ws-1"), set("ws-2"), [], 100)).toEqual([]);
    });

    it("ends the hold when the window is listed again, open or closed", () => {
        const held: HeldRow[] = [{ key: "ws-2", name: "Bravo", since: 100 }];
        expect(nextHeld(names(), set("ws-1"), set("ws-2"), held, 200)).toEqual([]);
        expect(nextHeld(names(), set("ws-1", "ws-2"), set(), held, 200)).toEqual([]);
    });

    it("ends the hold after CLOSING_HOLD_MS, once, and holds it again only if it was open again meanwhile", () => {
        const held: HeldRow[] = [{ key: "ws-2", name: "Bravo", since: 100 }];
        expect(nextHeld(names(), set("ws-1"), set(), held, 100 + CLOSING_HOLD_MS - 1)).toBe(held);
        expect(nextHeld(names(), set("ws-1"), set(), held, 100 + CLOSING_HOLD_MS)).toEqual([]);
    });

    it("keeps the same array when nothing changed, so nothing redraws", () => {
        const held: HeldRow[] = [{ key: "ws-2", name: "Bravo", since: 100 }];
        expect(nextHeld(names(), set("ws-1"), set(), held, 150)).toBe(held);
    });

    it("never holds a window whose workspace is not known yet", () => {
        expect(nextHeld(names(["label:window-2", "Window 2"]), set(), set(), [], 100)).toEqual([]);
    });
});
