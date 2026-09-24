// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import {
    createTranscriptSettleLatch,
    EchoLedger,
    GAP_FILL_MAX_LINES,
    historyPin,
    TranscriptCursor,
    type CursorReadResult,
    type StreamPos,
    type TranscriptFileEvent,
} from "./transcript-cursor";

const G = "g:agent:def:current";
const B = "b:blk";

/** A stream on "disk" the cursor can read gaps from, plus what it did. */
function harness(opts: { lines?: string[]; gen?: string; stream?: string } = {}) {
    const disk = { lines: opts.lines ?? [], gen: opts.gen ?? "g1", stream: opts.stream ?? G };
    const delivered: string[] = [];
    const echoed: string[] = [];
    const resets: string[] = [];
    const reads: Array<[number, number, string]> = [];
    const logs: string[] = [];
    const ownEchoes = new Set<string>();
    let gate: Promise<void> | null = null;
    const cursor = new TranscriptCursor({
        deliver: (text) => delivered.push(...text.split("\n").filter((l) => l !== "")),
        echo: (text) => echoed.push(...text.split("\n").filter((l) => l !== "")),
        isOwnEcho: (line) => ownEchoes.delete(line),
        reset: (op) => resets.push(op),
        readRange: async (offset, limit, expectGen): Promise<CursorReadResult> => {
            reads.push([offset, limit, expectGen]);
            if (gate) await gate;
            if (expectGen !== disk.gen) return { lines: [], stream: disk.stream, gen: disk.gen, genMismatch: true };
            return { lines: disk.lines.slice(offset, offset + limit), stream: disk.stream, gen: disk.gen };
        },
        log: (m) => logs.push(m),
    });
    return {
        cursor,
        disk,
        delivered,
        echoed,
        resets,
        reads,
        logs,
        ownEchoes,
        /** Hold every read until the returned release is called. */
        holdReads(): () => void {
            let release!: () => void;
            gate = new Promise((r) => (release = r));
            return () => {
                gate = null;
                release();
            };
        },
    };
}

/** An append event for records `recs`, landing at `line` of each stream. */
function append(recs: string[], at: Array<[string, number]>, gen = "g1", echo?: string): TranscriptFileEvent {
    const pos: StreamPos[] = at.map(([stream, line]) => ({ stream, gen, line, lines: line + recs.length }));
    return { fileop: "append", data64: btoa(recs.map((r) => r + "\n").join("")), pos, echo };
}

const flush = () => new Promise((r) => setTimeout(r, 0));

describe("TranscriptCursor", () => {
    it("holds events until the history load settles, then drops what history showed", () => {
        const h = harness();
        h.cursor.push(append(["r1"], [[G, 1]]));
        h.cursor.push(append(["r2"], [[G, 2]]));
        expect(h.delivered).toEqual([]);
        // History showed lines 0 and 1.
        h.cursor.settle({ stream: G, gen: "g1", next: 2 });
        expect(h.delivered).toEqual(["r2"]);
        expect(h.cursor.stats.duplicates).toBe(1);
        expect(h.cursor.position()).toEqual({ stream: G, gen: "g1", next: 3 });
    });

    it("drops duplicates and delivers in-order events synchronously", () => {
        const h = harness();
        h.cursor.settle({ stream: G, gen: "g1", next: 0 });
        h.cursor.push(append(["a", "b"], [[G, 0]]));
        h.cursor.push(append(["a", "b"], [[G, 0]]));
        h.cursor.push(append(["c"], [[G, 2]]));
        expect(h.delivered).toEqual(["a", "b", "c"]);
        expect(h.cursor.stats.duplicates).toBe(1);
    });

    it("fills a gap by reading it, in order, and holds later events behind the read", async () => {
        const h = harness({ lines: ["l0", "l1", "l2", "l3", "l4"] });
        h.cursor.settle({ stream: G, gen: "g1", next: 1 });
        const release = h.holdReads();
        // Lines 1–2 were appended by another writer: no event for them.
        h.cursor.push(append(["l3"], [[G, 3]]));
        h.cursor.push(append(["l4"], [[G, 4]]));
        expect(h.delivered).toEqual([]);
        release();
        await flush();
        expect(h.reads).toEqual([[1, 2, "g1"]]);
        expect(h.delivered).toEqual(["l1", "l2", "l3", "l4"]);
        expect(h.cursor.stats.gapsFilled).toBe(1);
        expect(h.cursor.stats.gapLinesFilled).toBe(2);
        expect(h.cursor.position()!.next).toBe(5);
    });

    it("drops an event's records a gap read already delivered", async () => {
        // The poll read past a line whose own event was still in flight.
        const h = harness({ lines: ["l0", "l1", "l2", "l3"] });
        h.cursor.settle({ stream: G, gen: "g1", next: 0 });
        h.cursor.observeCount(3, G, "g1");
        await flush();
        h.cursor.push(append(["l2", "l3"], [[G, 2]]));
        expect(h.delivered).toEqual(["l0", "l1", "l2", "l3"]);
    });

    it("answers a generation change mid-fetch by skipping, not by mixing files", async () => {
        const h = harness({ lines: ["l0", "l1", "l2"] });
        h.cursor.settle({ stream: G, gen: "g1", next: 0 });
        const release = h.holdReads();
        h.cursor.push(append(["l2"], [[G, 2]]));
        // The file is replaced while the read is in flight.
        h.disk.gen = "g2";
        h.disk.lines = ["other"];
        release();
        await flush();
        expect(h.delivered).toEqual(["l2"]);
        expect(h.cursor.stats.linesSkipped).toBe(2);
        expect(h.logs.some((l) => l.includes("generation gone"))).toBe(true);
    });

    it("keeps its line on a re-count (new generation, same lines) and fills from there", async () => {
        const h = harness({ lines: ["l0", "l1", "l2", "l3"], gen: "g2" });
        h.cursor.settle({ stream: G, gen: "g1", next: 2 });
        h.cursor.push(append(["l3"], [[G, 3]], "g2"));
        await flush();
        expect(h.delivered).toEqual(["l2", "l3"]);
        expect(h.cursor.stats.genChanges).toBe(1);
    });

    it("joins a recreated file (a new generation starting below its line) at the event", () => {
        // agent:session:archive deleted the shared zone; the next session's
        // first record is line 0 of a new file.
        const h = harness({ lines: ["first"], gen: "g2" });
        h.cursor.settle({ stream: G, gen: "g1", next: 40 });
        h.cursor.push(append(["first"], [[G, 0]], "g2"));
        h.cursor.push(append(["second"], [[G, 1]], "g2"));
        expect(h.delivered).toEqual(["first", "second"]);
        expect(h.reads).toEqual([]);
        expect(h.cursor.position()).toEqual({ stream: G, gen: "g2", next: 2 });
    });

    it("joins a new generation at the event after an unpositioned record, to avoid repeats", () => {
        const h = harness({ lines: ["l0", "l1", "x", "l3"], gen: "g2" });
        h.cursor.settle({ stream: G, gen: "g1", next: 2 });
        // The epoch was dropped: this record has no position in G.
        h.cursor.push({ fileop: "append", data64: btoa("x\n"), pos: [{ stream: B, gen: "b1", line: 0, lines: 1 }] });
        h.cursor.push(append(["l3"], [[G, 3]], "g2"));
        expect(h.delivered).toEqual(["x", "l3"]);
        expect(h.reads).toEqual([]);
        expect(h.cursor.stats.unpositioned).toBe(1);
    });

    it("delivers events without positions as before", () => {
        const h = harness();
        h.cursor.settle(null);
        h.cursor.push({ fileop: "append", data64: btoa("plain\n") });
        expect(h.delivered).toEqual(["plain"]);
        expect(h.cursor.stats.unpositioned).toBe(1);
    });

    it("starts at the first event when history can't say where it ended", () => {
        const h = harness({ lines: ["l0", "l1", "l2", "l3"] });
        h.cursor.settle(null);
        h.cursor.push(append(["l3"], [[G, 3]]));
        expect(h.delivered).toEqual(["l3"]);
        expect(h.reads).toEqual([]);
    });

    it("fills from line 0 when history showed an empty stream", async () => {
        const h = harness({ lines: ["l0", "l1"] });
        h.cursor.settle("empty");
        h.cursor.push(append(["l1"], [[G, 1]]));
        await flush();
        expect(h.delivered).toEqual(["l0", "l1"]);
    });

    it("prefers the global zone before a pin, and pins to the history's stream after", () => {
        const h = harness();
        h.cursor.settle("empty");
        h.cursor.push(append(["a"], [[B, 0], [G, 0]]));
        expect(h.cursor.position()!.stream).toBe(G);

        const b = harness({ stream: B });
        b.cursor.settle({ stream: B, gen: "g1", next: 1 });
        b.cursor.push(append(["x"], [[B, 1], [G, 7]]));
        // Contiguous in B: follows the global zone from its line.
        expect(b.cursor.position()).toEqual({ stream: G, gen: "g1", next: 8 });
        expect(b.delivered).toEqual(["x"]);
    });

    it("hands echoes over without parsing them, and advances past them", () => {
        const h = harness();
        h.cursor.settle({ stream: G, gen: "g1", next: 0 });
        h.cursor.push(append(["user"], [[G, 0]], "g1", "stdin"));
        h.cursor.push(append(["reply"], [[G, 1]]));
        expect(h.echoed).toEqual(["user"]);
        expect(h.delivered).toEqual(["reply"]);
        expect(h.cursor.position()!.next).toBe(2);
    });

    it("drops a gap line that echoes a message the pane already shows", async () => {
        const h = harness({ lines: ["reply0", "mine", "reply2"] });
        h.ownEchoes.add("mine");
        h.cursor.settle({ stream: G, gen: "g1", next: 1 });
        h.cursor.push(append(["reply2"], [[G, 2]]));
        await flush();
        expect(h.delivered).toEqual(["reply2"]);
        expect(h.cursor.stats.ownEchoesDropped).toBe(1);
    });

    it("skips a gap too large to parse at once", () => {
        const h = harness();
        h.cursor.settle({ stream: G, gen: "g1", next: 0 });
        h.cursor.push(append(["late"], [[G, GAP_FILL_MAX_LINES + 1]]));
        return flush().then(() => {
            expect(h.reads).toEqual([]);
            expect(h.delivered).toEqual(["late"]);
            expect(h.cursor.stats.linesSkipped).toBe(GAP_FILL_MAX_LINES + 1);
        });
    });

    it("reads a large gap in chunks", async () => {
        const lines = Array.from({ length: 2_500 }, (_, i) => `l${i}`);
        const h = harness({ lines });
        h.cursor.settle({ stream: G, gen: "g1", next: 0 });
        h.cursor.observeCount(2_500, G, "g1");
        await flush();
        await flush();
        expect(h.reads.map(([o, n]) => [o, n])).toEqual([
            [0, 1000],
            [1000, 1000],
            [2000, 500],
        ]);
        expect(h.delivered).toEqual(lines);
    });

    it("skips the gap when the read fails, then carries on", async () => {
        const h = harness();
        h.cursor.settle({ stream: G, gen: "g1", next: 0 });
        (h.cursor as any).deps.readRange = async () => {
            throw new Error("timeout");
        };
        h.cursor.push(append(["l2"], [[G, 2]]));
        await flush();
        h.cursor.push(append(["l3"], [[G, 3]]));
        expect(h.delivered).toEqual(["l2", "l3"]);
        expect(h.cursor.stats.linesSkipped).toBe(2);
    });

    it("ignores a count for another stream", async () => {
        const h = harness({ lines: ["l0"] });
        h.cursor.settle({ stream: G, gen: "g1", next: 0 });
        h.cursor.observeCount(1, B, "g1");
        await flush();
        expect(h.reads).toEqual([]);
    });

    it("follows a re-count seen by the poll, keeping its line", async () => {
        // An older build's write made a reader re-count: same lines, new gen.
        const h = harness({ lines: ["l0", "l1", "l2"], gen: "g2" });
        h.cursor.settle({ stream: G, gen: "g1", next: 1 });
        h.cursor.observeCount(3, G, "g2");
        await flush();
        expect(h.reads).toEqual([[1, 2, "g2"]]);
        expect(h.delivered).toEqual(["l1", "l2"]);
        expect(h.cursor.position()).toEqual({ stream: G, gen: "g2", next: 3 });
    });

    it("leaves a count in a new generation below its line (a recreated file) to the next event", async () => {
        const h = harness({ lines: ["n0"], gen: "g2" });
        h.cursor.settle({ stream: G, gen: "g1", next: 40 });
        h.cursor.observeCount(1, G, "g2");
        await flush();
        expect(h.reads).toEqual([]);
        expect(h.cursor.position()).toEqual({ stream: G, gen: "g1", next: 40 });
    });

    it("resets on truncate and delete, and starts the new file from line 0", async () => {
        const h = harness({ lines: ["n0", "n1"], gen: "g2" });
        h.cursor.settle({ stream: G, gen: "g1", next: 5 });
        h.cursor.push({ fileop: "delete" });
        h.cursor.push(append(["n1"], [[G, 1]], "g2"));
        await flush();
        expect(h.resets).toEqual(["delete"]);
        expect(h.delivered).toEqual(["n0", "n1"]);
    });

    it("joins after a replace: the restored content is history, not live", () => {
        const h = harness();
        h.cursor.settle({ stream: G, gen: "g1", next: 5 });
        h.cursor.push({ fileop: "replace", pos: [{ stream: G, gen: "g9", line: 0, lines: 40 }] });
        h.cursor.push(append(["new"], [[G, 40]], "g9"));
        expect(h.resets).toEqual(["replace"]);
        expect(h.delivered).toEqual(["new"]);
        expect(h.reads).toEqual([]);
    });

    it("keeps going after the parser throws", () => {
        const h = harness();
        let calls = 0;
        (h.cursor as any).deps.deliver = (text: string) => {
            calls++;
            if (calls === 1) throw new Error("bad record");
            h.delivered.push(text.trim());
        };
        h.cursor.settle({ stream: G, gen: "g1", next: 0 });
        h.cursor.push(append(["bad"], [[G, 0]]));
        h.cursor.push(append(["good"], [[G, 1]]));
        expect(h.delivered).toEqual(["good"]);
        expect(h.logs.some((l) => l.includes("bad record"))).toBe(true);
    });

    it("stops after dispose, even with a read in flight", async () => {
        const h = harness({ lines: ["l0", "l1"] });
        h.cursor.settle({ stream: G, gen: "g1", next: 0 });
        const release = h.holdReads();
        h.cursor.push(append(["l1"], [[G, 1]]));
        h.cursor.dispose();
        release();
        await flush();
        expect(h.delivered).toEqual([]);
    });
});

describe("EchoLedger", () => {
    const rec = (text: string) => JSON.stringify({ type: "user", message: { role: "user", content: text } });

    it("matches a gap line to a shown message once", () => {
        const l = new EchoLedger();
        l.accepted("hi");
        expect(l.isOwnEcho(rec("hi"))).toBe(true);
        expect(l.isOwnEcho(rec("hi"))).toBe(false);
    });

    it("doesn't match a message whose echo already arrived, in either order", () => {
        const before = new EchoLedger();
        before.accepted("a");
        before.echoed(rec("a") + "\n");
        expect(before.isOwnEcho(rec("a"))).toBe(false);

        const after = new EchoLedger();
        after.echoed(rec("b") + "\n");
        after.accepted("b");
        expect(after.isOwnEcho(rec("b"))).toBe(false);
    });

    it("ignores records that aren't user text", () => {
        const l = new EchoLedger();
        l.accepted("x");
        expect(l.isOwnEcho(JSON.stringify({ type: "assistant", message: { content: "x" } }))).toBe(false);
        expect(l.isOwnEcho('{"type":"user", broken')).toBe(false);
        expect(l.isOwnEcho(rec("x"))).toBe(true);
    });
});

describe("historyPin", () => {
    it("ends where the lines the read returned end", () => {
        expect(historyPin(100, { lines: ["a", "b"], total: 102, stream: G, gen: "g1" })).toEqual({ stream: G, gen: "g1", next: 102 });
    });

    it("clamps an offset past the end (a stale high-water mark) to the total", () => {
        expect(historyPin(15_000, { lines: [], total: 100, stream: G, gen: "g1" })).toEqual({ stream: G, gen: "g1", next: 100 });
    });

    it("says nothing without a stream and generation", () => {
        expect(historyPin(0, { lines: ["a"], total: 1 })).toBeNull();
        expect(historyPin(0, null)).toBeNull();
    });
});

describe("createTranscriptSettleLatch", () => {
    it("delivers the first outcome to earlier and later listeners", () => {
        const latch = createTranscriptSettleLatch();
        const seen: unknown[] = [];
        latch.onSettle((o) => seen.push(o));
        latch.settle("empty");
        latch.settle(null);
        latch.onSettle((o) => seen.push(o));
        expect(seen).toEqual(["empty", "empty"]);
    });
});
