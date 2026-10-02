// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The pane border/focus-ring color precedence, after the dead tab-level
// `bg:*` tier was removed
// (docs/specs/SPEC_PANE_COLOR_SYSTEM_CONSOLIDATION_2026_09_20.md §3).

import { describe, expect, it } from "vitest";
import { hueToActiveBorder, hueToBorder } from "./pane-color-menu";
import {
    computeBlockActiveBorderColor,
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
    it("focused: an explicit hue wins over the agent identity color", () => {
        const meta = { "frame:hue": 120, "frame:activebordercolor": "#ff0000" } as Block["meta"];
        expect(computeFocusRingBorderColor(true, meta)).toBe(hueToActiveBorder(120));
    });

    it("focused: falls back to the agent identity color, then to nothing", () => {
        expect(computeFocusRingBorderColor(true, { "frame:activebordercolor": "#ff0000" } as Block["meta"])).toBe("#ff0000");
        expect(computeFocusRingBorderColor(true, {} as Block["meta"])).toBeUndefined();
        expect(computeFocusRingBorderColor(true, undefined)).toBeUndefined();
    });

    it("focused: is exactly the block's own active color (one rule for ring and pill)", () => {
        const meta = { "frame:hue": 200 } as Block["meta"];
        expect(computeFocusRingBorderColor(true, meta)).toBe(computeBlockActiveBorderColor(meta));
    });

    it("unfocused: hue-derived dim color, then frame:bordercolor, then nothing", () => {
        expect(computeFocusRingBorderColor(false, { "frame:hue": 120, "frame:bordercolor": "#110000" } as Block["meta"])).toBe(
            hueToBorder(120)
        );
        expect(computeFocusRingBorderColor(false, { "frame:bordercolor": "#110000" } as Block["meta"])).toBe("#110000");
        expect(computeFocusRingBorderColor(false, {} as Block["meta"])).toBeUndefined();
    });

    it("a cleared hue (null) falls through to the agent color, not to nothing", () => {
        const meta = { "frame:hue": null, "frame:activebordercolor": "#00ff00" } as Block["meta"];
        expect(computeFocusRingBorderColor(true, meta)).toBe("#00ff00");
    });
});
