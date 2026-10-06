// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { preparePlotSvg } from "./sysinfo-plot";

describe("preparePlotSvg", () => {
    it("drops Plot's per-chart stylesheet and lets the drawing stretch with its SVG", () => {
        const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
        svg.appendChild(document.createElementNS("http://www.w3.org/2000/svg", "style")).textContent = ":where(.x) {}";
        const g = svg.appendChild(document.createElementNS("http://www.w3.org/2000/svg", "g"));

        preparePlotSvg(svg);

        expect(svg.querySelector("style")).toBeNull();
        expect(svg.contains(g)).toBe(true);
        expect(svg.getAttribute("preserveAspectRatio")).toBe("none");
    });
});
