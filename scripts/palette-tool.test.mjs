// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The palette file follows the method (what `palette-tool.mjs --check` checks), and the tool measures with the tones
// the app draws.

import { describe, expect, it } from "vitest";
import { PANE_COLOR_TOKENS } from "../frontend/app/block/pane-color-scheme";
import { best, measure, problems, readPalette, SWATCH, TOKENS } from "./palette-tool.mjs";

describe("palette-tool", () => {
    const read = readPalette();

    it("finds nothing wrong with schema/color-palette.json", () => {
        expect(problems(read)).toEqual([]);
    });

    it("reads 12 hues and the two neutrals", () => {
        expect(read.hues.map((s) => s.id)).toEqual(["rose", "red", "orange", "amber", "yellow", "green", "jade", "teal", "sky", "blue", "violet", "magenta"]);
        expect(read.neutrals.map((s) => [s.id, s.neutral, s.hex])).toEqual([
            ["black", "black", "#000000"],
            ["white", "white", "#ffffff"],
        ]);
    });

    it("measures with the app's own tones", () => {
        expect(TOKENS).toEqual(PANE_COLOR_TOKENS);
        expect(SWATCH).toEqual(read.palette.swatch);
    });

    it("gives the file's hues for 12, and the neighbour distances the spec publishes", () => {
        const hues = read.hues.map((s) => s.hue);
        expect(best(12).hues).toEqual(hues);
        const m = measure(hues);
        const one = (x) => Number(x.toFixed(1));
        expect([one(m.dark.neighbourMin), one(m.dark.neighbourMax)]).toEqual([12.9, 17.4]);
        expect([one(m.light.neighbourMin), one(m.light.neighbourMax)]).toEqual([13.8, 16.4]);
    });

    it("says what is wrong with a file that has drifted", () => {
        const moved = structuredClone(read.palette);
        moved.slots[3].hue += 4;
        moved.slots[12].hex = "#111111";
        const found = problems({ palette: moved, hues: moved.slots.filter((s) => s.neutral === undefined), neutrals: moved.slots.filter((s) => s.neutral !== undefined) });
        expect(found.some((p) => p.startsWith("hues are"))).toBe(true);
        expect(found.some((p) => p.startsWith("amber: hex is"))).toBe(true);
        expect(found.some((p) => p.startsWith("black:"))).toBe(true);
    });
});
