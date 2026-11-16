// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * `toneMove`: a colour made to read on a surface, in lightness only. Proved over the terminal palette the transcript
 * draws its syntax and status colours from (theme.scss's `--term-*`), on a pale and on a dark surface.
 */

import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { contrastRatio } from "../block/pane-color-scheme";
import { oklchOfHex } from "../theme/oklch";
import { deriveBuiltin } from "./builtins";
import { toneMove } from "./ink";

/** theme.scss's own `--term-*` colours: the palette `_document-nodes.scss` and the shell drawer read. */
function termPalette(): Array<[string, string]> {
    const scss = readFileSync(join(__dirname, "..", "theme.scss"), "utf8");
    const found = [...scss.matchAll(/^\s*(--term-[a-z-]+):\s*(#[0-9a-f]{6})\b/gim)].map((m) => [m[1], m[2].toLowerCase()] as [string, string]);
    return [...new Map(found).entries()];
}

const PALE = deriveBuiltin("yellow", "light")!.surfaces.surface;
const DARK = deriveBuiltin("violet", "dark")!.surfaces.surface;
const TARGET = 4.5;

const hueGap = (a: number, b: number) => Math.min(Math.abs(a - b), 360 - Math.abs(a - b));

describe("toneMove over the terminal palette", () => {
    const palette = termPalette();

    it("finds the palette it is tested on: the eight ANSI colours and their bright forms", () => {
        const names = palette.map(([n]) => n);
        for (const n of ["--term-green", "--term-yellow", "--term-blue", "--term-magenta", "--term-cyan", "--term-bright-green", "--term-bright-cyan", "--term-bright-magenta"]) {
            expect(names, n).toContain(n);
        }
        expect(palette.length).toBeGreaterThanOrEqual(14);
    });

    for (const [label, surface] of [["a pale surface", PALE], ["a dark surface", DARK]] as const) {
        describe(`on ${label}`, () => {
            it(`gives every colour ${TARGET}:1 or says it could not, measured on the hex it returns`, () => {
                for (const [name, hex] of palette) {
                    const r = toneMove(hex, surface, TARGET);
                    expect(r.ratio, name).toBeCloseTo(contrastRatio(r.hex, surface), 10);
                    expect(r.met, `${name} ${hex} -> ${r.hex} at ${r.ratio.toFixed(2)}`).toBe(true);
                    expect(r.ratio, name).toBeGreaterThanOrEqual(TARGET);
                }
            });

            it("keeps a colour that already reads, byte for byte", () => {
                for (const [name, hex] of palette) {
                    if (contrastRatio(hex, surface) < TARGET) continue;
                    expect(toneMove(hex, surface, TARGET), name).toEqual({ hex, ratio: contrastRatio(hex, surface), met: true, moved: false });
                }
            });

            it("keeps the hue of one it moves, and moves it toward the side with more contrast", () => {
                const lightSurface = surface === PALE;
                for (const [name, hex] of palette) {
                    const r = toneMove(hex, surface, TARGET);
                    if (!r.moved) continue;
                    const [before, after] = [oklchOfHex(hex)!, oklchOfHex(r.hex)!];
                    // A hue is only defined with some chroma; the clip to the sRGB gamut at the ends moves it a little.
                    if (before.c > 0.04 && after.c > 0.04) expect(hueGap(before.h, after.h), name).toBeLessThan(12);
                    expect(after.l < before.l, name).toBe(lightSurface);
                }
            });

            it("is deterministic", () => {
                for (const [, hex] of palette) expect(toneMove(hex, surface, TARGET)).toEqual(toneMove(hex, surface, TARGET));
            });
        });
    }

    it("moves something on each surface: bright cyan is unreadable on the pale one, a dark blue on the dark one", () => {
        const cyan = palette.find(([n]) => n === "--term-bright-cyan")![1];
        expect(contrastRatio(cyan, PALE)).toBeLessThan(TARGET);
        expect(toneMove(cyan, PALE, TARGET).moved).toBe(true);
        const blue = palette.find(([n]) => n === "--term-blue")![1];
        expect(contrastRatio(blue, DARK)).toBeLessThan(TARGET);
        expect(toneMove(blue, DARK, TARGET).moved).toBe(true);
    });

    it("reports when no lightness reaches the target: the farthest colour, met false", () => {
        const r = toneMove("#34e2e2", PALE, 21);
        expect(r.met).toBe(false);
        expect(r.moved).toBe(true);
        expect(r.ratio).toBeLessThan(21);
        expect(r.ratio).toBeGreaterThan(contrastRatio("#34e2e2", PALE));
    });

    it("leaves something that is not a colour as it is", () => {
        expect(toneMove("tomato", PALE, TARGET).moved).toBe(false);
    });
});
