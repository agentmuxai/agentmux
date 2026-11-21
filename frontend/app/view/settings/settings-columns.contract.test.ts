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

    it("makes a flow section's rows containers, and asks them at the pane's old thresholds minus 32px", () => {
        expect(css).toContain(".settings-section-body:not(.settings-section-body--single) > .setting-row");
        expect(css).toContain("container-name: setting-row");
        // 479 (controls fill the row) and 639, 459, 259 (widget-color swatches), each less the body's 2 x 16px; the
        // widget-color row is indented 24px, so its thresholds are lower by that as well: 583, 403, 203.
        for (const width of [447, 423, 583, 403, 203]) expect(css).toContain(`@container setting-row (max-width: ${width}px)`);
        // The pane-keyed rules that remain are the single sections', the device cards and the widget list's.
        expect(css.match(/@container settings \(max-width: 479px\)/g)?.length).toBe(2);
        expect(css).not.toContain("@container settings (max-width: 639px)");
    });
});
