// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The compiled Settings stylesheet keeps what the column layout relies on, where a real browser is not at hand
 * (settings-columns.layout.test.ts measures it in one): the flow rules, the width cap on single sections only, and
 * the rules that ask a row at the pane's old threshold minus the body's 2 x 1rem padding.
 * docs/specs/SPEC_SETTINGS_WIDE_PANE_COLUMNS_2026_11_20.md 3.2 and 3.5.
 */

import { join } from "node:path";
import * as sass from "sass";
import { describe, expect, it } from "vitest";

const css = sass.compile(join(__dirname, "settings.scss")).css;

/** The declarations of the first rule whose selector list is exactly `selector`, or null. */
function rule(selector: string, from = 0): string | null {
    const at = css.indexOf(`${selector} {`, from);
    if (at < 0) return null;
    return css.slice(at, css.indexOf("}", at));
}

describe("settings.scss columns", () => {
    it("flows a section body into columns of at least 26rem, 2rem apart, balanced", () => {
        const body = rule(".settings-section-body");
        expect(body).toContain("column-width: 26rem");
        expect(body).toContain("column-gap: 2rem");
        expect(body).toContain("column-fill: balance");
        expect(rule(".settings-section-body > *")).toContain("break-inside: avoid");
        expect(rule(".settings-section-body > .settings-subheader")).toContain("break-after: avoid");
    });

    it("scrolls in the wrapper, and a single section keeps its own scroll and one column", () => {
        expect(rule(".settings-scroll")).toContain("overflow-y: auto");
        const single = rule(".settings-section-body--single");
        expect(single).toContain("overflow-y: auto");
        expect(single).toContain("column-width: auto");
        expect(rule(".settings-section-body")).not.toContain("overflow");
    });

    it("caps the 720px reading width on single sections only, from a pane of 1024px", () => {
        const cap = css.indexOf("@container settings (min-width: 1024px)");
        expect(cap).toBeGreaterThan(-1);
        const block = css.slice(cap, css.indexOf("\n}", cap));
        expect(block).toContain(".settings-section-body--single");
        expect(block).toContain("max-width: 720px");
        expect(css).not.toMatch(/\.settings-section-body \{[^}]*max-width: 720px/);
    });

    it("levels the headers that start a column, and keeps a group's last row with the row before it, from 56rem", () => {
        const wide = css.indexOf("@container settings (min-width: 56rem)");
        const header = css.indexOf(".settings-section-body:not(.settings-section-body--single) > .settings-subheader {", wide);
        expect(header).toBeGreaterThan(wide);
        expect(css.slice(header, css.indexOf("}", header))).toMatch(/margin-top: var\(--space-4\)[\s\S]*padding-top: 0/);
        const last = css.indexOf(".setting-row:has(+ .settings-subheader)", wide);
        expect(last).toBeGreaterThan(wide);
        expect(css.slice(last, css.indexOf("}", last))).toContain("break-before: avoid");
    });

    it("keeps the pane-keyed narrow rules as they were, for every section", () => {
        // A pane under 56rem is one column: its controls stack at 479px of pane, its swatches wrap at 639, 459 and 259.
        for (const width of [479, 639, 459, 259]) expect(css).toContain(`@container settings (max-width: ${width}px)`);
        expect(css).toContain(".setting-row .ui-field-control {");
    });

    it("asks the row only from a pane of 56rem, where a section has columns, so a narrow pane has no new containers", () => {
        const wide = css.indexOf("@container settings (min-width: 56rem)");
        expect(wide).toBeGreaterThan(-1);
        // The row container is declared inside that query, never outside it.
        expect(css.indexOf("container-name: setting-row")).toBeGreaterThan(wide);
        expect(css).toContain(".settings-section-body:not(.settings-section-body--single) > .setting-row");
        // The rules that ask it: a column's controls stack under 447px (423px for an indented row, which has 24px
        // less), and the widget-color swatches wrap at the pane's 639, 459 and 259 less that indent and the padding.
        for (const width of [447, 423, 583, 403, 203]) {
            expect(css.indexOf(`@container setting-row (max-width: ${width}px)`), `${width}px`).toBeGreaterThan(wide);
        }
    });
});
