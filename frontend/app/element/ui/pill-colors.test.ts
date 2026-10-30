// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The shared `Pill` reads in every theme. A tone is a wash of the theme's own
 * semantic colour (`--success-bg`, `--warning-bg`, `--error-bg`,
 * `--accent-bg`, theme.scss) with the main text colour on it. This
 * reads the theme files themselves, resolves the tokens the way the cascade
 * does (the theme's value, then the light polarity's, then `:root`'s), and
 * checks the main text colour against the wash laid over the theme's surface, so a later
 * theme tweak can't push a pill below the bar unnoticed
 * (docs/reports/REPORT_TOOLCHAIN_PANE_THEME_INTEGRATION_2026_10_19.md).
 */

import { describe, expect, it } from "vitest";
import { bundledThemeIds, contrast, resolveTheme } from "./theme-test-utils";

// WCAG AA for body text. The pills are 11px, so 4.5:1 is the bar. The themes'
// own success and warning colours do not reach it as text on a wash of
// themselves (about 2.6:1 on the light themes, and 3.4 to 4.4 on several dark
// ones), which is why the pill text is the main text colour and the semantic
// colour is the wash, the line and the icon.
const AA = 4.5;

describe("the shared pills", () => {
    const themes = bundledThemeIds();

    it("cover every theme", () => {
        expect(themes.length).toBeGreaterThanOrEqual(12);
    });

    for (const id of ["default", ...themes]) {
        it(`read in ${id}`, () => {
            // Tokens resolve the way the cascade does (theme-test-utils.ts). A wash token is
            // read from the token itself, so a theme that overrides it, or a shared change of
            // its percentage, is what gets measured.
            const { pick, washOf } = resolveTheme(id);
            const surface = pick("main-bg-color");
            expect(surface, "the theme's surface resolves").toBeTruthy();
            const text = pick("main-text-color");
            expect(text, "the theme's text colour resolves").toBeTruthy();
            // The plain pill (checking, managed, up to date, optional): the secondary
            // text colour on `--tint-3`.
            const secondary = pick("secondary-text-color");
            expect(secondary, "the secondary text colour resolves").toBeTruthy();
            const plain = contrast(secondary!, washOf("tint-3", surface!));
            expect(plain, `plain pill text in ${id}`).toBeGreaterThanOrEqual(AA);
            for (const kind of ["success", "warning", "error", "accent"] as const) {
                const wash = washOf(`${kind}-bg`, surface!);
                expect(contrast(text!, wash), `pill text on the ${kind} wash in ${id}`).toBeGreaterThanOrEqual(AA);
            }
        });
    }
});
