// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The text of the dividers between sessions reads in every theme (WCAG 1.4.3, 4.5:1): the detail line under
 * "Session continued" and its kin (`.agent-session-outcome-detail`), and the date chip of the history view's day
 * divider (`.agent-day-divider-label`). Both were a white wash (`rgba(255,255,255,0.3)`) that is 1.02:1 on a light
 * theme and 2.7:1 on Dark. They take the theme's secondary text colour; resolved from the theme files the way
 * the cascade does, as search-dim-contrast.test.ts does for the search dim.
 */

import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { bundledThemeIds, contrast, resolveTheme } from "@/app/element/ui/theme-test-utils";

const AA = 4.5;
const sheet = readFileSync(new URL("./_document-nodes.scss", import.meta.url), "utf8");

/** The declarations of the first rule that opens with `selector`. */
function ruleOf(selector: string): string {
    const at = sheet.indexOf(`${selector} {`);
    expect(at, `${selector} has a rule`).toBeGreaterThanOrEqual(0);
    return sheet.slice(at, sheet.indexOf("}", at));
}

describe("session divider text, in every theme", () => {
    for (const [selector, ground] of [
        [".agent-session-outcome-detail", "main-bg-color"],
        [".agent-day-divider-label", "panel-bg-color"],
    ] as const) {
        it(`${selector} paints the secondary text colour`, () => {
            expect(ruleOf(selector)).toMatch(/color:\s*var\(--secondary-text-color\)/);
        });
        for (const id of ["default", ...bundledThemeIds()]) {
            it(`${selector} reads on ${ground} in ${id}`, () => {
                const { pick, washOf } = resolveTheme(id);
                const text = pick("secondary-text-color");
                // The panel colour is a wash over the main surface, not a colour of its own.
                const main = pick("main-bg-color")!;
                const surface = ground === "main-bg-color" ? main : washOf(ground, main);
                expect(text, "the text colour resolves").toBeTruthy();
                expect(surface, "the surface resolves").toBeTruthy();
                expect(contrast(text!, surface), `${selector} in ${id}`).toBeGreaterThanOrEqual(AA);
            });
        }
    }
});
