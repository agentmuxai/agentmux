// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The New activity pill floats over the transcript, so its fill must hide the text under it at rest and on
 * hover. `--main-bg-color` follows the window's opacity and the kit's hover is a 12% tint over nothing, so
 * the pill takes the opaque `--elevated-bg-color` for both. The stylesheet is read as text, like
 * PaneTabStrip.progress-style.test.ts.
 */
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const here = dirname(fileURLToPath(import.meta.url));
const css = readFileSync(join(here, "../styles/_document.scss"), "utf8").replace(/\r\n/g, "\n");

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

describe("the New activity pill's fill", () => {
    const pill = rule(css, ".agent-new-activity-pill");

    it("is the opaque elevated surface at rest", () => {
        expect(pill).toMatch(/^\s*background: var\(--elevated-bg-color\);/m);
    });

    it("keeps the kit's hover tint over that surface, not over nothing", () => {
        expect(pill).toMatch(/--ui-bg-hover: color-mix\(in srgb, var\(--ui-tone\) var\(--ui-tint-hover\), var\(--elevated-bg-color\)\);/);
    });

    it("never takes the window-opacity surface or a transparent mix", () => {
        expect(pill).not.toMatch(/--main-bg-color/);
        expect(pill).not.toMatch(/transparent/);
    });
});
