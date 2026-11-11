// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Search in an agent pane reads in every theme (SPEC_AGENT_PANE_SEARCH_AND_DIM_MODE_2026_11_06 §4.4, §4.6). A row the
 * query does not match is drawn at `--search-dim-opacity`; the theme's main text over its surface at that
 * opacity keeps 4.5:1 (WCAG 1.4.3), and the value is the lowest step, not below 0.5, that does, so a theme
 * that cannot reach 4.5:1 at 0.5 sets its own. The words the query finds are washed with `--search-mark-bg`
 * (the current row's with `--search-mark-current-bg`) and keep the main text colour, so each wash has to leave
 * 4.5:1 too. Resolved from the theme files the way the cascade does, as pill-colors.test.ts does for the pills.
 */

import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { bundledThemeIds, contrast, over, resolveTheme } from "@/app/element/ui/theme-test-utils";

const AA = 4.5;
const FLOOR = 0.5;
const STEP = 0.05;

const sheet = readFileSync(new URL("../styles/_search.scss", import.meta.url), "utf8");

describe("search dim and marks in every theme", () => {
    for (const id of ["default", ...bundledThemeIds()]) {
        it(`read in ${id}`, () => {
            const { pick, raw, washOf } = resolveTheme(id);
            const surface = pick("main-bg-color")!;
            const text = pick("main-text-color")!;
            expect(surface, "the surface resolves").toBeTruthy();
            expect(text, "the text colour resolves").toBeTruthy();

            const dim = Number(raw("search-dim-opacity"));
            expect(dim, "the theme's dim opacity").toBeGreaterThanOrEqual(FLOOR);
            expect(dim).toBeLessThanOrEqual(1);
            const dimmed = (opacity: number) => contrast(over(text, surface, opacity), surface);
            expect(dimmed(dim), `dimmed text in ${id} at ${dim}`).toBeGreaterThanOrEqual(AA);
            // The lowest step: one below it would fail, unless that is under the floor.
            const lower = Math.round((dim - STEP) * 100) / 100;
            if (lower >= FLOOR) expect(dimmed(lower), `${id} could dim to ${lower}`).toBeLessThan(AA);

            // The words sit on the current row's own wash (`--accent-bg`), the most demanding ground.
            const row = washOf("accent-bg", surface);
            for (const mark of ["search-mark-bg", "search-mark-current-bg"]) {
                expect(contrast(text, washOf(mark, row)), `text on ${mark} in ${id}`).toBeGreaterThanOrEqual(AA);
            }
        });
    }
});

describe("the stylesheet paints what is measured", () => {
    it("dims a row that is not a match, by opacity alone, through the token", () => {
        const rule = sheet.match(/\[data-search-dim\][^{]*\[data-search-match="false"\]\s*\{([^}]*)\}/);
        expect(rule, "the dim rule").toBeTruthy();
        expect(rule![1]).toMatch(/opacity:\s*var\(--search-dim-opacity\)/);
        expect(rule![1]).not.toMatch(/(height|display|visibility|content-visibility)\s*:/);
    });

    it("paints both highlights from the mark tokens", () => {
        expect(sheet).toMatch(/::highlight\(agent-search\)\s*\{[^}]*var\(--search-mark-bg\)/);
        expect(sheet).toMatch(/::highlight\(agent-search-current\)\s*\{[^}]*var\(--search-mark-current-bg\)/);
    });
});
