// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A picked pill (Ctrl/Cmd+click) draws its highlight inside itself and changes nothing about its
 * box or its neighbours' (the operator, 2026-10-27: the border grew and the pill jerked). Read as
 * stylesheet text, like PaneTabStrip.progress-style.test.ts; the live check compares
 * getBoundingClientRect before and after.
 */
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const here = dirname(fileURLToPath(import.meta.url));
const strip = readFileSync(join(here, "PaneTabStrip.scss"), "utf8").replace(/\r\n/g, "\n");

/** The text of the first rule whose selector line contains `selector`. */
function rule(css: string, selector: string): string {
    const at = css.indexOf(selector);
    expect(at, `${selector} is in the stylesheet`).toBeGreaterThanOrEqual(0);
    const open = css.indexOf("{", at);
    let depth = 1;
    let i = open + 1;
    while (depth > 0 && i < css.length) {
        if (css[i] === "{") depth++;
        else if (css[i] === "}") depth--;
        i++;
    }
    return css.slice(open + 1, i - 1);
}

/** The geometry properties that would move a pill or its neighbours. */
const LAYOUT = /^\s*(border[a-z-]*|outline-width|padding[a-z-]*|margin[a-z-]*|width|min-width|height|gap|flex[a-z-]*|display):/m;

describe("the picked pill's stylesheet", () => {
    const selected = rule(strip, "&--selected {");
    const ring = rule(selected, "&::after {");
    const mark = rule(strip, ".pane-tab-selected-mark {");

    it("sets nothing on the pill itself that changes its box", () => {
        const own = selected.replace(ring, "");
        expect(own).not.toMatch(LAYOUT);
        expect(own).not.toMatch(/\boutline:/);
    });

    it("draws its ring inside, as an inset shadow on an overlay that takes no space", () => {
        expect(ring).toMatch(/position: absolute|abs-fill/);
        expect(ring).toMatch(/box-shadow: inset 0 0 0 2px var\(--accent-color\)/);
        expect(ring).toMatch(/pointer-events: none/);
    });

    it("keeps the check mark out of the flow, so it adds no width", () => {
        expect(mark).toMatch(/position: absolute/);
        expect(mark).not.toMatch(/\bmargin/);
    });

    it("uses theme tokens, no colour literals", () => {
        expect(selected + mark).not.toMatch(/#[0-9a-f]{3,8}\b|rgb\(|hsl\(/i);
    });
});
