// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import {
    WEDGE_TIMEOUT_MS,
    markRecovered,
    newWedgeState,
    onSettle,
    onWrite,
    shouldRecover,
} from "./parser-wedge";

describe("parser wedge detection", () => {
    it("an idle terminal is never considered wedged", () => {
        const s = newWedgeState(0);
        // No pending writes: nothing has been handed to the parser, so silence
        // proves nothing. Without this an idle pane would recover on a timer.
        expect(shouldRecover(s, 10 * WEDGE_TIMEOUT_MS)).toBe(false);
    });

    it("a busy terminal that keeps acknowledging is not wedged", () => {
        const s = newWedgeState(0);
        let t = 0;
        for (let i = 0; i < 50; i++) {
            onWrite(s, t);
            t += 1000; // slower than a real terminal, still healthy
            onSettle(s, t);
            expect(shouldRecover(s, t)).toBe(false);
        }
    });

    it("writes that are never acknowledged trip recovery", () => {
        const s = newWedgeState(0);
        onWrite(s, 0);
        expect(shouldRecover(s, WEDGE_TIMEOUT_MS - 1)).toBe(false);
        expect(shouldRecover(s, WEDGE_TIMEOUT_MS + 1)).toBe(true);
    });

    it("a partially-drained backlog still trips if the parser then stops", () => {
        // The real shape of the bug: some chunks parse, then one throws and
        // everything behind it stalls.
        const s = newWedgeState(0);
        onWrite(s, 0);
        onWrite(s, 10);
        onSettle(s, 20); // first chunk fine
        onWrite(s, 30);
        expect(s.pending).toBeGreaterThan(0);
        expect(shouldRecover(s, 20 + WEDGE_TIMEOUT_MS + 1)).toBe(true);
    });

    it("does not thrash: recovery is rate-limited", () => {
        const s = newWedgeState(0);
        onWrite(s, 0);
        const trip = WEDGE_TIMEOUT_MS + 1;
        expect(shouldRecover(s, trip)).toBe(true);
        markRecovered(s, trip);

        // Still broken and still being written to, but inside the cooldown.
        onWrite(s, trip + 100);
        expect(shouldRecover(s, trip + 100 + WEDGE_TIMEOUT_MS + 1)).toBe(false);
    });

    it("clears the backlog on recovery so the cooldown can't immediately re-trip", () => {
        const s = newWedgeState(0);
        onWrite(s, 0);
        onWrite(s, 1);
        markRecovered(s, 100);
        expect(s.pending).toBe(0);
        // Those callbacks are never coming; without the reset this would fire
        // again the instant the cooldown lapsed.
        expect(shouldRecover(s, 100 + 10 * WEDGE_TIMEOUT_MS)).toBe(false);
    });

    it("recovers again if the pane wedges once more after the cooldown", () => {
        const s = newWedgeState(0);
        onWrite(s, 0);
        markRecovered(s, 0);
        const later = 60000;
        onWrite(s, later);
        expect(shouldRecover(s, later + WEDGE_TIMEOUT_MS + 1)).toBe(true);
    });
});
