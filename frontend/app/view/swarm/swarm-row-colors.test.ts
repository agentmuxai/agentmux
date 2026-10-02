// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { computeFocusRingBorderColor } from "@/app/block/blockframe";
import { swarmRowColors } from "./swarm-row-colors";

const meta = (m: Record<string, unknown>) => m as Block["meta"];

describe("swarmRowColors", () => {
    for (const light of [false, true]) {
        const theme = light ? "light" : "dark";

        it(`${theme}: selection uses the focused-ring colour and hover the unselected-pane one`, () => {
            const m = meta({ "frame:activebordercolor": "#ef4444", "frame:bordercolor": "#7a2323" });
            const c = swarmRowColors(m, light);
            expect(c.active).toBe(computeFocusRingBorderColor(true, m, light));
            expect(c.hover).toBe(computeFocusRingBorderColor(false, m, light));
            expect(c.active).toBeTruthy();
            expect(c.hover).toBeTruthy();
            expect(c.hover).not.toBe(c.active);
        });

        it(`${theme}: an explicit frame:hue wins for both, exactly as it does on the pane`, () => {
            const withHue = swarmRowColors(
                meta({ "frame:hue": 200, "frame:activebordercolor": "#ef4444", "frame:bordercolor": "#7a2323" }),
                light
            );
            const agentOnly = swarmRowColors(meta({ "frame:activebordercolor": "#ef4444" }), light);
            expect(withHue.active).not.toBe(agentOnly.active);
            expect(withHue.hover).not.toBe(agentOnly.hover);
            expect(withHue.hover).not.toBe(withHue.active);
        });

        it(`${theme}: a block with no colour, or no meta at all, yields undefined so the CSS fallbacks apply`, () => {
            expect(swarmRowColors(meta({}), light)).toEqual({ active: undefined, hover: undefined });
            expect(swarmRowColors(undefined, light)).toEqual({ active: undefined, hover: undefined });
        });
    }
});
