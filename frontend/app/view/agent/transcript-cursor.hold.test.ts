// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { TranscriptCursor, type StreamPos, type TranscriptFileEvent } from "./transcript-cursor";

const G = "g:agent:def:current";

/** An append of `recs` landing at `line` of the shared stream. */
function append(recs: string[], line: number, gen = "g1"): TranscriptFileEvent {
    const pos: StreamPos[] = [{ stream: G, gen, line, lines: line + recs.length }];
    return { fileop: "append", data64: btoa(recs.map((r) => r + "\n").join("")), pos };
}

/** A cursor that counts what reaches the parser, with a pane that can be held and a scheduler slot it can wake. */
function pane(opts: { held?: boolean } = {}) {
    const state = { held: opts.held ?? false, wakes: [] as Array<() => void> };
    const parses: Array<{ text: string; line?: number }> = [];
    const echoed: string[] = [];
    const order: string[] = [];
    const cursor = new TranscriptCursor({
        deliver: (text, from) => {
            parses.push({ text, line: from?.line });
            order.push("deliver");
        },
        echo: (text) => {
            echoed.push(text);
            order.push("echo");
        },
        isOwnEcho: () => false,
        reset: () => order.push("reset"),
        readRange: async () => ({ lines: [] }),
        log: () => {},
        hold: () => state.held,
        wake: (drain) => state.wakes.push(drain),
    });
    cursor.settle({ stream: G, gen: "g1", next: 0 });
    return { cursor, state, parses, echoed, order };
}

const lines = (parses: Array<{ text: string }>) => parses.flatMap((p) => p.text.split("\n").filter(Boolean));

describe("TranscriptCursor while the pane is held", () => {
    it("decodes and parses nothing, and asks for one slot however many events arrive", () => {
        const p = pane({ held: true });
        for (let i = 0; i < 50; i++) p.cursor.push(append([`r${i}`], i));
        expect(p.parses).toHaveLength(0);
        expect(p.cursor.queued()).toBe(50);
        expect(p.state.wakes).toHaveLength(1);
    });

    it("places everything in one parse when its slot comes: same lines, same order, same positions", () => {
        const p = pane({ held: true });
        for (let i = 0; i < 50; i++) p.cursor.push(append([`r${i}`], i));
        p.state.wakes[0]();
        expect(p.parses).toHaveLength(1);
        expect(p.parses[0].line).toBe(0);
        expect(lines(p.parses)).toEqual(Array.from({ length: 50 }, (_, i) => `r${i}`));
        expect(p.cursor.position()?.next).toBe(50);
    });

    it("a pane that is shown again catches up at once, and the pending slot does nothing more", () => {
        const p = pane({ held: true });
        p.cursor.push(append(["a", "b"], 0));
        p.cursor.push(append(["c"], 2));
        p.state.held = false;
        p.cursor.release();
        expect(p.parses).toHaveLength(1);
        expect(lines(p.parses)).toEqual(["a", "b", "c"]);
        p.state.wakes[0]();
        expect(p.parses).toHaveLength(1);
    });

    it("asks for a new slot for what arrives after a pass", () => {
        const p = pane({ held: true });
        p.cursor.push(append(["a"], 0));
        p.state.wakes[0]();
        p.cursor.push(append(["b"], 1));
        expect(p.state.wakes).toHaveLength(2);
        p.state.wakes[1]();
        expect(lines(p.parses)).toEqual(["a", "b"]);
    });

    it("a duplicate of what was already placed is dropped, held or not", () => {
        const p = pane({ held: true });
        p.cursor.push(append(["a", "b"], 0));
        p.cursor.push(append(["b"], 1));
        p.cursor.push(append(["c"], 2));
        p.state.wakes[0]();
        expect(lines(p.parses)).toEqual(["a", "b", "c"]);
    });

    it("keeps order around an echo and a reset: records before them are parsed first", () => {
        const p = pane({ held: true });
        p.cursor.push(append(["a"], 0));
        p.cursor.push({ ...append(["you said"], 1), echo: "1" });
        p.cursor.push(append(["b"], 2));
        p.cursor.push({ fileop: "truncate" });
        p.state.wakes[0]();
        expect(p.order).toEqual(["deliver", "echo", "deliver", "reset"]);
        expect(lines(p.parses)).toEqual(["a", "b"]);
    });

    it("events that wait for the history load are placed when it settles, held or not", () => {
        const state = { held: true, wakes: [] as Array<() => void> };
        const parses: string[] = [];
        const cursor = new TranscriptCursor({
            deliver: (t) => parses.push(t),
            echo: () => {},
            isOwnEcho: () => false,
            reset: () => {},
            readRange: async () => ({ lines: [] }),
            log: () => {},
            hold: () => state.held,
            wake: (d) => state.wakes.push(d),
        });
        cursor.push(append(["a"], 0));
        expect(state.wakes).toHaveLength(0);
        cursor.settle({ stream: G, gen: "g1", next: 0 });
        expect(state.wakes).toHaveLength(1);
        state.wakes[0]();
        expect(parses.join("")).toBe("a\n");
    });
});

describe("TranscriptCursor on a shown pane", () => {
    it("places each event on arrival, as before: a shown pane waits for nothing", () => {
        const p = pane();
        p.cursor.push(append(["a"], 0));
        expect(lines(p.parses)).toEqual(["a"]);
        p.cursor.push(append(["b"], 1));
        expect(lines(p.parses)).toEqual(["a", "b"]);
        expect(p.state.wakes).toHaveLength(0);
    });

    it("does not join records whose positions do not run on", () => {
        const p = pane({ held: true });
        p.cursor.push(append(["a"], 0));
        p.cursor.push({ ...append(["b"], 1), pos: [{ stream: G, gen: "g1", line: 1, lines: 2 }] });
        p.cursor.push({ fileop: "append", data64: btoa("loose\n") }); // no position: delivered as it is
        p.state.wakes[0]();
        expect(lines(p.parses)).toEqual(["a", "b", "loose"]);
        expect(p.parses.length).toBeGreaterThanOrEqual(2);
    });

    it("a parser that throws loses only its own pass, and the cursor keeps going", () => {
        let boom = true;
        const parses: string[] = [];
        const cursor = new TranscriptCursor({
            deliver: (t) => {
                if (boom) {
                    boom = false;
                    throw new Error("parser");
                }
                parses.push(t);
            },
            echo: () => {},
            isOwnEcho: () => false,
            reset: () => {},
            readRange: async () => ({ lines: [] }),
            log: () => {},
        });
        cursor.settle({ stream: G, gen: "g1", next: 0 });
        cursor.push(append(["a"], 0));
        cursor.push(append(["b"], 1));
        expect(parses.join("")).toBe("b\n");
    });
});
