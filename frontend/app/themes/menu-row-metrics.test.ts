// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The rows of every menu share one set of metrics (`--menu-row-*`, `--menu-icon-*`, `--menu-pad` and
 * `--menu-divider-margin` in theme.scss): the `.menu` family (element/flyoutmenu.scss) and the Solid context menu
 * (components/context-menu.scss) read them, so a row is as tall, as padded and as wide in its icon column in either,
 * and a change to a metric is made once. Spec: docs/specs/SPEC_MENU_SURFACES_STAND_OUT_2026_11_17.md, follow-up #47922.
 */
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const read = (rel: string) => readFileSync(new URL(`../${rel}`, import.meta.url), "utf8");

/** The declarations of the first top-level rule (selector at the start of a line) named exactly `selector`. */
function rule(css: string, selector: string): string {
    const open = css.search(new RegExp(`^${selector.replace(/[.]/g, "\.")} \{`, "m"));
    expect(open, selector).toBeGreaterThanOrEqual(0);
    return css.slice(open, css.indexOf("\n}", open));
}

const flyout = read("element/flyoutmenu.scss");
const ctx = read("components/context-menu.scss");
const theme = read("theme.scss");

describe("the shared menu row metrics", () => {
    it("are defined once, in theme.scss", () => {
        for (const token of ["menu-pad", "menu-row-gap", "menu-row-pad-y", "menu-row-pad-x", "menu-row-font-size", "menu-icon-col", "menu-icon-gap", "menu-divider-margin"]) {
            expect(theme, token).toMatch(new RegExp(`--${token}:`));
        }
    });

    it("are what a `.menu` row, its icon and its divider read", () => {
        const item = rule(flyout, ".menu-item");
        expect(item).toContain("padding: var(--menu-row-pad-y) var(--menu-row-pad-x)");
        expect(item).toContain("font-size: var(--menu-row-font-size)");
        expect(rule(flyout, ".menu-item-icon")).toContain("width: var(--menu-icon-col)");
        expect(rule(flyout, ".menu-item-icon")).toContain("margin-right: var(--menu-icon-gap)");
        expect(rule(flyout, ".menu-divider")).toContain("margin: var(--menu-divider-margin)");
        expect(rule(flyout, ".menu")).toContain("gap: var(--menu-row-gap)");
    });

    it("are what a context-menu row and divider read, and the same frame padding", () => {
        expect(rule(ctx, ".ctx-menu")).toContain("padding: var(--menu-pad)");
        expect(rule(ctx, ".ctx-menu")).toContain("font-size: var(--menu-row-font-size)");
        expect(rule(ctx, ".ctx-menu")).toContain("gap: var(--menu-row-gap)");
        expect(rule(ctx, ".ctx-menu-item")).toContain("padding: var(--menu-row-pad-y) var(--menu-row-pad-x)");
        expect(rule(ctx, ".ctx-menu-sep")).toContain("margin: var(--menu-divider-margin)");
        expect(read("element/menu-frame.scss")).toContain("padding: var(--menu-pad)");
    });

    it("leave neither stylesheet a literal row padding of its own", () => {
        expect(rule(ctx, ".ctx-menu-item")).not.toMatch(/padding:\s*\d/);
        expect(rule(flyout, ".menu-item")).not.toMatch(/padding:\s*\d/);
    });
});
