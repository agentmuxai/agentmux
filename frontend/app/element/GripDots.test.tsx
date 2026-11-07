// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it } from "vitest";
import { GripDots } from "./GripDots";

afterEach(() => cleanup());

// SPEC_DRAG_SPACE_AFFORDANCE_2026_10_13.md §5.6: panes' grips are 4×3, the window's is 4×4.
describe("GripDots", () => {
    const dots = (rows?: 3 | 4) => {
        const { container } = render(() => <GripDots rows={rows} />);
        const svg = container.querySelector("svg")!;
        return { svg, circles: [...svg.querySelectorAll("circle")] };
    };

    it("draws 4×3 dots by default, 1em tall", () => {
        const { svg, circles } = dots();
        expect(circles.length).toBe(12);
        expect(svg.getAttribute("viewBox")).toBe("0 0 16 12");
        expect(svg.getAttribute("height")).toBe("1em");
    });

    it("draws 4×4 dots with rows 4: the same dots and pitch, one row taller", () => {
        const { svg, circles } = dots(4);
        expect(circles.length).toBe(16);
        expect(svg.getAttribute("viewBox")).toBe("0 0 16 16");
        expect(svg.getAttribute("width")).toBe("1.333em");
        expect(svg.getAttribute("height")).toBe("1.333em");
        expect(new Set(circles.map((c) => c.getAttribute("cy")))).toEqual(new Set(["2", "6", "10", "14"]));
    });

    it("keeps the dots clear of the viewBox edge by the same margin on every side", () => {
        for (const rows of [3, 4] as const) {
            const { svg, circles } = dots(rows);
            const box = Number(svg.getAttribute("viewBox")!.split(" ")[3]);
            const ys = circles.map((c) => Number(c.getAttribute("cy")));
            const r = Number(circles[0].getAttribute("r"));
            expect(Math.min(...ys) - r).toBeCloseTo(box - (Math.max(...ys) + r));
            cleanup();
        }
    });
});
