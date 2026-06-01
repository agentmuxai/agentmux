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

    it("flushes (rolls back) AND disarms on non-printable input", () => {
        const h = harness();
        h.arm();
        h.input("a"); // predicted, screen "aa"
        expect(h.screen).toBe("aa");
        h.input("\r"); // Enter → flush + disarm
        expect(h.ops).toContain("erase:1");
        expect(h.screen).toBe("a");
        expect(h.pe.isArmed).toBe(false); // re-observe before predicting again
    });
});

describe("PredictiveEcho — mode-transition safety", () => {
    it("disarms at the Enter boundary so a following echo-off password char is never painted (reagent P0)", () => {
        const h = harness();
        h.arm(); // armed: echo was on and confirmed
        expect(h.pe.isArmed).toBe(true);
        h.input("\r"); // user runs `sudo …` — Enter is the line/echo-off boundary
        expect(h.pe.isArmed).toBe(false);
        // sudo prompts (echo OFF). The first password keystroke must be observed,
        // not predicted — the armed→echo-off transition is the hole reagent flagged.
        h.input("s");
        h.advance(700); // echo never returns (echo off)
        h.sweep();
        expect(h.ops).toEqual([]); // never painted → zero plaintext flash
        expect(h.screen).toBe("a");
    });

    it("disarms on alt-screen enter output so TUI commands aren't mispredicted (codex P1)", () => {
        const h = harness();
        h.arm();
        h.echo("\x1b[?1049h"); // a full-screen app (vim/less) takes over
        expect(h.pe.isArmed).toBe(false);
        const before = h.ops.length;
        h.input("j"); // vim normal-mode command — must NOT be painted
        expect(h.ops.length).toBe(before);
    });

    it("does not re-arm when alt-screen enter and an echo confirmation are in the same PTY chunk (codex P2)", () => {
        const h = harness();
        h.arm();
        // A chunk that contains both the echo confirmation AND the alt-screen enter.
        // The disarm must win; confirming the echo should not re-arm.
        h.echo("a\x1b[?1049h");
        expect(h.pe.isArmed).toBe(false);
        h.input("j"); // vim normal-mode — must NOT be painted
        expect(h.ops.filter(o => o.startsWith("paint"))).toHaveLength(0);
    });

    it("does not paint when unconfirmed observations are ahead in the queue — burst ordering (reagent P1)", () => {
        // "ab" typed at high RTT: "a" observed (unarmed), "b" typed before "a" echoes.
        // queue=[{a,obs}] → b must be OBSERVED, not painted, else "b" glyph appears
        // before authoritative "a" echo writes, scrambling order to "ba".
        const h = harness();
        h.input("a"); // unarmed → observed, queue=[{a,obs}]
        h.input("b"); // obs pending ahead → must observe, NOT paint
        expect(h.ops.filter(o => o.startsWith("paint"))).toHaveLength(0);
        h.advance(5);
        h.echo("a"); // "a" confirmed, armed=true, auth "a" written
        h.echo("b"); // "b" confirmed (was observed), auth "b" written
        expect(h.screen).toBe("ab"); // correct order, no scramble
    });

    it("sweeps a stale painted prediction on the next keystroke, with no PTY chunk and no timer (reagent P1)", () => {
        const h = harness();
        h.arm();
        h.input("b"); // armed → painted, queue=[b]
        expect(h.screen).toBe("ab");
        h.advance(700); // echo stalls past predictTimeout; NO chunk arrives
        h.input("c"); // the keystroke itself drives the sweep
        expect(h.ops).toContain("erase:1"); // stale 'b' rolled back on input
        expect(h.screen).not.toContain("b");
    });

    it("reset() rolls back a pending painted prediction so the authoritative echo writes cleanly (disable mid-prediction, codex P2)", () => {
        const h = harness();
        h.arm();
        h.input("b"); // painted, still pending
        expect(h.screen).toBe("ab");
        expect(h.pe.pending).toBe(1);
        // user toggles term:predictiveecho OFF mid-prediction; the output path
        // (doTerminalWrite) calls reset() before writing the authoritative echo.
        h.pe.reset();
        expect(h.ops).toContain("erase:1"); // speculative glyph rolled back
        expect(h.pe.pending).toBe(0);
        expect(h.screen).toBe("a"); // clean — the authoritative 'b' now writes exactly once
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
