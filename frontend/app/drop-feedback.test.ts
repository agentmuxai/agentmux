// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The Pane Tab strip's drop-target hover lands inverted and stays so until the
// hover ends: docs/specs/SPEC_DROP_TARGET_LANDS_INVERTED_2026_10_21.md. jsdom
// runs no CSS animations, so this compiles the real stylesheets and checks the
// rules that make it so.

import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import * as sass from "sass";
import {
    BLINK_COST,
    BLINK_COUNT,
    BLINK_PHASE_MS,
    CHANGES_PER_SECOND,
    EDGE_STRIP_PX,
    EDGE_STRIP_SHARE,
    edgeFlashArea,
    EXEMPT_AREA_PX2,
    HEADER_PX,
} from "./element/drop-blink-clock";
import { describe, expect, it } from "vitest";

const APP = dirname(fileURLToPath(import.meta.url));

function compile(file: string): string {
    return sass.compile(join(APP, file), { loadPaths: [APP] }).css;
}

/** The declarations of the first rule whose selector is exactly `selector`. */
function rule(css: string, selector: string): string {
    const start = css.split("\n").findIndex((line) => line.trim() === `${selector} {`);
    if (start < 0) throw new Error(`no rule for ${selector}`);
    const lines = css.split("\n").slice(start + 1);
    const end = lines.findIndex((line) => line.trim() === "}");
    return lines.slice(0, end).join("\n");
}

describe("drop target hover that lands inverted", () => {
    const strip = compile("element/PaneTabStrip.scss");
    const indicators = compile("drag/drop-indicators.scss");

    it("a hover costs at most three flashes in any one second, counting its entry and its exit (WCAG 2.3.1)", () => {
        const own = rule(strip, ".pane-tab-strip--foreign-hover");
        const m = /tab-drop-invert-hold (\d+)ms steps\(1, end\) (\d+) forwards/.exec(own);
        expect(m).not.toBeNull();
        const [cycleMs, count] = [Number(m![1]), Number(m![2])];
        // The stylesheet and the clock that budgets it must agree.
        expect(count).toBe(BLINK_COUNT);
        expect(cycleMs / 2).toBe(BLINK_PHASE_MS);
        expect(cycleMs).toBeLessThanOrEqual(200);
        // Changes in the worst second: entering (1), two per blink, leaving (1); the burst is the only flashing.
        const burstMs = cycleMs * count;
        expect(burstMs).toBeLessThanOrEqual(1000);
        const changes = 1 + 2 * count + 1;
        expect(changes).toBeLessThanOrEqual(CHANGES_PER_SECOND);
        expect(changes).toBe(BLINK_COST);
    });

    it("the outline pulse runs at 0.4 s a direction, twice as fast as before", () => {
        expect(rule(strip, ".pane-tab-strip--foreign-hover")).toMatch(/tab-drop-pulse 0\.4s /);
    });

    it("a strip used as its own drop zone that finds room for its two changes but not a burst lands inverted without the blink", () => {
        expect(rule(strip, ".pane-tab-strip--foreign-hover.pane-tab-strip--no-blink")).toContain("filter: invert(1)");
    });

    it("a strip used as its own drop zone that finds the flash budget spent shows the outline alone", () => {
        for (const prefix of ["", ":root[data-motion-reduce~=feedback] "]) {
            const own = rule(strip, prefix + ".pane-tab-strip--foreign-hover.pane-tab-strip--no-invert");
            expect(own).toContain("filter: none");
        }
    });

    it("the hold keyframes end inverted, after a blink", () => {
        const start = indicators.indexOf("@keyframes tab-drop-invert-hold");
        expect(start).toBeGreaterThanOrEqual(0);
        const block = indicators.slice(start, indicators.indexOf("\n}", start));
        const steps = [...block.matchAll(/(\d+)%\s*\{\s*filter:\s*invert\((\d)\)/g)].map((m) => [m[1], m[2]]);
        expect(steps).toEqual([
            ["0", "1"],
            ["50", "0"],
            ["100", "1"],
        ]);
    });

    it("the tab strip used as its own drop zone runs the hold animation", () => {
        const own = rule(strip, ".pane-tab-strip--foreign-hover");
        expect(own).toContain("tab-drop-invert-hold 200ms steps(1, end) 2 forwards");
    });

    it("the Window Tab bar keeps the flash that settles back to normal", () => {
        const tabbar = compile("tab/tabbar.scss");
        expect(tabbar).toContain("tab-drop-invert-strobe 400ms steps(1, end) 2");
        expect(tabbar).not.toContain("tab-drop-invert-hold");
    });
});

describe("the pane header's edge glow", () => {
    const strip = compile("element/PaneTabStrip.scss");

    it("the header itself is not inverted, outlined or counter-inverted", () => {
        const header = rule(strip, ".pane-header--foreign-hover");
        expect(header).not.toContain("filter");
        expect(header).not.toContain("outline");
        expect(strip).not.toContain(".pane-header--foreign-hover .block-frame-default-header-tabstrip");
    });

    it("two strips at the header's ends, in the pane's identity colour, with the blink and then the breath", () => {
        for (const [side, dir] of [
            ["before", "to right"],
            ["after", "to left"],
        ]) {
            const edge = rule(strip, `.pane-header--foreign-hover::${side}`);
            expect(edge).toContain("pointer-events: none");
            expect(edge).toContain(`linear-gradient(${dir}, var(--block-identity-color, var(--drop-target-border)), transparent)`);
            expect(edge).toContain("am-blink 200ms steps(1, end) 2 forwards");
            expect(edge).toContain("am-pulse 2.4s ease-in-out 400ms infinite");
            expect(edge).toContain("--pulse-min: 0.55");
        }
    });

    it("the strip's blink is the clock's: same cycle and count as the hold, and it ends lit", () => {
        const edge = rule(strip, ".pane-header--foreign-hover::before");
        const m = /am-blink (\d+)ms steps\(1, end\) (\d+) forwards/.exec(edge)!;
        expect(Number(m[2])).toBe(BLINK_COUNT);
        expect(Number(m[1]) / 2).toBe(BLINK_PHASE_MS);
        const keyframes = compile("_keyframes.scss");
        const start = keyframes.indexOf("@keyframes am-blink");
        const block = keyframes.slice(start, keyframes.indexOf("\n}", start));
        // Lit at both ends, dark in the middle: with two iterations, lit, dark, lit, dark, then held lit.
        const steps = [...block.matchAll(/(\d+)%(?:,\s*(\d+)%)?\s*\{\s*opacity:\s*(\d)/g)].map((x) => [x[1], x[2] ?? "", x[3]]);
        expect(steps).toEqual([
            ["0", "100", "1"],
            ["50", "", "0"],
        ]);
    });

    it("the strip's width is the clock's: the smaller of a quarter of the header and 40 px", () => {
        const edge = rule(strip, ".pane-header--foreign-hover::before");
        expect(edge).toContain(`width: min(${EDGE_STRIP_SHARE * 100}%, ${EDGE_STRIP_PX}px)`);
    });

    it("with its budget short, or under reduced motion, the strips are lit without the blink", () => {
        expect(rule(strip, ".pane-header--foreign-hover.pane-header--no-blink::before")).not.toContain("tab-drop-edge-blink");
        for (const side of ["before", "after"]) {
            const reduced = rule(strip, `:root[data-motion-reduce~=feedback] .pane-header--foreign-hover::${side}`);
            expect(reduced).toContain("animation: none");
        }
    });

    it("every header's blink is outside the flash rule: 2,000 px wide, and at 200% chrome zoom", () => {
        for (const [w, zoom] of [
            [330, 1],
            [1000, 1],
            [2000, 1],
            [4000, 1],
            [1000, 2],
            [2000, 2],
        ]) {
            expect(edgeFlashArea(w, HEADER_PX * zoom), `${w} px at ${zoom}x`).toBeLessThanOrEqual(EXEMPT_AREA_PX2);
        }
        // The rule is by area: past 2.03 it no longer holds, and the budget applies as a backstop.
        expect(edgeFlashArea(4000, HEADER_PX * 2.1)).toBeGreaterThan(EXEMPT_AREA_PX2);
    });
});
