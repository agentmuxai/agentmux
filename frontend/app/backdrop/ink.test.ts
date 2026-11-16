// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The contrast proof for backdrops (spec §3.4): every built-in under both app themes, and a sweep of 4,096 colours,
 * against the targets in `INK_TARGETS`, on every surface a token can sit on.
 */

import { describe, expect, it } from "vitest";
import { contrastRatio } from "../block/pane-color-scheme";
import { hexToRgb, oklchOfHex, oklchToHex } from "../theme/oklch";
import { BUILTINS, deriveBuiltin, type AppScheme } from "./builtins";
import { deriveInk, INK_TARGETS, type DerivedBackdrop } from "./ink";

const SCHEMES: AppScheme[] = ["dark", "light"];

const layers = (d: DerivedBackdrop) => Object.values(d.surfaces);

/** The alpha a `#rrggbbaa` token paints with. */
const alphaOf = (token: string) => parseInt(token.slice(7, 9), 16) / 255;

/** A layer with the ink washed over it by `t`: what a selected row looks like. */
function washed(layer: string, ink: string, t: number): string {
    const [a, b] = [hexToRgb(layer)!, hexToRgb(ink.slice(0, 7))!];
    const ch = (i: number) =>
        Math.round((a[i] + (b[i] - a[i]) * t) * 255)
            .toString(16)
            .padStart(2, "0");
    return `#${ch(0)}${ch(1)}${ch(2)}`;
}

/** Every ink token against every surface it can sit on, and the text on the washes. */
function assertTargets(d: DerivedBackdrop, text: number, label: string) {
    expect(d.unmet, `${label}: ink tokens under their floor`).toEqual([]);
    for (const layer of layers(d)) {
        const at = (name: string, hex: string, floor: number) =>
            expect(contrastRatio(hex, layer), `${label}: ${name} ${hex} on ${layer}`).toBeGreaterThanOrEqual(floor);
        at("text", d.ink.text, text);
        at("muted", d.ink.muted, INK_TARGETS.muted.floor);
        at("faint", d.ink.faint, INK_TARGETS.faint.floor);
        at("border", d.ink.border, INK_TARGETS.border.floor);
        at("divider", d.ink.divider, INK_TARGETS.divider.floor);
        for (const k of ["accent", "error", "warning", "success", "info", "focus"] as const) {
            at(k, d.ink[k], INK_TARGETS.signal.floor);
        }
        // The text stays readable over a hovered and a selected row.
        at("text on hover", d.ink.text, text);
        expect(contrastRatio(d.ink.text, washed(layer, d.ink.text, 0.16)), `${label}: text on selected`).toBeGreaterThanOrEqual(
            4.5,
        );
    }
}

describe("the 16 built-ins", () => {
    for (const scheme of SCHEMES) {
        for (const b of BUILTINS) {
            it(`${b.id} under a ${scheme} app theme: primary text 7:1 and every token at its floor`, () => {
                const d = deriveBuiltin(b.id, scheme)!;
                expect(d.nudged).toBe(false);
                expect(d.level).toBe("high");
                expect(d.scheme).toBe(scheme);
                assertTargets(d, INK_TARGETS.text.floor, `${b.id}/${scheme}`);
            });
        }
    }

    it("has 16, in the palette's order, and none of the deep or pale tones is in the 7:1 dead zone", () => {
        expect(BUILTINS).toHaveLength(16);
        for (const scheme of SCHEMES) {
            for (const b of BUILTINS) {
                const surface = deriveBuiltin(b.id, scheme)!.surfaces.surface;
                const best = Math.max(contrastRatio("#ffffff", surface), contrastRatio("#000000", surface));
                expect(best, `${b.id}/${scheme}`).toBeGreaterThanOrEqual(7);
            }
        }
    });

    it("keeps error, warning and success apart from each other, in every built-in", () => {
        // OKLab distance (spec §3.4): colour is never the only carrier, but the three must not collapse.
        const lab = (hex: string) => {
            const o = oklchOfHex(hex)!;
            const rad = (o.h * Math.PI) / 180;
            return [o.l, o.c * Math.cos(rad), o.c * Math.sin(rad)];
        };
        const dist = (a: string, b: string) => Math.hypot(...lab(a).map((x, i) => x - lab(b)[i]));
        for (const scheme of SCHEMES) {
            for (const b of BUILTINS) {
                const { error, warning, success } = deriveBuiltin(b.id, scheme)!.ink;
                for (const [x, y] of [
                    [error, warning],
                    [error, success],
                    [warning, success],
                ]) {
                    expect(dist(x, y), `${b.id}/${scheme}: ${x} vs ${y}`).toBeGreaterThanOrEqual(0.08);
                }
            }
        }
    });

    it("keeps each status hue within 12 degrees of the app's own", () => {
        for (const scheme of SCHEMES) {
            for (const b of BUILTINS) {
                const d = deriveBuiltin(b.id, scheme)!;
                for (const [k, base] of [
                    ["error", "#e54d2e"],
                    ["warning", "#e0b956"],
                    ["success", "#4e9a06"],
                ] as const) {
                    const [a, c] = [oklchOfHex(base)!.h, oklchOfHex(d.ink[k])!.h];
                    const delta = Math.abs(((a - c + 540) % 360) - 180);
                    expect(delta, `${b.id}/${scheme}/${k}`).toBeLessThanOrEqual(12);
                }
            }
        }
    });
});

describe("any colour", () => {
    it("is held to the targets across a sweep of 4,096 colours, or reports what it could not reach", () => {
        let nudged = 0;
        let readable = 0;
        const bad: string[] = [];
        for (let hi = 0; hi < 16; hi++) {
            for (let ci = 0; ci < 8; ci++) {
                for (let li = 0; li < 32; li++) {
                    const hex = oklchToHex(0.03 + (li / 31) * 0.95, (ci / 7) * 0.3, hi * 22.5);
                    const d = deriveInk(hex);
                    if (d.nudged) nudged++;
                    if (d.level === "readable") readable++;
                    if (d.unmet.length) bad.push(`${hex}: ${d.unmet.join(",")}`);
                    for (const layer of layers(d)) {
                        const floor = d.level === "high" ? INK_TARGETS.text.floor : 4.5;
                        if (contrastRatio(d.ink.text, layer) < floor) bad.push(`${hex}: text on ${layer}`);
                        // The text keeps 4.5:1 over a hovered and a selected row, at every level.
                        for (const token of [d.ink.hover, d.ink.selected]) {
                            if (contrastRatio(d.ink.text, washed(layer, d.ink.text, alphaOf(token))) < 4.5) bad.push(`${hex}: text on ${layer} washed ${token}`);
                        }
                    }
                    expect(d.chosen).toBe(hex);
                }
            }
        }
        expect(bad.slice(0, 10)).toEqual([]);
        // The dead zone is real: some colours are nudged, and the nudge is not the common case.
        expect(nudged).toBeGreaterThan(0);
        expect(nudged).toBeLessThan(4096 / 2);
        // Past the cap only the middle of the dead zone (and the overlay layer's step) is left, a small share of all colours.
        expect(readable).toBeLessThan(4096 * 0.2);
    });

    it("nudges a mid-tone colour the smallest amount that gets primary text to 7:1, and says so", () => {
        const mid = "#808080";
        const d = deriveInk(mid);
        expect(d.nudged).toBe(true);
        expect(d.chosen).toBe(mid);
        expect(d.surfaces.surface).not.toBe(mid);
        // Within the cap it reaches 7:1; past it the colour is only "readable", moved just far enough for 4.5:1.
        const moved = Math.abs(oklchOfHex(d.surfaces.surface)!.l - oklchOfHex(mid)!.l);
        if (d.level === "high") {
            expect(moved).toBeLessThanOrEqual(0.081);
            expect(contrastRatio(d.ink.text, d.surfaces.raised)).toBeGreaterThanOrEqual(7);
        } else {
            expect(contrastRatio(d.ink.text, d.surfaces.raised)).toBeGreaterThanOrEqual(4.5);
        }
    });

    it("keeps primary text at 4.5:1 on a selected row for a mid-tone colour that is only readable", () => {
        const d = deriveInk("#808080");
        expect(d.level).toBe("readable");
        expect(d.unmet).toEqual([]);
        for (const layer of layers(d)) {
            expect(contrastRatio(d.ink.text, washed(layer, d.ink.text, alphaOf(d.ink.selected))), `on ${layer}`).toBeGreaterThanOrEqual(4.5);
        }
    });

    it("judges the wash by the alpha the token encodes, not the unrounded 16 percent", () => {
        // 0.16 is 41/255 = 0.16078 once written as a byte; a colour near the line passes at one and fails at the other.
        const d = deriveInk("#89557b");
        expect(alphaOf(d.ink.selected)).toBeCloseTo(41 / 255, 6);
        for (const layer of layers(d)) {
            expect(contrastRatio(d.ink.text, washed(layer, d.ink.text, alphaOf(d.ink.selected))), `on ${layer}`).toBeGreaterThanOrEqual(4.5);
        }
    });

    it("does not touch a colour that already reads", () => {
        const dark = deriveInk("#1a2b3c");
        expect(dark.nudged).toBe(false);
        expect(dark.surfaces.surface).toBe("#1a2b3c");
        expect(dark.scheme).toBe("dark");
        const pale = deriveInk("#f2efe6");
        expect(pale.nudged).toBe(false);
        expect(pale.scheme).toBe("light");
    });

    it("gives the same answer for the same colour", () => {
        expect(deriveInk("#445566")).toEqual(deriveInk("#445566"));
    });

    it("steps the raised and overlay layers away from the ink", () => {
        const dark = deriveInk("#202830");
        expect(oklchOfHex(dark.surfaces.raised)!.l).toBeGreaterThan(oklchOfHex(dark.surfaces.surface)!.l);
        expect(oklchOfHex(dark.surfaces.overlay)!.l).toBeGreaterThan(oklchOfHex(dark.surfaces.raised)!.l);
        const pale = deriveInk("#eef2f6");
        expect(oklchOfHex(pale.surfaces.raised)!.l).toBeLessThan(oklchOfHex(pale.surfaces.surface)!.l);
    });

    it("falls back to a colour for something that is not one", () => {
        expect(deriveInk("not a colour").unmet).toEqual([]);
    });
});
