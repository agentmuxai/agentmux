// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The command palette's search row is balanced (#47671): the kit's magnifier cell is a square of (--ui-height - 2px),
 * so the row gives it a definite --ui-height and no size of its own, and ESC gets as much room on the right as the
 * cell leaves left of the glyph. Before, the row overrode the cell to `width: auto; aspect-ratio: 1`, which collapsed
 * it to the glyph's own width (10px) and left the magnifier flush against the left edge. The stylesheet is read as
 * text, like PaneTabStrip.progress-style.test.ts.
 */
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const here = dirname(fileURLToPath(import.meta.url));
const css = readFileSync(join(here, "command-palette.scss"), "utf8").replace(/\r\n/g, "\n");

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

describe("the command palette's search row", () => {
    const row = rule(css, ".command-palette-input-row");
    const cell = rule(row, ".ui-search-icon");

    it("gives the kit's magnifier cell a definite height to be square against", () => {
        expect(row).toMatch(/--ui-height:\s*42px;/);
        expect(row).toMatch(/min-height:\s*var\(--ui-height\);/);
        expect(row).toMatch(/box-sizing:\s*border-box;/);
    });

    it("leaves the cell's size to the kit: no width, height or aspect ratio of its own", () => {
        expect(cell).not.toMatch(/\bwidth\s*:/);
        expect(cell).not.toMatch(/height\s*:/);
        expect(cell).not.toMatch(/aspect-ratio/);
    });

    it("puts as much room right of ESC as the cell leaves left of the glyph, ((42 - 2) - 10) / 2 = 15px", () => {
        expect(row).toMatch(/padding:\s*0 15px 0 0;/);
    });
});
