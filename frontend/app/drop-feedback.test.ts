// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The Pane Tab strip's drop-target hover lands inverted and stays so until the
// hover ends: docs/specs/SPEC_DROP_TARGET_LANDS_INVERTED_2026_10_21.md. jsdom
// runs no CSS animations, so this compiles the real stylesheets and checks the
// rules that make it so.

import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import * as sass from "sass";
import { BLINK_COST, BLINK_COUNT, BLINK_PHASE_MS, CHANGES_PER_SECOND } from "./element/drop-blink-clock";
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
        const header = rule(strip, ".pane-header--foreign-hover");
        const m = /tab-drop-invert-hold (\d+)ms steps\(1, end\) (\d+) forwards/.exec(header);
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
        const header = rule(strip, ".pane-header--foreign-hover");
        expect(header).toMatch(/tab-drop-pulse 0\.4s /);
    });

    it("a hover that finds room for its two changes but not a burst lands inverted without the blink, tabs cancelled", () => {
        const header = rule(strip, ".pane-header--foreign-hover.pane-header--no-blink");
        expect(header).toContain("filter: invert(1)");
        expect(header).not.toContain("tab-drop-invert-hold");
        const tabs = rule(strip, ".pane-header--foreign-hover.pane-header--no-blink .block-frame-default-header-tabstrip");
        expect(tabs).toContain("animation: none");
        expect(tabs).toContain("filter: invert(1)");
        expect(rule(strip, ".pane-tab-strip--foreign-hover.pane-tab-strip--no-blink")).toContain("filter: invert(1)");
    });

    it("a hover that finds the flash budget spent shows the outline alone, tabs and all, in both motion tiers", () => {
        for (const prefix of ["", ":root[data-motion-reduce~=feedback] "]) {
            const header = rule(strip, prefix + ".pane-header--foreign-hover.pane-header--no-invert");
            expect(header).toContain("filter: none");
            expect(header).toContain("tab-drop-pulse");
            expect(header).not.toContain("tab-drop-invert-hold");
            const tabs = rule(strip, prefix + ".pane-header--foreign-hover.pane-header--no-invert .block-frame-default-header-tabstrip");
            expect(tabs).toContain("animation: none");
            expect(tabs).toContain("filter: none");
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

    it("the pane header drop zone runs the hold animation, and keeps the pulse", () => {
        const header = rule(strip, ".pane-header--foreign-hover");
        expect(header).toContain("tab-drop-invert-hold 200ms steps(1, end) 2 forwards");
        expect(header).not.toContain("tab-drop-invert-strobe");
        expect(header).toContain("tab-drop-pulse");
    });

    it("the tab strip used as its own drop zone does the same", () => {
        const own = rule(strip, ".pane-tab-strip--foreign-hover");
        expect(own).toContain("tab-drop-invert-hold 200ms steps(1, end) 2 forwards");
    });

    it("the tabs inside run the same animation, so the pair cancels while the hover lasts", () => {
        const tabs = rule(strip, ".pane-header--foreign-hover .block-frame-default-header-tabstrip");
        expect(tabs).toContain("animation: tab-drop-invert-hold 200ms steps(1, end) 2 forwards");
    });

    it("the reduced-motion treatment holds the negative without blinking", () => {
        const header = rule(strip, ':root[data-motion-reduce~=feedback] .pane-header--foreign-hover');
        expect(header).toContain("filter: invert(1)");
        expect(header).toContain("animation: tab-drop-pulse");
        const tabs = rule(strip, ':root[data-motion-reduce~=feedback] .pane-header--foreign-hover .block-frame-default-header-tabstrip');
        expect(tabs).toContain("animation: none");
        expect(tabs).toContain("filter: invert(1)");
    });

    it("the Window Tab bar keeps the flash that settles back to normal", () => {
        const tabbar = compile("tab/tabbar.scss");
        expect(tabbar).toContain("tab-drop-invert-strobe 400ms steps(1, end) 2");
        expect(tabbar).not.toContain("tab-drop-invert-hold");
    });
});
