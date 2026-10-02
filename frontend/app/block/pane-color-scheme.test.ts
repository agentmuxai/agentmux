// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { AGENT_COLOR_PALETTE } from "@/app/view/agent/agent-color";
import { hueToAgentIdentityColor, PANE_HUE_OPTIONS } from "./pane-color-menu";
import { contrastRatio, identityOklchHue, oklchToHex, PANE_COLOR_TOKENS, paneRoleColor } from "./pane-color-scheme";

// Every identity a pane can have: each "Pane Color" preset and each agent palette colour.
const IDENTITIES: Array<{ name: string; hue?: number; hex?: string }> = [
    ...PANE_HUE_OPTIONS.map((o) => ({ name: `preset ${o.label}`, hue: o.hue })),
    ...AGENT_COLOR_PALETTE.map((hex) => ({ name: `agent ${hex}`, hex })),
];

// The neutral header a mixed pane falls back to in the dark theme
// (blockframe.tsx NON_AGENT_DEFAULT_HEADER_BG, hsl(220, 12%, 16%)).
const DARK_NEUTRAL_HEADER = "#24272e";

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
        const lums = IDENTITIES.map((id) => contrastRatio(paneRoleColor(id.hue, id.hex, false, "identity")!, "#000000"));
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
        it(`${theme} theme: every focused ring stands out more than any unfocused border`, () => {
            const prominence = (role: "identity" | "border") =>
                IDENTITIES.map((id) => contrastRatio(paneRoleColor(id.hue, id.hex, light, role)!, page));
            expect(Math.min(...prominence("identity"))).toBeGreaterThan(Math.max(...prominence("border")) * 1.5);
        });
    }

    it("maps a preset hue and the hex it persists as to the same identity hue", () => {
        // "Pane Color: Blue" persists hsl(240, 65%, 52%) as the agent's ui:color;
        // both must render identically.
        expect(identityOklchHue(240, undefined)).toBeCloseTo(identityOklchHue(undefined, hueToAgentIdentityColor(240))!, 0);
    });

    it("returns undefined for a pane with no colour of its own", () => {
        expect(paneRoleColor(undefined, undefined, false, "pill")).toBeUndefined();
        expect(paneRoleColor(undefined, "not-a-color", false, "pill")).toBeUndefined();
    });

    it("keeps out-of-gamut colours inside sRGB by lowering chroma", () => {
        expect(oklchToHex(0.75, 0.4, 145)).toMatch(/^#[0-9a-f]{6}$/);
        expect(oklchToHex(PANE_COLOR_TOKENS.dark.identity.l, 0, 0)).toMatch(/^#([0-9a-f]{2})\1\1$/);
    });
});
