// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Pins the fix for REPORT_TAB_PANES_OFFSET_HALF_WINDOW_2026_09_24.md: a
 * `focus()` without `preventScroll` scrolled an `overflow: hidden`
 * `.tile-layout` by half its height, shifting every pane of the tab up with no
 * way for the user to scroll it back.
 *
 * Grep-shaped, same as `agent-view-dispatch-via-pane-model.test.ts`: jsdom has
 * no layout, so it can't reproduce the scroll itself. What it can do is fail
 * if either guard is removed.
 */

import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

const TILE_LAYOUT_SCSS = join(__dirname, "..", "lib", "tilelayout.scss");
const AGENT_MODEL = join(__dirname, "..", "..", "app", "view", "agent", "agent-model.ts");

/** The declarations directly inside the top-level `.tile-layout {` rule, before its first nested block. */
function tileLayoutOwnDeclarations(scss: string): string {
    const start = scss.search(/^\.tile-layout\s*\{/m);
    expect(start).toBeGreaterThanOrEqual(0);
    const body = scss.slice(scss.indexOf("{", start) + 1);
    return body.slice(0, body.indexOf("{"));
}

describe(".tile-layout can't be scrolled by focus", () => {
    it("uses overflow: clip, which makes it not a scroll container", () => {
        const own = tileLayoutOwnDeclarations(readFileSync(TILE_LAYOUT_SCSS, "utf8"));
        expect(own).toMatch(/^\s*overflow:\s*clip;/m);
        // `hidden` is still a scroll container: focus() and scrollIntoView() can scroll it.
        expect(own).not.toMatch(/^\s*overflow:\s*hidden;/m);
    });
});

describe("the agent pane focuses its composer without scrolling ancestors", () => {
    it("giveFocus() passes preventScroll", () => {
        const src = readFileSync(AGENT_MODEL, "utf8");
        const start = src.indexOf("giveFocus(): boolean {");
        expect(start).toBeGreaterThanOrEqual(0);
        const body = src.slice(start, src.indexOf("\n    }", start));
        expect(body).toMatch(/\.focus\(\{\s*preventScroll:\s*true\s*\}\)/);
    });
});
