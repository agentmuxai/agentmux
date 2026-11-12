// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, render } from "@solidjs/testing-library";
import type { JSX } from "solid-js";
import { afterEach, describe, expect, it } from "vitest";
import { DeclIconButton } from "./ui/DeclIconButton";
import { PaneMergeGlyph, PaneMinimizeGlyph, PaneRestoreGlyph } from "./pane-control-glyphs";

afterEach(cleanup);

describe("PaneRestoreGlyph", () => {
    it("is an 18-unit, 1em, currentColor SVG with no fixed colour", () => {
        const { container } = render(() => <PaneRestoreGlyph />);
        const svg = container.querySelector("svg")!;
        expect(svg.getAttribute("viewBox")).toBe("0 0 18 18");
        expect(svg.getAttribute("width")).toBe("1em");
        expect(svg.getAttribute("height")).toBe("1em");
        expect(svg.getAttribute("stroke")).toBe("currentColor");
        expect(svg.getAttribute("aria-hidden")).toBe("true");
        // Every paint is currentColor (or none): a hard-coded colour would ignore the theme and the hover rules.
        for (const el of Array.from(svg.querySelectorAll("*"))) {
            for (const attr of ["fill", "stroke"]) {
                const v = el.getAttribute(attr);
                if (v !== null) expect(["currentColor", "none"]).toContain(v);
            }
        }
    });

    it("keeps its ink inside the 18px box, centred like window-maximize (17.7 x 15.7, y 1.15 to 16.85)", () => {
        const { container } = render(() => <PaneRestoreGlyph />);
        const frame = container.querySelector("rect")!;
        const sw = Number(container.querySelector("svg")!.getAttribute("stroke-width"));
        const x = Number(frame.getAttribute("x")), y = Number(frame.getAttribute("y"));
        const w = Number(frame.getAttribute("width")), h = Number(frame.getAttribute("height"));
        const top = y - sw / 2, bottom = y + h + sw / 2, left = x - sw / 2, right = x + w + sw / 2;
        expect(left).toBeGreaterThanOrEqual(0);
        expect(right).toBeLessThanOrEqual(18);
        expect(top).toBeGreaterThanOrEqual(0);
        expect(bottom).toBeLessThanOrEqual(18);
        expect(right - left).toBeCloseTo(17.7, 0);
        expect(bottom - top).toBeCloseTo(15.7, 0);
        expect((top + bottom) / 2).toBeCloseTo(9, 1);
    });

    it("renders inside a DeclIconButton as the icon, not through a Font Awesome <i>", () => {
        const decl = { elemtype: "iconbutton", icon: <PaneRestoreGlyph />, title: "Restore" } as IconButtonDecl;
        const { container } = render(() => <DeclIconButton decl={decl} className="block-frame-minimize" />);
        const btn = container.querySelector("button")!;
        expect(btn.getAttribute("title")).toBe("Restore");
        expect(btn.querySelector("svg.pane-restore-glyph")).not.toBeNull();
        expect(btn.querySelector("i")).toBeNull();
        expect(btn.innerHTML).not.toContain("fa-window-restore");
    });
});

describe("the minimize pair's glyphs", () => {
    const draw = (el: () => JSX.Element) => render(el).container.querySelector("svg")!;

    it("share one box, and draw with currentColor", () => {
        const solid = draw(() => <PaneMinimizeGlyph />);
        const broken = draw(() => <PaneMergeGlyph />);
        for (const svg of [solid, broken]) {
            expect(svg.getAttribute("viewBox")).toBe("0 0 16 2");
            expect(svg.getAttribute("width")).toBe("16");
            expect(svg.getAttribute("height")).toBe("2");
            expect(svg.getAttribute("fill")).toBe("currentColor");
        }
        expect(solid.getAttribute("viewBox")).toBe(broken.getAttribute("viewBox"));
    });

    it("the solid one is one 16 px bar; the broken one has three 4 px segments and two 2 px gaps", () => {
        const bars = (svg: SVGElement) =>
            [...svg.querySelectorAll("rect")].map((r) => [Number(r.getAttribute("x")), Number(r.getAttribute("width"))]);
        expect(bars(draw(() => <PaneMinimizeGlyph />))).toEqual([[0, 16]]);
        const broken = bars(draw(() => <PaneMergeGlyph />));
        expect(broken).toEqual([[0, 4], [6, 4], [12, 4]]);
        const gaps = broken.slice(1).map(([x], i) => x - (broken[i][0] + broken[i][1]));
        expect(gaps).toEqual([2, 2]);
        // The same extent as the solid bar: first edge to last edge.
        expect(broken[2][0] + broken[2][1]).toBe(16);
    });
});
