// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The model panel marks the row the keys are on with the class `active` (the dropup's tests cover the
 * class following the keys). The shared `.menu-item` styles only :hover, so the panel has to style
 * `active` itself, or a person using only the keyboard cannot see where they are.
 */

import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

const scss = readFileSync(join(__dirname, "../styles/_composer-strip.scss"), "utf8");

/** The text of the first block that opens with `selector {` (nested braces counted). */
function block(source: string, selector: string): string {
    const start = source.indexOf(`${selector} {`);
    expect(start, `no ${selector} block`).toBeGreaterThanOrEqual(0);
    let depth = 0;
    for (let i = source.indexOf("{", start); i < source.length; i++) {
        if (source[i] === "{") depth++;
        if (source[i] === "}" && --depth === 0) return source.slice(start, i + 1);
    }
    throw new Error(`${selector} block is not closed`);
}

describe("the model panel's row the keys are on", () => {
    it("has a visible background, the same fill a hovered row gets", () => {
        const row = block(scss, ".agent-runtime-dropup-row");
        const active = block(row, "&.active");
        expect(active).toMatch(/background-color:\s*var\(--hover-bg-color\)/);
    });

    it("is not given a fill on a row that cannot be chosen", () => {
        const row = block(scss, ".agent-runtime-dropup-row");
        expect(block(row, "&.is-disabled")).toMatch(/&:hover\s*{\s*background:\s*transparent/);
    });
});
