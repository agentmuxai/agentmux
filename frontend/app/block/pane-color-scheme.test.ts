// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { HUE_SLOTS, PALETTE, slotById } from "@/app/theme/palette";
import {
    contrastRatio,
    identitySlot,
    NEUTRAL_TONES,
    oklchToHex,
    PANE_COLOR_TOKENS,
    paneRoleColor,
    slotRoleColor,
    slotSwatchColor,
    type PaneColorRole,
} from "./pane-color-scheme";

// Every identity a pane can have: each palette slot, as its id (a pane's own
// pick, a widget's colour) and as the hex an agent stores, and each older HSL
// pick the Pane Color menu used to write.
interface Identity {
    name: string;
    hue?: number;
    hex?: string;
}
// The identities with a hue: each of the 12, and each older HSL pick.
const HUE_IDENTITIES: Identity[] = [
    ...Array.from({ length: 12 }, (_, i) => ({ name: `legacy hue ${i * 30}`, hue: i * 30 })),
    ...HUE_SLOTS.map((s) => ({ name: `slot ${s.id}`, hex: s.id })),
    ...HUE_SLOTS.map((s) => ({ name: `slot hex ${s.id}`, hex: s.hex })),
];
const NEUTRALS = PALETTE.filter((s) => s.neutral);
const NEUTRAL_IDENTITIES: Identity[] = [...NEUTRALS.map((s) => ({ name: `slot ${s.id}`, hex: s.id })), ...NEUTRALS.map((s) => ({ name: `slot hex ${s.id}`, hex: s.hex }))];
const IDENTITIES: Identity[] = [...HUE_IDENTITIES, ...NEUTRAL_IDENTITIES];

// The neutral header a mixed pane falls back to in the dark theme
// (blockframe.tsx NON_AGENT_DEFAULT_HEADER_BG, hsl(220, 12%, 16%)).
const DARK_NEUTRAL_HEADER = "#24272e";

// The top bar's opaque background, under its translucency (theme.scss and
// themes/light.scss --main-bg-color).
const TOP_BAR_BG = { dark: "#222222", light: "#f6f6f7" };
// The monochrome widget icon the tint replaces (--secondary-text-color).
const MONO_ICON = { dark: "#c3c8c2", light: "#4b535c" };

describe("widgetTint (a top-bar widget icon)", () => {
    for (const theme of ["dark", "light"] as const) {
        for (const o of PALETTE) {
            it(`${theme}, ${o.name}: clears WCAG 3:1 against the bar and stays as bright as the monochrome icon`, () => {
                const tint = paneRoleColor(undefined, o.id, theme === "light", "widgetTint")!;
                expect(contrastRatio(tint, TOP_BAR_BG[theme])).toBeGreaterThanOrEqual(3);
                // Subtle: about the same luminance as the grey icon, so no
                // widget is louder than the others. Black and White are not
                // held to it: their icon is the neutral's own line tone.
                if (!o.neutral) expect(contrastRatio(tint, MONO_ICON[theme])).toBeLessThan(1.25);
            });
        }
    }
});

describe("pane colour scheme", () => {
    for (const theme of ["dark", "light"] as const) {
        const light = theme === "light";
        describe(`${theme} theme`, () => {
            for (const id of IDENTITIES) {
                it(`${id.name}: the active underline clears WCAG 3:1 against every surface it touches`, () => {
                    const c = (role: Parameters<typeof paneRoleColor>[3]) => paneRoleColor(id.hue, id.hex, light, role)!;
                    const identity = c("identity");
                    // The underline sits on the active pill; inactive pills and the
                    // header row are its other neighbours (SC 1.4.11).
                    expect(contrastRatio(identity, c("pillActive"))).toBeGreaterThanOrEqual(3);
                    expect(contrastRatio(identity, c("pill"))).toBeGreaterThanOrEqual(3);
                    expect(contrastRatio(identity, c("headerTint"))).toBeGreaterThanOrEqual(3);
                    if (!light) expect(contrastRatio(identity, DARK_NEUTRAL_HEADER)).toBeGreaterThanOrEqual(3);
                });
            }

            it("the active pill is distinguishable from an inactive one of the same hue, not only by the underline", () => {
                for (const id of IDENTITIES) {
                    const a = paneRoleColor(id.hue, id.hex, light, "pillActive")!;
                    const b = paneRoleColor(id.hue, id.hex, light, "pill")!;
                    expect(a).not.toBe(b);
                }
            });
        });
    }

    it("gives every hue the same perceived lightness for a role, so no agent is louder than another", () => {
        // OKLCH L is fixed per role; gamut mapping only lowers chroma, so the
        // luminance spread across hues stays small (HSL gave 0.55–0.82 in OKLCH L).
        const lums = HUE_IDENTITIES.map((id) => contrastRatio(paneRoleColor(id.hue, id.hex, false, "identity")!, "#000000"));
        const spread = Math.max(...lums) / Math.min(...lums);
        expect(spread).toBeLessThan(1.25);
    });

    // The focus ring tells you which pane has focus. Under HSL an unfocused
    // yellow border (OKLCH L 0.56) was brighter than a focused blue ring
    // (0.50), so a yellow pane next to a blue one could look focused when it
    // was not. Now every hue's focused ring stands out from the page more than
    // any hue's unfocused border: brighter on dark, darker on light.
    for (const [theme, light, page] of [
        ["dark", false, "#000000"],
        ["light", true, "#ffffff"],
    ] as const) {
        const prominence = (ids: Identity[], role: "identity" | "border") => ids.map((id) => contrastRatio(paneRoleColor(id.hue, id.hex, light, role)!, page));

        it(`${theme} theme: every focused ring stands out more than any hue's unfocused border`, () => {
            expect(Math.min(...prominence(IDENTITIES, "identity"))).toBeGreaterThan(Math.max(...prominence(HUE_IDENTITIES, "border")) * 1.5);
        });

        // Not across the two neutrals: White's unfocused border on a dark theme is brighter than Black's focused
        // ring (graphite), and the same the other way round on a light theme. Each still stands clear of its own.
        it(`${theme} theme: Black's and White's focused rings stand out more than their own unfocused borders`, () => {
            for (const id of NEUTRAL_IDENTITIES) {
                expect(prominence([id], "identity")[0], id.name).toBeGreaterThan(prominence([id], "border")[0] * 1.5);
            }
        });
    }

    it("maps a slot id and the hex it persists as to the same slot, and an older HSL pick to its nearest hue", () => {
        for (const s of PALETTE) {
            expect(identitySlot(undefined, s.id)).toBe(s);
            expect(identitySlot(undefined, s.hex)).toBe(s);
        }
        expect(identitySlot(240, undefined)).toBe(slotById("blue"));
        // The explicit pick wins over the agent's colour.
        expect(identitySlot(240, "#000000")).toBe(slotById("blue"));
        expect(identitySlot(undefined, undefined)).toBeUndefined();
    });

    it("returns undefined for a pane with no colour of its own", () => {
        expect(paneRoleColor(undefined, undefined, false, "pill")).toBeUndefined();
        expect(paneRoleColor(undefined, "not-a-color", false, "pill")).toBeUndefined();
    });

    it("paints a picker's swatch in the hue's identity tone for the theme, and Black and White as themselves in both", () => {
        for (const light of [false, true]) {
            for (const s of HUE_SLOTS) expect(slotSwatchColor(s, light)).toBe(paneRoleColor(undefined, s.id, light, "identity"));
            expect(slotSwatchColor(slotById("black")!, light)).toBe("#000000");
            expect(slotSwatchColor(slotById("white")!, light)).toBe("#ffffff");
        }
    });

    it("keeps out-of-gamut colours inside sRGB by lowering chroma", () => {
        expect(oklchToHex(0.75, 0.4, 145)).toMatch(/^#[0-9a-f]{6}$/);
        expect(oklchToHex(PANE_COLOR_TOKENS.dark.identity.l, 0, 0)).toMatch(/^#([0-9a-f]{2})\1\1$/);
    });
});

// docs/specs/SPEC_SHARED_PALETTE_12_HUES_BLACK_WHITE_2026_11_16.md §3: Black and White have a lightness per role and
// theme, since a black line cannot be seen on dark chrome nor a white one on light chrome.
describe("Black and White", () => {
    // The chrome a pane's ring and a tab's underline are drawn on: the window, the tab strip, the pane, and the
    // neutral header (theme.scss and themes/light.scss; blockframe.tsx NON_AGENT_DEFAULT_HEADER_BG).
    const CHROME = { dark: ["#222222", "#292929", "#111111", DARK_NEUTRAL_HEADER], light: ["#f6f6f7", "#eeeeef", "#fbfbfc", "#ffffff"] };
    // The theme's own text (--main-text-color).
    const TEXT = { dark: "#f7f7f7", light: "#0d0f11" };
    const FILLS: PaneColorRole[] = ["pill", "pillActive", "headerTint"];

    it("draws the tones the spec lists, greys with no chroma", () => {
        const hexes = (theme: "dark" | "light", neutral: "black" | "white") =>
            (["identity", "border", "pill", "pillActive", "headerTint", "widgetTint"] as const).map((role) => paneRoleColor(undefined, neutral, theme === "light", role));
        expect(hexes("dark", "black")).toEqual(["#747474", "#484848", "#0d0d0d", "#000000", "#090909", "#747474"]);
        expect(hexes("dark", "white")).toEqual(["#f5f5f5", "#868686", "#4d4d4d", "#636363", "#3d3d3d", "#f5f5f5"]);
        expect(hexes("light", "black")).toEqual(["#0b0b0b", "#717171", "#bebebe", "#a4a4a4", "#d1d1d1", "#0b0b0b"]);
        expect(hexes("light", "white")).toEqual(["#868686", "#cacaca", "#fafafa", "#ffffff", "#ffffff", "#868686"]);
    });

    for (const theme of ["dark", "light"] as const) {
        for (const slot of NEUTRALS) {
            const c = (role: PaneColorRole) => slotRoleColor(slot, theme === "light", role);

            it(`${theme}, ${slot.name}: its line clears 3:1 against its own fills and against the chrome it is drawn on`, () => {
                for (const role of FILLS) expect(contrastRatio(c("identity"), c(role)), role).toBeGreaterThanOrEqual(3);
                for (const surface of CHROME[theme]) expect(contrastRatio(c("identity"), surface), surface).toBeGreaterThanOrEqual(3);
            });

            it(`${theme}, ${slot.name}: the theme's text keeps 4.5:1 on each of its fills`, () => {
                for (const role of FILLS) expect(contrastRatio(TEXT[theme], c(role)), role).toBeGreaterThanOrEqual(4.5);
            });
        }
    }

    it("has a tone for every role the hues have", () => {
        for (const theme of ["dark", "light"] as const) {
            for (const neutral of ["black", "white"] as const) {
                expect(Object.keys(NEUTRAL_TONES[theme][neutral]).sort()).toEqual(Object.keys(PANE_COLOR_TOKENS[theme]).sort());
            }
        }
    });
});
