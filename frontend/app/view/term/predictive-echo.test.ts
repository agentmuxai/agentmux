// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { PredictiveEcho, isPredictable, type PredictiveEchoOptions } from "./predictive-echo";

/** Test harness: a mock xterm (a "screen" string) + a controllable clock. */
function harness(opts: Partial<PredictiveEchoOptions> = {}) {
    let t = 0;
    let screen = "";
    const ops: string[] = [];
    const sink = {
        paint(g: string) {
            ops.push(`paint:${g}`);
            screen += g;
        },
        erase(n: number) {
            ops.push(`erase:${n}`);
            screen = screen.slice(0, screen.length - n);
        },
    };
    const pe = new PredictiveEcho(sink, {
        enabled: () => true,
        thresholdMs: 0,
        now: () => t,
        ...opts,
    });
    return {
        pe,
        ops,
        get screen() {
            return screen;
        },
        advance(ms: number) {
            t += ms;
        },
        input(data: string) {
            pe.onInput(data);
        },
        /** Simulate the PTY emitting `chunk`: reconcile (may erase via the sink),
         *  THEN append the authoritative remainder. Two statements so the erase
         *  lands before we read `screen` for the append. */
        echo(chunk: string) {
            const auth = pe.reconcile(chunk);
            screen += auth;
        },
        sweep() {
            pe.sweep();
        },
        /** Bring the instance to `armed` (first char is observed, its echo confirms). */
        arm() {
            this.input("a");
            this.advance(5);
            this.echo("a"); // screen == "a", armed
        },
    };
}

describe("isPredictable (safe set)", () => {
    it("accepts single printable ASCII only", () => {
        for (const c of ["a", "Z", " ", "0", "~"]) expect(isPredictable(c)).toBe(true);
    });
    it("rejects control, escape, multi-char, and wide/CJK", () => {
        for (const c of ["\r", "\n", "\b", "\x1b", "\x1b[A", "ab", "中", "😀", ""]) {
            expect(isPredictable(c)).toBe(false);
        }
    });
});

describe("PredictiveEcho — arming & password safety", () => {
    it("never paints the first keystroke of a burst; arms only on a confirmed echo", () => {
        const h = harness();
        h.input("a");
        expect(h.ops).toEqual([]); // nothing speculative shown
        expect(h.pe.isArmed).toBe(false);
        h.advance(10);
        h.echo("a"); // PTY echoes it
        expect(h.screen).toBe("a"); // appears via the authoritative write
        expect(h.pe.isArmed).toBe(true);
    });

    it("password prompt (echo off): first char never painted, no flash even if echo never returns", () => {
        const h = harness();
        h.input("s"); // a password char — unarmed, so observed not painted
        expect(h.ops).toEqual([]);
        h.advance(700); // > predictTimeout
        h.sweep();
        expect(h.ops).toEqual([]); // never painted → zero plaintext flash
        expect(h.screen).toBe("");
    });
});

describe("PredictiveEcho — predict, confirm, diverge", () => {
    it("predicts once armed and consumes the confirming echo (no double render)", () => {
        const h = harness();
        h.arm();
        h.input("b");
        expect(h.ops).toContain("paint:b");
        expect(h.screen).toBe("ab");
        h.advance(8);
        h.echo("b"); // confirmed → consumed
        expect(h.screen).toBe("ab"); // unchanged, not "abb"
        expect(h.pe.pending).toBe(0);
    });

    it("rolls back and cools down on divergence; buffer converges to authoritative", () => {
        const h = harness();
        h.arm();
        h.input("b");
        expect(h.screen).toBe("ab");
        h.advance(5);
        h.echo("X"); // PTY echoed something else
        expect(h.ops).toContain("erase:1");
        expect(h.screen).toBe("aX"); // converged
        expect(h.pe.isArmed).toBe(false); // cooldown disarms
    });

    it("times out an unconfirmed prediction (echo turned off mid-burst)", () => {
        const h = harness();
        h.arm();
        h.input("p");
        expect(h.screen).toBe("ap");
        h.advance(601); // > predictTimeout default 600
        h.sweep();
        expect(h.ops).toContain("erase:1");
        expect(h.screen).toBe("a");
    });

    it("flushes (rolls back) on non-printable input", () => {
        const h = harness();
        h.arm();
        h.input("a"); // predicted, screen "aa"
        expect(h.screen).toBe("aa");
        h.input("\r"); // Enter → flush
        expect(h.ops).toContain("erase:1");
        expect(h.screen).toBe("a");
    });
});

describe("PredictiveEcho — gates", () => {
    it("does not predict when the round-trip is below threshold", () => {
        const h = harness({ thresholdMs: 100 });
        h.input("a");
        h.advance(5);
        h.echo("a"); // armed, rtt p50 ≈ 5
        const before = h.ops.length;
        h.input("b");
        expect(h.ops.length).toBe(before); // no paint — 5ms < 100ms
        expect(h.screen).toBe("a");
        h.advance(5);
        h.echo("b"); // 'b' still shows via authoritative write
        expect(h.screen).toBe("ab");
    });

    it("disabled → never paints, never touches the stream", () => {
        const h = harness({ enabled: () => false });
        h.input("a");
        h.echo("a");
        h.input("b");
        h.echo("b");
        expect(h.ops).toEqual([]);
        expect(h.screen).toBe("ab"); // pure passthrough
    });
});

describe("PredictiveEcho — convergence property", () => {
    it("the visible buffer always equals the authoritative echo stream", () => {
        const h = harness();
        let authoritative = "";
        h.input("a");
        h.advance(5);
        h.echo("a");
        authoritative += "a";
        // clean predicted chars
        for (const ch of ["b", "c", "d"]) {
            h.input(ch);
            h.advance(3);
            h.echo(ch);
            authoritative += ch;
        }
        // a divergence (e.g. shell auto-capitalized) then recovery after cooldown
        h.input("e");
        h.advance(3);
        h.echo("E");
        authoritative += "E";
        expect(h.screen).toBe(authoritative);
    });
});
