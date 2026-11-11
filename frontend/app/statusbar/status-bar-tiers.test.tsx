// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";

import { fitToWidth } from "@/app/element/width-fit";
import { FIT_TIERS } from "./status-bar-fit";

describe("the fit tiers", () => {
    // jsdom has no layout: the bar overflows while its `data-fit` is not yet one of `fits`.
    const barFittingFrom = (fits: string[]) => {
        const bar = document.createElement("div");
        Object.defineProperty(bar, "scrollWidth", { get: () => (fits.includes(bar.getAttribute("data-fit") ?? "") ? 100 : 300) });
        Object.defineProperty(bar, "clientWidth", { get: () => 100 });
        return bar;
    };

    it("begins with a tier that sheds only the slots that drop at 0, before any other readout goes", () => {
        expect(FIT_TIERS[0]).toBe("0");
        const bar = barFittingFrom(["0", "1", "2", "3", "4"]);
        fitToWidth(bar, FIT_TIERS);
        expect(bar.getAttribute("data-fit")).toBe("0");
    });

    it("goes on to the next tier when that is not enough, and leaves the bar whole when it fits", () => {
        const next = barFittingFrom(["1", "2", "3", "4"]);
        fitToWidth(next, FIT_TIERS);
        expect(next.getAttribute("data-fit")).toBe("1");
        const whole = barFittingFrom(["", "0", "1", "2", "3", "4"]);
        fitToWidth(whole, FIT_TIERS);
        expect(whole.hasAttribute("data-fit")).toBe(false);
    });
});
