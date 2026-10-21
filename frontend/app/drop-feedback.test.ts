// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The Pane Tab strip's drop-target hover lands inverted and stays so until the
// hover ends: docs/specs/SPEC_DROP_TARGET_LANDS_INVERTED_2026_10_21.md. jsdom
// runs no CSS animations, so this compiles the real stylesheets and checks the
// rules that make it so.

import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import * as sass from "sass";
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
        expect(header).toContain("tab-drop-invert-hold 400ms steps(1, end) forwards");
        expect(header).not.toContain("tab-drop-invert-strobe");
        expect(header).toContain("tab-drop-pulse");
    });

    it("the tab strip used as its own drop zone does the same", () => {
        const own = rule(strip, ".pane-tab-strip--foreign-hover");
        expect(own).toContain("tab-drop-invert-hold 400ms steps(1, end) forwards");
    });

    it("the tabs inside run the same animation, so the pair cancels while the hover lasts", () => {
        const tabs = rule(strip, ".pane-header--foreign-hover .block-frame-default-header-tabstrip");
        expect(tabs).toContain("animation: tab-drop-invert-hold 400ms steps(1, end) forwards");
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
