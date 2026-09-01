// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * ShrinkTrace — per-node height-shrink attribution
 * (SPEC_CONTENT_RESIZE_CONTRACT_2026_08_31.md step 1).
 *
 * Pure bookkeeping, no DOM: the ResizeObserver that feeds it lives in
 * AgentDocumentVirtualList. Time is passed in explicitly rather than read from
 * `performance.now()` so the window logic is deterministic here.
 */

import { describe, expect, it } from "vitest";

import { ATTRIBUTION_WINDOW_MS, ShrinkTrace, formatAttribution } from "./shrink-trace";

describe("ShrinkTrace", () => {
    it("does not report the first observation of a node as a shrink", () => {
        const t = new ShrinkTrace();
        t.record("n1", "tool", 300, 1000);
        expect(t.attribute(0, 1000).shrinks).toEqual([]);
    });

    it("records a shrink and ignores growth", () => {
        const t = new ShrinkTrace();
        t.record("n1", "tool", 300, 1000);
        t.record("n1", "tool", 500, 1010); // growth — not a shrink
        t.record("n1", "tool", 120, 1020); // shrink 500 -> 120
        const a = t.attribute(380, 1020);
        expect(a.shrinks).toEqual([
            { nodeId: "n1", nodeType: "tool", fromPx: 500, toPx: 120, atMs: 1020 },
        ]);
        expect(a.attributedPx).toBe(380);
        expect(a.unattributedPx).toBe(0);
    });

    it("sums several rows and reports the unattributed remainder", () => {
        // The case the 08-22 findings could not distinguish: a pane delta that
        // is a SUM of small shrinks, not one component of that size. This is
        // why the ~251-252px lead had no matching height constant.
        const t = new ShrinkTrace();
        t.record("tool-a", "tool", 200, 1000);
        t.record("md-b", "markdown", 100, 1000);
        t.record("tool-a", "tool", 60, 1005); // -140
        t.record("md-b", "markdown", 12, 1006); // -88

        const a = t.attribute(251, 1006);
        expect(a.attributedPx).toBe(228);
        expect(a.unattributedPx).toBe(23); // 251 observed at the pane, 228 explained
        expect(a.shrinks.map((s) => s.nodeId)).toEqual(["tool-a", "md-b"]);
    });

    it("reports a negative remainder when rows shrank more than the pane did", () => {
        // Something else grew in the same frame. Sign matters: this is a
        // different situation from an under-explained shrink and must not be
        // rounded away or reported as zero.
        const t = new ShrinkTrace();
        t.record("n1", "tool", 500, 1000);
        t.record("n1", "tool", 100, 1001);
        expect(t.attribute(150, 1001).unattributedPx).toBe(-250);
    });

    it("excludes shrinks older than the attribution window", () => {
        const t = new ShrinkTrace();
        t.record("old", "tool", 400, 1000);
        t.record("old", "tool", 100, 1000); // -300, will fall outside the window
        t.record("new", "markdown", 50, 5000);
        t.record("new", "markdown", 20, 5000); // -30, inside

        const a = t.attribute(30, 5000, ATTRIBUTION_WINDOW_MS);
        expect(a.shrinks.map((s) => s.nodeId)).toEqual(["new"]);
        expect(a.attributedPx).toBe(30);
    });

    it("does not credit the same shrink to two consecutive pane deltas", () => {
        const t = new ShrinkTrace();
        t.record("n1", "tool", 300, 1000);
        t.record("n1", "tool", 100, 1001);

        expect(t.attribute(200, 1001).attributedPx).toBe(200);
        // Second pin check, same window — the shrink was already consumed.
        expect(t.attribute(200, 1002).attributedPx).toBe(0);
    });

    it("forgets a node's baseline on unmount so a remount is not a fake shrink", () => {
        // A node can leave the streaming buffer tall (cap-advance) and later
        // re-render short. Without forget(), the gap would register as one
        // enormous fabricated shrink.
        const t = new ShrinkTrace();
        t.record("n1", "tool", 900, 1000);
        t.forget("n1");
        t.record("n1", "tool", 40, 1010);
        expect(t.attribute(0, 1010).shrinks).toEqual([]);
    });

    it("keeps the ring bounded under a long burst", () => {
        const t = new ShrinkTrace();
        for (let i = 0; i < 500; i++) {
            t.record(`n${i}`, "tool", 100, 1000);
            t.record(`n${i}`, "tool", 50, 1000);
        }
        expect(t.attribute(0, 1000).shrinks.length).toBeLessThanOrEqual(64);
    });
});

describe("formatAttribution", () => {
    it("names the rows, the sum, and the remainder", () => {
        const t = new ShrinkTrace();
        t.record("tc-9abcdef01", "tool", 13400, 1000);
        t.record("tc-9abcdef01", "tool", 120, 1001);
        const s = formatAttribution(t.attribute(13502, 1001));
        expect(s).toContain("tc-9abc(tool) 13400->120px");
        expect(s).toContain("sum=13280px");
        expect(s).toContain("unattributed=222px");
    });

    it("is explicit when nothing observed shrank", () => {
        const t = new ShrinkTrace();
        // The important case: the pane shrank but no observed row did, so the
        // cause is somewhere not being watched.
        expect(formatAttribution(t.attribute(251, 1000))).toContain("none");
        expect(formatAttribution(t.attribute(251, 1000))).toContain("unattributed=251px");
    });
});
