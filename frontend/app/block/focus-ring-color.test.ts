// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The pane border/focus-ring color precedence, after the dead tab-level
// `bg:*` tier was removed
// (docs/specs/SPEC_PANE_COLOR_SYSTEM_CONSOLIDATION_2026_09_20.md §3).

import { describe, expect, it } from "vitest";
import { hueToActiveBorder, hueToBorder } from "./pane-color-menu";
import { computeBlockActiveBorderColor, computeFocusRingBorderColor } from "./blockframe";

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
