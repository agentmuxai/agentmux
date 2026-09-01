// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * ShrinkTrace — per-node height-shrink attribution
 * (SPEC_CONTENT_RESIZE_CONTRACT_2026_08_31.md step 1).
 *
 * Pure snapshot-diffing, no DOM and no clock: the caller reads row heights out
 * of the DOM and hands them in. See the module header for why this is sampled
 * synchronously rather than driven by a ResizeObserver.
 */

import { describe, expect, it } from "vitest";

import { ShrinkTrace, attribute, formatAttribution, type RowSample } from "./shrink-trace";

const row = (id: string, px: number, type = "tool"): RowSample => ({ id, type, px });

describe("ShrinkTrace.sample", () => {
    it("treats the first sample of a node as a baseline, not a shrink", () => {
        const t = new ShrinkTrace();
        expect(t.sample([row("n1", 300)])).toEqual([]);
    });

    it("reports a shrink between consecutive samples", () => {
        const t = new ShrinkTrace();
        t.sample([row("n1", 300)]);
        expect(t.sample([row("n1", 120)])).toEqual([
            { nodeId: "n1", nodeType: "tool", fromPx: 300, toPx: 120 },
        ]);
    });

    it("ignores growth", () => {
        const t = new ShrinkTrace();
        t.sample([row("n1", 100)]);
        expect(t.sample([row("n1", 500)])).toEqual([]);
    });

    it("reports several rows shrinking in the same sample", () => {
        const t = new ShrinkTrace();
        t.sample([row("a", 200), row("b", 100, "markdown")]);
        const shrinks = t.sample([row("a", 60), row("b", 12, "markdown")]);
        expect(shrinks).toEqual([
            { nodeId: "a", nodeType: "tool", fromPx: 200, toPx: 60 },
            { nodeId: "b", nodeType: "markdown", fromPx: 100, toPx: 12 },
        ]);
    });

    it("does not report the same shrink twice", () => {
        const t = new ShrinkTrace();
        t.sample([row("n1", 300)]);
        expect(t.sample([row("n1", 100)])).toHaveLength(1);
        expect(t.sample([row("n1", 100)])).toEqual([]); // unchanged since
    });

    it("drops a node that disappeared, so a remount is not a fake shrink", () => {
        // A cap-advance can retire a tall row; the same node can later render
        // again much shorter. Without pruning, that gap would look like one
        // enormous shrink that never actually happened on screen.
        const t = new ShrinkTrace();
        t.sample([row("n1", 900)]);
        t.sample([]); // n1 unmounted
        expect(t.sample([row("n1", 40)])).toEqual([]); // re-baselined, not a shrink
    });

    it("tracks rows independently as the buffer window slides", () => {
        const t = new ShrinkTrace();
        t.sample([row("a", 100), row("b", 200)]);
        // `a` scrolls out of the buffer, `c` enters; `b` shrinks.
        const shrinks = t.sample([row("b", 150), row("c", 300)]);
        expect(shrinks).toEqual([{ nodeId: "b", nodeType: "tool", fromPx: 200, toPx: 150 }]);
    });
});

describe("attribute", () => {
    it("sums the shrinks and reports a zero remainder on an exact match", () => {
        const a = attribute(380, [{ nodeId: "n1", nodeType: "tool", fromPx: 500, toPx: 120 }]);
        expect(a.attributedPx).toBe(380);
        expect(a.unattributedPx).toBe(0);
    });

    it("reports a positive remainder when observed rows under-explain the pane", () => {
        // The case the 08-22 findings could not distinguish: a pane delta that
        // is a SUM, with some of it coming from somewhere unobserved. This is
        // why the ~251-252px lead had no matching height constant.
        const a = attribute(251, [
            { nodeId: "a", nodeType: "tool", fromPx: 200, toPx: 60 },
            { nodeId: "b", nodeType: "markdown", fromPx: 100, toPx: 12 },
        ]);
        expect(a.attributedPx).toBe(228);
        expect(a.unattributedPx).toBe(23);
    });

    it("reports a negative remainder when rows shrank more than the pane did", () => {
        // Something else grew in the same window. Sign matters — this is a
        // different situation from an under-explained shrink, and must not be
        // clamped to zero.
        const a = attribute(150, [{ nodeId: "n1", nodeType: "tool", fromPx: 500, toPx: 100 }]);
        expect(a.unattributedPx).toBe(-250);
    });

    it("attributes nothing when no row shrank", () => {
        const a = attribute(251, []);
        expect(a.attributedPx).toBe(0);
        expect(a.unattributedPx).toBe(251);
    });
});

describe("formatAttribution", () => {
    it("names the rows, the sum, and the remainder", () => {
        const s = formatAttribution(
            attribute(13502, [{ nodeId: "tc-9abcdef01", nodeType: "tool", fromPx: 13400, toPx: 120 }]),
        );
        expect(s).toContain("tc-9abc(tool) 13400->120px");
        expect(s).toContain("sum=13280px");
        expect(s).toContain("unattributed=222px");
    });

    it("is explicit when nothing observed shrank", () => {
        // The important signal: the pane shrank but nothing being watched did,
        // so the cause is somewhere this instrumentation cannot yet see.
        const s = formatAttribution(attribute(251, []));
        expect(s).toContain("none");
        expect(s).toContain("unattributed=251px");
    });
});
