// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import {
    SHOW_SETTLE_MAX_MS,
    SHOW_SETTLE_MIN_MS,
    SHOW_SETTLE_SAMPLE_MS,
    SHOW_SETTLE_STILL_SAMPLES,
    ShowSettle,
} from "./show-settle";

/** Sample every SHOW_SETTLE_SAMPLE_MS from `from` to `to`, with the height each tick gives. */
function run(
    s: ShowSettle,
    from: number,
    to: number,
    height: (t: number) => number,
    covered: (t: number) => boolean = () => false
) {
    for (let t = from; t <= to; t += SHOW_SETTLE_SAMPLE_MS) {
        if (s.sample(t, covered(t), height(t))) return t;
    }
    return null;
}

describe("ShowSettle", () => {
    it("is settled before any show (there is no catch-up to wait for)", () => {
        expect(new ShowSettle().settled(0)).toBe(true);
    });

    it("settles at the minimum when the layout is still from the start", () => {
        const s = new ShowSettle();
        s.start(0);
        const at = run(s, SHOW_SETTLE_SAMPLE_MS, 5_000, () => 1000);
        expect(at).not.toBeNull();
        expect(at!).toBeGreaterThanOrEqual(SHOW_SETTLE_MIN_MS);
        expect(at!).toBeLessThan(SHOW_SETTLE_MIN_MS + SHOW_SETTLE_SAMPLE_MS);
        expect(s.settled(at!)).toBe(true);
    });

    it("waits past the minimum for a return that is still growing, then settles once it stops", () => {
        const s = new ShowSettle();
        s.start(0);
        // Grows every sample until 1500 ms, then stays.
        const at = run(s, SHOW_SETTLE_SAMPLE_MS, 5_000, (t) => 1000 + Math.min(t, 1500));
        expect(at).not.toBeNull();
        expect(at!).toBeGreaterThanOrEqual(1500 + SHOW_SETTLE_STILL_SAMPLES * SHOW_SETTLE_SAMPLE_MS);
        expect(at!).toBeLessThan(SHOW_SETTLE_MAX_MS);
    });

    it("does not count samples taken under the cover", () => {
        const s = new ShowSettle();
        s.start(0);
        // Height is steady, but the cover stays up until 1000 ms.
        const at = run(
            s,
            SHOW_SETTLE_SAMPLE_MS,
            5_000,
            () => 1000,
            (t) => t < 1000
        );
        expect(at!).toBeGreaterThanOrEqual(1000 + (SHOW_SETTLE_STILL_SAMPLES - 1) * SHOW_SETTLE_SAMPLE_MS);
    });

    it("is settled at the cap whatever the layout does, and also when nothing is sampling", () => {
        const s = new ShowSettle();
        s.start(0);
        expect(run(s, SHOW_SETTLE_SAMPLE_MS, SHOW_SETTLE_MAX_MS - 1, (t) => t)).toBeNull(); // always changing
        expect(s.settled(SHOW_SETTLE_MAX_MS - 1)).toBe(false);
        expect(s.settled(SHOW_SETTLE_MAX_MS)).toBe(true);
        const t = new ShowSettle();
        t.start(0);
        expect(t.sample(SHOW_SETTLE_MAX_MS, true, 1)).toBe(true);
    });

    it("a new show starts over, and a cancelled one stops waiting", () => {
        const s = new ShowSettle();
        s.start(0);
        expect(run(s, SHOW_SETTLE_SAMPLE_MS, 5_000, () => 1)).not.toBeNull();
        s.start(10_000);
        expect(s.settled(10_000)).toBe(false);
        s.cancel();
        expect(s.settled(10_001)).toBe(true);
    });
});
