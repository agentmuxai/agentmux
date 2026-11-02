// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { PANE_COLOR_TOKENS, contrastRatio, paneRoleColor, type PaneColorRole } from "../block/pane-color-scheme";
import { hexToRgb, oklchHueOfHex, oklchToHex, srgbToLinear } from "./oklch";
import { farthestFreeSlot, hueDistance, PALETTE, pickSlot, slotById, slotOf } from "./palette";

// CIEDE2000 between two `#rrggbb`, the metric the palette's spacing is judged by
// (spec §3: it predicts observers better than plain Oklab distance).
function deltaE2000(hexA: string, hexB: string): number {
    const lab = (hex: string): [number, number, number] => {
        const [r, g, b] = hexToRgb(hex)!.map(srgbToLinear);
        const X = 0.4124564 * r + 0.3575761 * g + 0.1804375 * b;
        const Y = 0.2126729 * r + 0.7151522 * g + 0.072175 * b;
        const Z = 0.0193339 * r + 0.119192 * g + 0.9503041 * b;
        const f = (t: number) => (t > 216 / 24389 ? Math.cbrt(t) : ((24389 / 27) * t + 16) / 116);
        const [fx, fy, fz] = [f(X / 0.95047), f(Y), f(Z / 1.08883)];
        return [116 * fy - 16, 500 * (fx - fy), 200 * (fy - fz)];
    };
    const rad = (d: number) => (d * Math.PI) / 180;
    const deg = (r: number) => (r * 180) / Math.PI;
    const [L1, a1, b1] = lab(hexA);
    const [L2, a2, b2] = lab(hexB);
    const Cb = (Math.hypot(a1, b1) + Math.hypot(a2, b2)) / 2;
    const G = 0.5 * (1 - Math.sqrt(Cb ** 7 / (Cb ** 7 + 25 ** 7)));
    const [a1p, a2p] = [(1 + G) * a1, (1 + G) * a2];
    const [C1p, C2p] = [Math.hypot(a1p, b1), Math.hypot(a2p, b2)];
    const hp = (b: number, a: number) => (b === 0 && a === 0 ? 0 : (deg(Math.atan2(b, a)) + 360) % 360);
    const [h1p, h2p] = [hp(b1, a1p), hp(b2, a2p)];
    let dh = 0;
    if (C1p * C2p !== 0) {
        dh = h2p - h1p;
        if (dh > 180) dh -= 360;
        else if (dh < -180) dh += 360;
    }
    const dL = L2 - L1;
    const dC = C2p - C1p;
    const dH = 2 * Math.sqrt(C1p * C2p) * Math.sin(rad(dh / 2));
    const Lb = (L1 + L2) / 2;
    const Cbp = (C1p + C2p) / 2;
    let hb = h1p + h2p;
    if (C1p * C2p !== 0) hb = Math.abs(h1p - h2p) <= 180 ? hb / 2 : (hb + (hb < 360 ? 360 : -360)) / 2;
    const T =
        1 - 0.17 * Math.cos(rad(hb - 30)) + 0.24 * Math.cos(rad(2 * hb)) + 0.32 * Math.cos(rad(3 * hb + 6)) - 0.2 * Math.cos(rad(4 * hb - 63));
    const dTheta = 30 * Math.exp(-(((hb - 275) / 25) ** 2));
    const Rc = 2 * Math.sqrt(Cbp ** 7 / (Cbp ** 7 + 25 ** 7));
    const Sl = 1 + (0.015 * (Lb - 50) ** 2) / Math.sqrt(20 + (Lb - 50) ** 2);
    const Sc = 1 + 0.045 * Cbp;
    const Sh = 1 + 0.015 * Cbp * T;
    const Rt = -Math.sin(rad(2 * dTheta)) * Rc;
    return Math.sqrt((dL / Sl) ** 2 + (dC / Sc) ** 2 + (dH / Sh) ** 2 + Rt * (dC / Sc) * (dH / Sh));
}

const THEMES = ["dark", "light"] as const;
const tone = (hue: number, role: PaneColorRole, theme: (typeof THEMES)[number]) => {
    const t = PANE_COLOR_TOKENS[theme][role];
    return oklchToHex(t.l, t.c, hue);
};

describe("the palette", () => {
    it("has 16 slots with unique ids, in hue order, each a valid hex at its own hue", () => {
        expect(PALETTE).toHaveLength(16);
        expect(new Set(PALETTE.map((s) => s.id)).size).toBe(16);
        for (const [i, s] of PALETTE.entries()) {
            expect(s.id).toBe(s.name.toLowerCase());
            if (i > 0) expect(s.hue).toBeGreaterThan(PALETTE[i - 1].hue);
            expect(s.hex).toMatch(/^#[0-9a-f]{6}$/);
            expect(hueDistance(oklchHueOfHex(s.hex)!, s.hue), s.id).toBeLessThan(0.5);
        }
    });

    it("stores each hex at the swatch lightness and chroma its file declares", () => {
        for (const s of PALETTE) expect(oklchToHex(0.7, 0.14, s.hue), s.id).toBe(s.hex);
    });

    for (const theme of THEMES) {
        it(`${theme}: neighbouring slots are at least 9 apart (CIEDE2000) at the identity tone, and none is an outlier`, () => {
            const gaps = PALETTE.map((s, i) => deltaE2000(tone(s.hue, "identity", theme), tone(PALETTE[(i + 1) % 16].hue, "identity", theme)));
            expect(Math.min(...gaps)).toBeGreaterThanOrEqual(9);
            // "Equidistant": the widest gap is within 1.5x of the narrowest.
            expect(Math.max(...gaps) / Math.min(...gaps)).toBeLessThan(1.5);
        });

        it(`${theme}: every slot's identity tone clears 3:1 against its pill, active pill and header tint`, () => {
            for (const s of PALETTE) {
                const id = tone(s.hue, "identity", theme);
                for (const bg of ["pill", "pillActive", "headerTint"] as const) {
                    expect(contrastRatio(id, tone(s.hue, bg, theme)), `${s.id} on ${bg}`).toBeGreaterThanOrEqual(3);
                }
            }
        });
    }
});

describe("slotOf", () => {
    it("takes a slot id, in any case", () => {
        expect(slotOf("azure")?.id).toBe("azure");
        expect(slotOf(" Azure ")?.id).toBe("azure");
    });

    it("takes a slot's own hex back to its slot", () => {
        for (const s of PALETTE) expect(slotOf(s.hex)?.id).toBe(s.id);
    });

    it("snaps any hex to the nearest hue", () => {
        expect(slotOf("#3b82f6")?.id).toBe("azure");
        expect(slotOf("#14b8a6")?.id).toBe("teal");
        expect(slotOf("#ec4899")?.id).toBe("pink");
        expect(slotOf("#a855f7")?.id).toBe("violet");
        expect(slotOf("#f59e0b")?.id).toBe("amber");
        expect(slotOf("#F59E0B")?.id).toBe("amber");
    });

    it("keeps today's 14 agent colours on 14 different slots", () => {
        const old = ["#ef4444", "#f97316", "#f59e0b", "#eab308", "#84cc16", "#22c55e", "#14b8a6", "#06b6d4", "#3b82f6", "#6366f1", "#8b5cf6", "#d946ef", "#ec4899", "#f43f5e"];
        expect(new Set(old.map((h) => slotOf(h)!.id)).size).toBe(14);
    });

    it("reads a legacy HSL hue (the old Pane Color menu) as the slot nearest the colour it drew", () => {
        // hsl(h, 65%, 52%) at the twelve 30° steps.
        const want = ["red", "amber", "lime", "green", "green", "jade", "cyan", "azure", "indigo", "violet", "magenta", "pink"];
        expect(Array.from({ length: 12 }, (_, i) => slotOf(i * 30)!.id)).toEqual(want);
        expect(slotOf(360)?.id).toBe(slotOf(0)?.id);
        expect(slotOf(-30)?.id).toBe(slotOf(330)?.id);
    });

    it("is no colour for anything else", () => {
        for (const bad of [undefined, null, "", "chartreuse", "#fff", "#gggggg", "#3b82f6; }", NaN, Infinity]) {
            expect(slotOf(bad as never), String(bad)).toBeUndefined();
        }
        expect(slotById("nope")).toBeUndefined();
    });

    it("lets a pane draw an old hex in the palette: the same tones as its slot", () => {
        for (const theme of [false, true]) {
            expect(paneRoleColor(undefined, "#3b82f6", theme, "identity")).toBe(paneRoleColor(undefined, "azure", theme, "identity"));
        }
    });
});

describe("pickSlot", () => {
    it("is deterministic and a member", () => {
        for (const id of ["a", "abc-123", ""]) expect(pickSlot(id)).toBe(pickSlot(id));
    });

    it("takes the least-used slot, so 25 agents use no colour more than twice", () => {
        const taken: string[] = [];
        for (let i = 0; i < 25; i++) taken.push(pickSlot(`agent-${i}`, taken).hex);
        const counts = new Map<string, number>();
        for (const h of taken) counts.set(h, (counts.get(h) ?? 0) + 1);
        expect(Math.max(...counts.values())).toBeLessThanOrEqual(2);
        expect(new Set(taken.slice(0, 16)).size).toBe(16);
    });

    it("counts an old hex toward the slot it snaps to", () => {
        const others = PALETTE.filter((s) => s.id !== "teal").map((s) => s.hex);
        expect(pickSlot("x", [...others, "#14b8a6"]).id).not.toBe("teal");
        expect(pickSlot("x", others).id).toBe("teal");
    });
});

describe("farthestFreeSlot", () => {
    it("starts at the first slot, then takes the farthest unused one", () => {
        const first = farthestFreeSlot();
        expect(first.id).toBe("crimson");
        expect(hueDistance(first.hue, farthestFreeSlot([first.hex]).hue)).toBeGreaterThan(150);
    });

    it("fills a gap left by a deleted profile, never a repeat", () => {
        const used = PALETTE.filter((s) => s.id !== "jade").map((s) => s.hex);
        expect(farthestFreeSlot(used).id).toBe("jade");
    });
});
