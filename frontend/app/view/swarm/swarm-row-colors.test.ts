// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { swarmRowColors } from "./swarm-row-colors";

const meta = (m: Record<string, unknown>) => m as Block["meta"];

describe("swarmRowColors", () => {
    it("selection uses the full-strength colour and hover the unselected-pane (dim) one", () => {
        const c = swarmRowColors(meta({ "frame:activebordercolor": "#ef4444", "frame:bordercolor": "#7a2323" }));
        expect(c.active).toBe("#ef4444");
        expect(c.hover).toBe("#7a2323");
    });

    it("an explicit frame:hue wins for both, exactly as it does on the pane", () => {
        const c = swarmRowColors(
            meta({ "frame:hue": 200, "frame:activebordercolor": "#ef4444", "frame:bordercolor": "#7a2323" })
        );
        expect(c.active).not.toBe("#ef4444");
        expect(c.hover).not.toBe("#7a2323");
        expect(c.hover).toBeTruthy();
        expect(c.hover).not.toBe(c.active);
    });

    it("a block with no colour, or no meta at all, yields undefined so the CSS fallbacks apply", () => {
        expect(swarmRowColors(meta({}))).toEqual({ active: undefined, hover: undefined });
        expect(swarmRowColors(undefined)).toEqual({ active: undefined, hover: undefined });
    });
});
