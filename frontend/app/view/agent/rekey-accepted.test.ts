// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { HIDDEN_DELAY_MS } from "./stream-scheduler";
import { REKEY_PENDING_MAX_MS, createRekeyAccepted } from "./rekey-accepted";

const FRAME_MS = 16;
const at = { gen: "g", line: 3 };

function harness() {
    let clock = 0;
    let frames: Array<() => void> = [];
    const doc = new Set<string>();
    const queued = new Set<string>();
    const rekeyed: string[] = [];
    const rekeyAccepted = createRekeyAccepted({
        recordId: (record) => (record === "none" ? undefined : `h:${record}`),
        rekey: (from, to) => {
            if (!doc.delete(from)) return false;
            doc.add(to);
            return true;
        },
        isQueued: (id) => queued.has(id),
        onRekeyed: (to) => rekeyed.push(to),
        nextFrame: (run) => frames.push(run),
        now: () => clock,
    });
    /** Runs frames for `ms`, calling `each` at every frame (where the flush queue can land the row). */
    const run = (ms: number, each: () => void = () => {}) => {
        const end = clock + ms;
        while (clock < end) {
            clock += FRAME_MS;
            each();
            const due = frames;
            frames = [];
            for (const f of due) f();
        }
    };
    return { rekeyAccepted, doc, queued, rekeyed, run, clock: () => clock };
}

describe("rekeying an accepted pending message", () => {
    it("renames a row the document holds at once", () => {
        const h = harness();
        h.doc.add("p1");
        h.rekeyAccepted("p1", "rec", at);
        expect(h.rekeyed).toEqual(["h:rec"]);
        expect(h.doc.has("h:rec")).toBe(true);
    });

    it("does nothing for a record that holds no user message, or one with the row's own id", () => {
        const h = harness();
        h.doc.add("p1");
        h.rekeyAccepted("p1", "none", at);
        h.rekeyAccepted("h:rec", "rec", at);
        expect(h.rekeyed).toEqual([]);
    });

    it("waits for a row still in the flush queue and renames it once it lands", () => {
        const h = harness();
        h.queued.add("p1");
        h.rekeyAccepted("p1", "rec", at);
        h.run(3 * FRAME_MS);
        expect(h.rekeyed).toEqual([]);
        h.queued.delete("p1");
        h.doc.add("p1");
        h.run(2 * FRAME_MS);
        expect(h.rekeyed).toEqual(["h:rec"]);
    });

    it("a hidden pane's row, which lands HIDDEN_DELAY_MS or more after its request, is still renamed", () => {
        const h = harness();
        h.queued.add("p1");
        h.rekeyAccepted("p1", "rec", at);
        let landed = false;
        h.run(HIDDEN_DELAY_MS + 100, () => {
            if (!landed && h.clock() >= HIDDEN_DELAY_MS + FRAME_MS) {
                landed = true;
                h.queued.delete("p1");
                h.doc.add("p1");
            }
        });
        expect(landed).toBe(true);
        expect(h.rekeyed).toEqual(["h:rec"]);
    });

    it("stops after REKEY_PENDING_MAX_MS for a row that never leaves the queue", () => {
        const h = harness();
        h.queued.add("p1");
        h.rekeyAccepted("p1", "rec", at);
        h.run(REKEY_PENDING_MAX_MS + 10 * FRAME_MS);
        const stoppedAt = h.clock();
        h.run(10 * FRAME_MS);
        expect(h.rekeyed).toEqual([]);
        // Nothing is left to run: the loop ended rather than going on every frame.
        expect(h.clock()).toBe(stoppedAt + 10 * FRAME_MS);
    });

    it("does not retry a row that is neither in the document nor queued", () => {
        const h = harness();
        h.rekeyAccepted("p1", "rec", at);
        h.run(10 * FRAME_MS);
        expect(h.rekeyed).toEqual([]);
    });
});
