// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { applyTransparencyToColor } from "./termutil";

/** What colord's `colord(c).alpha(1 - t).toHex()` gave, which this replaced. */
describe("applyTransparencyToColor", () => {
    it.each([
        ["#1e1e2e", 0.5, "#1e1e2e80"],
        ["#fff", 0.25, "#ffffffbf"],
        ["#11223344", 0.5, "#11223380"],
        ["#abcdef", 0, "#abcdef"],
        ["rgb(30, 30, 46)", 0.5, "#1e1e2e80"],
        ["rgba(30 30 46 / 0.2)", 0.5, "#1e1e2e80"],
    ])("%s at %s → %s", (color, transparency, expected) => {
        expect(applyTransparencyToColor(color, transparency)).toBe(expected);
    });

    it("returns a colour it can't read as it is", () => {
        expect(applyTransparencyToColor("not-a-colour", 0.5)).toBe("not-a-colour");
    });
});
