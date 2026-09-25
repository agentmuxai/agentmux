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
const COMPOSER_FOCUS = join(__dirname, "..", "..", "app", "view", "agent", "composer-focus.ts");

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

/** The body of the function/method that starts at `signature`, up to its closing brace at `indent`. */
function bodyOf(src: string, signature: string, indent: string): string {
    const start = src.indexOf(signature);
    expect(start).toBeGreaterThanOrEqual(0);
    return src.slice(start, src.indexOf(`\n${indent}}`, start));
}

const PREVENT_SCROLL = /\.focus\(\{\s*preventScroll:\s*true\s*\}\)/;

describe("the agent pane focuses its composer without scrolling ancestors", () => {
    it("giveFocus() passes preventScroll, directly or through focusComposer()", () => {
        const body = bodyOf(readFileSync(AGENT_MODEL, "utf8"), "giveFocus(): boolean {", "    ");
        if (PREVENT_SCROLL.test(body)) return;
        // giveFocus() delegates to composer-focus.ts
        // (SPEC_AGENT_PANE_HOVER_CLOSE_FOCUS_REFINEMENTS_2026_09_23.md §3), so the
        // guard must hold there instead.
        expect(body).toMatch(/\bfocusComposer\(/);
        const composer = bodyOf(
            readFileSync(COMPOSER_FOCUS, "utf8"),
            "export function focusComposer(ta: HTMLTextAreaElement): boolean {",
            ""
        );
        expect(composer).toMatch(PREVENT_SCROLL);
        // …and no other focus() call in it that could scroll.
        expect(composer.match(/\.focus\(/g)?.length).toBe(1);
    });
});
