// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The pane border/focus-ring color precedence, after the dead tab-level
// `bg:*` tier was removed
// (docs/specs/SPEC_PANE_COLOR_SYSTEM_CONSOLIDATION_2026_09_20.md §3), and its
// OKLCH roles (pane-color-scheme.ts `identity` focused, `border` unfocused).

import { describe, expect, it } from "vitest";
import { paneBorderForEffectiveColor, paneIdentityForEffectiveColor } from "./pane-color-menu";
import {
    computeBlockActiveBorderColor,
    computeBlockIdentityColor,
    computeBlockTabPillNeutralBg,
    computeFocusRingBorderColor,
    computeMixedPaneHeaderBg,
    computeNonAgentHeaderBg,
} from "./blockframe";

// An uncoloured non-agent pane's header (CPU, terminal…) on a light theme was
// the dark-theme slate hsl(220, 12%, 16%), a solid bar across a light UI.
describe("computeNonAgentHeaderBg", () => {
    it("light theme: the theme surface, the same as that pane's own pill and a mixed pane's tail", () => {
        expect(computeNonAgentHeaderBg(true)).toBe("var(--block-bg-solid-color)");
        expect(computeNonAgentHeaderBg(true)).toBe(computeBlockTabPillNeutralBg({ view: "sysinfo" } as Block["meta"], true));
        expect(computeNonAgentHeaderBg(true)).toBe(computeMixedPaneHeaderBg(true));
    });

    it("dark theme: the fixed neutral header, unchanged", () => {
        expect(computeNonAgentHeaderBg(false)).toBe("hsl(220, 12%, 16%)");
        expect(computeNonAgentHeaderBg(false)).toBe(computeMixedPaneHeaderBg(false));
    });
});

describe("computeFocusRingBorderColor", () => {
    for (const light of [false, true]) {
        const theme = light ? "light" : "dark";

        it(`${theme}: focused, an explicit hue wins over the agent identity color`, () => {
            const meta = { "frame:hue": 120, "frame:activebordercolor": "#ff0000" } as Block["meta"];
            expect(computeFocusRingBorderColor(true, meta, light)).toBe(paneIdentityForEffectiveColor(120, undefined, light));
        });

        it(`${theme}: focused, falls back to the agent identity color, then to nothing`, () => {
            expect(computeFocusRingBorderColor(true, { "frame:activebordercolor": "#ff0000" } as Block["meta"], light)).toBe(
                paneIdentityForEffectiveColor(undefined, "#ff0000", light)
            );
            expect(computeFocusRingBorderColor(true, {} as Block["meta"], light)).toBeUndefined();
            expect(computeFocusRingBorderColor(true, undefined, light)).toBeUndefined();
        });

        it(`${theme}: the focused ring is the block's active colour and the active-tab underline`, () => {
            const meta = { "frame:hue": 200 } as Block["meta"];
            expect(computeFocusRingBorderColor(true, meta, light)).toBe(computeBlockActiveBorderColor(meta, light));
            expect(computeFocusRingBorderColor(true, meta, light)).toBe(computeBlockIdentityColor(meta, light));
        });

        it(`${theme}: unfocused, derived from the hue, else the agent colour, else frame:bordercolor, else nothing`, () => {
            const all = { "frame:hue": 120, "frame:activebordercolor": "#ff0000", "frame:bordercolor": "#110000" };
            expect(computeFocusRingBorderColor(false, all as Block["meta"], light)).toBe(
                paneBorderForEffectiveColor(120, undefined, light)
            );
            expect(
                computeFocusRingBorderColor(false, { "frame:activebordercolor": "#ff0000", "frame:bordercolor": "#110000" } as Block["meta"], light)
            ).toBe(paneBorderForEffectiveColor(undefined, "#ff0000", light));
            expect(computeFocusRingBorderColor(false, { "frame:bordercolor": "#110000" } as Block["meta"], light)).toBe("#110000");
            expect(computeFocusRingBorderColor(false, {} as Block["meta"], light)).toBeUndefined();
        });

        it(`${theme}: a cleared hue (null) falls through to the agent color, not to nothing`, () => {
            const meta = { "frame:hue": null, "frame:activebordercolor": "#00ff00" } as Block["meta"];
            expect(computeFocusRingBorderColor(true, meta, light)).toBe(paneIdentityForEffectiveColor(undefined, "#00ff00", light));
        });
    }

    it("passes a non-hex agent colour through unchanged rather than dropping it", () => {
        expect(computeBlockActiveBorderColor({ "frame:activebordercolor": "rebeccapurple" } as Block["meta"], false)).toBe(
            "rebeccapurple"
        );
    });
});
