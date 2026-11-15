// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A pending attachment is cancelled from the round x on its tile (#47669), so the x is always there: not hidden until
 * the mouse is over the tile. The stylesheet is read as text, like PaneTabStrip.progress-style.test.ts.
 */
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const here = dirname(fileURLToPath(import.meta.url));
const css = readFileSync(join(here, "../styles/_attachments.scss"), "utf8").replace(/\r\n/g, "\n");

/** The text of the first rule whose selector is `selector`. */
function rule(source: string, selector: string): string {
    const at = source.indexOf(`${selector} {`);
    expect(at, `${selector} is in the stylesheet`).toBeGreaterThanOrEqual(0);
    const open = source.indexOf("{", at);
    let depth = 1;
    let i = open + 1;
    while (depth > 0 && i < source.length) {
        if (source[i] === "{") depth++;
        else if (source[i] === "}") depth--;
        i++;
    }
    return source.slice(open + 1, i - 1);
}

describe("a tile's remove control", () => {
    const remove = rule(css, "&__remove.ui-close");

    it("shows at rest: it is not made transparent or shown only on hover or focus", () => {
        expect(remove).not.toMatch(/opacity:\s*0\s*;/);
        expect(remove).not.toMatch(/visibility:\s*hidden/);
        expect(remove).not.toMatch(/display:\s*none/);
        expect(css).not.toMatch(/agent-attachment-tile__remove\s*\{\s*opacity:\s*1/);
    });

    it("sits inside the tile's box, on its top-right corner", () => {
        expect(remove).toMatch(/position:\s*absolute;/);
        expect(remove).toMatch(/top:\s*3px;/);
        expect(remove).toMatch(/right:\s*3px;/);
    });
});
