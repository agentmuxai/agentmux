// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import {
    __getCurated,
    hashToolToParams,
    paramsForTool,
} from "../tool-tones";

describe("tool-tones params", () => {
    describe("curated palette", () => {
        const curated = __getCurated();

        it("contains every documented canonical tool", () => {
            for (const tool of [
                "Read",
                "Edit",
                "Write",
                "Bash",
                "Grep",
                "Glob",
                "Task",
                "Agent",
            ]) {
                expect(curated[tool]).toBeDefined();
            }
        });

        it("Read = falling major 2nd (B4 → A4)", () => {
            const r = curated["Read"];
            expect(r.tones).toEqual([494, 440]);
            expect(r.wave).toBe("sine");
            expect(r.durationMs).toBe(60);
        });

        it("Bash uses triangle wave, octave below Write", () => {
            const w = curated["Write"];
            const b = curated["Bash"];
            expect(b.wave).toBe("triangle");
            // Both rise by a perfect 5th; Bash an octave below.
            // Equal-temperament rounding leaves ~1 Hz tolerance.
            expect(Math.abs(b.tones[0] - w.tones[0] / 2)).toBeLessThan(1);
            expect(Math.abs(b.tones[1] - w.tones[1] / 2)).toBeLessThan(1);
        });

        it("Edit and Agent are the three-tone canonical syllables", () => {
            const threeToneCanonicals = Object.entries(curated)
                .filter(([_, p]) => p.tones.length === 3)
                .map(([k]) => k);
            // Edit + Agent are both 3-tone syllables per Appendix A.
            expect(threeToneCanonicals.sort()).toEqual(["Agent", "Edit"]);
        });

        it("all curated tones fall within the documented duration / gap windows", () => {
            for (const p of Object.values(curated)) {
                expect(p.durationMs).toBeGreaterThanOrEqual(40);
                expect(p.durationMs).toBeLessThanOrEqual(80);
                expect(p.gapMs).toBeGreaterThanOrEqual(10);
                expect(p.gapMs).toBeLessThanOrEqual(24);
            }
        });
    });

    describe("hash fallback", () => {
        it("is deterministic", () => {
            const a = hashToolToParams("mcp__weatherserver__get_forecast");
            const b = hashToolToParams("mcp__weatherserver__get_forecast");
            expect(a).toEqual(b);
        });

        it("produces different params for different tools", () => {
            const a = hashToolToParams("toolA");
            const b = hashToolToParams("toolB");
            // Same length (always 2 for hashed), but at least one
            // frequency or wave differs. We can't assert non-equality
            // across all hash inputs (theoretical collisions), but for
            // these two strings the FNV-1a output diverges.
            const same =
                a.tones[0] === b.tones[0] &&
                a.tones[1] === b.tones[1] &&
                a.wave === b.wave;
            expect(same).toBe(false);
        });

        it("only emits frequencies from the pentatonic set", () => {
            // G3 = 196, plus the 8 pentatonic degrees up from it
            // (0, 2, 4, 7, 9, 12, 14, 16 semitones).
            const allowed = [0, 2, 4, 7, 9, 12, 14, 16].map((s) =>
                196 * Math.pow(2, s / 12),
            );
            for (const sample of [
                "mcp__one",
                "foo",
                "barbaz",
                "WeirdTool-1",
                "Other",
                "",
            ]) {
                const p = hashToolToParams(sample);
                for (const t of p.tones) {
                    const close = allowed.some((a) => Math.abs(a - t) < 0.01);
                    expect(close).toBe(true);
                }
            }
        });

        it("uses only sine or triangle waves", () => {
            for (const sample of ["a", "b", "c", "d", "e", "Other", "x"]) {
                const p = hashToolToParams(sample);
                expect(["sine", "triangle"]).toContain(p.wave);
            }
        });
    });

    describe("paramsForTool", () => {
        it("returns curated for canonical tools", () => {
            expect(paramsForTool("Read")).toBe(__getCurated()["Read"]);
        });

        it("falls back to hash for unknown tools", () => {
            const params = paramsForTool("mcp__unknown__tool");
            // Hash output is always 2 tones (no curated 3-tone path).
            expect(params.tones.length).toBe(2);
        });

        it("hashes the literal raw tool name (no normalization)", () => {
            // "read" (lowercase) is NOT a curated key — different from "Read".
            const lower = paramsForTool("read");
            const canonical = paramsForTool("Read");
            expect(lower.tones).not.toEqual(canonical.tones);
        });
    });
});
