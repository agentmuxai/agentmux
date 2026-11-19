// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { readdirSync, readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

import { contrastRatio } from "../block/pane-color-scheme";
import { oklchOfHex } from "./oklch";
import { BAR_FLOOR, CLOSE_FLOOR, DEFAULT_TAB_INK, deltaE, LABEL_FLOOR, readableFill, tabPaint, tabSample, type TabInk } from "./paint";
import { HUE_SLOTS, PALETTE } from "./palette";

const THEMES = [false, true] as const;
/** The surfaces a tab sits against: the window behind the tab bar, opaque, in the default dark and light themes. */
const STRIPS = { dark: ["#000000", "#222222"], light: ["#ffffff", "#f6f6f7"] } as const;
const name = (light: boolean) => (light ? "light" : "dark");

describe("tabPaint, for every slot, state and theme", () => {
    for (const light of THEMES) {
        describe(name(light), () => {
            for (const slot of PALETTE) {
                const ink = DEFAULT_TAB_INK[name(light)];
                const p = tabPaint(slot, light);

                it(`${slot.id}: the picker's sample is the selected fill and the bar the tab paints`, () => {
                    expect(tabSample(slot, light)).toEqual({ fill: p.selected, bar: p.bar });
                });

                it(`${slot.id}: the label reads on every fill, and the bar on the selected one`, () => {
                    expect(contrastRatio(ink.selected, p.selected)).toBeGreaterThanOrEqual(LABEL_FLOOR);
                    expect(contrastRatio(ink.selected, p.selectedHover)).toBeGreaterThanOrEqual(LABEL_FLOOR);
                    expect(contrastRatio(ink.unselected, p.unselected)).toBeGreaterThanOrEqual(LABEL_FLOOR);
                    expect(contrastRatio(ink.unselected, p.unselectedHover)).toBeGreaterThanOrEqual(LABEL_FLOOR);
                    expect(contrastRatio(p.bar, p.selected)).toBeGreaterThanOrEqual(BAR_FLOOR);
                    expect(contrastRatio(p.bar, p.selectedHover)).toBeGreaterThanOrEqual(BAR_FLOOR);
                });

                it(`${slot.id}: the bar reads against the strip, and the close mark on the fills it shows on`, () => {
                    for (const strip of STRIPS[name(light)]) expect(contrastRatio(p.bar, strip)).toBeGreaterThanOrEqual(BAR_FLOOR);
                    // The close mark is drawn in the secondary text, always on the selected tab and on a hovered one.
                    for (const fill of [p.selected, p.selectedHover, p.unselectedHover]) {
                        expect(contrastRatio(ink.unselected, fill)).toBeGreaterThanOrEqual(CLOSE_FLOOR);
                    }
                });

                it(`${slot.id}: hover is a small step from its resting fill, and the bar has no hover colour`, () => {
                    for (const [rest, hover] of [[p.selected, p.selectedHover], [p.unselected, p.unselectedHover]]) {
                        expect(hover).not.toBe(rest);
                        const step = deltaE(rest, hover);
                        expect(step).toBeGreaterThan(1);
                        expect(step).toBeLessThan(8);
                    }
                });

                it(`${slot.id}: unselected is clearly unselected next to the selected tab`, () => {
                    expect(deltaE(p.selected, p.unselected)).toBeGreaterThan(5);
                });
            }
        });
    }

    it("keeps the hue of a hue slot in every fill", () => {
        for (const light of THEMES) {
            for (const slot of HUE_SLOTS) {
                const p = tabPaint(slot, light);
                for (const fill of [p.selected, p.unselected, p.selectedHover, p.unselectedHover]) {
                    const o = oklchOfHex(fill)!;
                    const dh = Math.abs(((o.h - slot.hue + 540) % 360) - 180);
                    expect(dh).toBeLessThan(8);
                }
            }
        }
    });
});

describe("readableFill", () => {
    it("leaves a fill that already reads", () => {
        expect(readableFill("#203654", "#f7f7f7")).toBe("#203654");
    });

    it("moves a fill that does not toward the side away from its label, keeping hue, until it reads", () => {
        const moved = readableFill("#8a8100", "#f7f7f7");
        expect(moved).not.toBe("#8a8100");
        expect(contrastRatio("#f7f7f7", moved)).toBeGreaterThanOrEqual(LABEL_FLOOR);
        expect(oklchOfHex(moved)!.l).toBeLessThan(oklchOfHex("#8a8100")!.l);
        const lighter = readableFill("#444444", "#0d0f11");
        expect(contrastRatio("#0d0f11", lighter)).toBeGreaterThanOrEqual(LABEL_FLOOR);
        expect(oklchOfHex(lighter)!.l).toBeGreaterThan(oklchOfHex("#444444")!.l);
    });
});

/** `--main-text-color` and `--secondary-text-color` of a theme's stylesheet, as `#rrggbb`. */
function themeInk(file: string): TabInk | undefined {
    const css = readFileSync(file, "utf8");
    const token = (prop: string): string | undefined => {
        const raw = new RegExp(`${prop}:\\s*([^;]+);`).exec(css)?.[1]?.trim();
        if (!raw) return undefined;
        if (/^#[0-9a-f]{6}$/i.test(raw)) return raw.toLowerCase();
        const m = /^rgb\(\s*(\d+),\s*(\d+),\s*(\d+)\s*\)$/.exec(raw);
        return m ? "#" + [m[1], m[2], m[3]].map((n) => Number(n).toString(16).padStart(2, "0")).join("") : undefined;
    };
    const [selected, unselected] = [token("--main-text-color"), token("--secondary-text-color")];
    return selected && unselected ? { selected, unselected } : undefined;
}

describe("tab fills made under every theme's own text, as the app makes them", () => {
    const themesDir = fileURLToPath(new URL("../themes/", import.meta.url));
    const LIGHT = new Set(["light", "catppuccin-latte", "solarized-light", "gruvbox-light"]);
    const files = [
        { id: "default", path: fileURLToPath(new URL("../theme.scss", import.meta.url)) },
        ...readdirSync(themesDir)
            .filter((f) => f.endsWith(".scss") && f !== "index.scss")
            .map((f) => ({ id: f.replace(/\.scss$/, ""), path: themesDir + f })),
    ];

    it("reads the text tokens of every theme file", () => {
        expect(files.length).toBeGreaterThanOrEqual(13);
        for (const f of files) expect(themeInk(f.path), f.id).toBeDefined();
    });

    for (const f of files) {
        it(`${f.id}: the label reads on every hue's tab fills`, () => {
            const ink = themeInk(f.path)!;
            const light = LIGHT.has(f.id);
            for (const slot of PALETTE) {
                const p = tabPaint(slot, light, ink);
                expect(contrastRatio(ink.selected, p.selected), `${slot.id} selected`).toBeGreaterThanOrEqual(LABEL_FLOOR);
                expect(contrastRatio(ink.selected, p.selectedHover), `${slot.id} selected hover`).toBeGreaterThanOrEqual(LABEL_FLOOR);
                expect(contrastRatio(ink.unselected, p.unselected), `${slot.id} unselected`).toBeGreaterThanOrEqual(LABEL_FLOOR);
                expect(contrastRatio(ink.unselected, p.unselectedHover), `${slot.id} unselected hover`).toBeGreaterThanOrEqual(LABEL_FLOOR);
                expect(contrastRatio(p.bar, p.selected), `${slot.id} bar`).toBeGreaterThanOrEqual(BAR_FLOOR);
                expect(deltaE(p.selected, p.unselected), `${slot.id} selected from unselected`).toBeGreaterThan(5);
            }
        });
    }
});
