// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The working row's pulsing dot is the status text's color — the pane's
 * identity color, falling back to the theme accent. jsdom has no cascade over
 * SCSS, so this reads the source, as tool-panel-height.test.ts does.
 */
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

const scss = readFileSync(join(__dirname, "_control-bar.scss"), "utf8");
const IDENTITY = "var(--block-identity-color, var(--accent-color))";

/** The text between the `{` that follows `start` and its matching `}`. */
function block(start: number): string {
    let depth = 0;
    let from = -1;
    for (let i = scss.indexOf("{", start); i < scss.length; i++) {
        if (scss[i] === "{") {
            if (depth++ === 0) from = i + 1;
        } else if (scss[i] === "}" && --depth === 0) return scss.slice(from, i);
    }
    throw new Error("unbalanced braces");
}

describe("working row dot color", () => {
    const loading = block(scss.indexOf("&--loading {"));

    it("the status text uses the identity color", () => {
        expect(loading).toContain(`color: ${IDENTITY};`);
    });

    it("the dot inside the loading row uses the same color, fill and halo", () => {
        const dot = block(loading.indexOf(".agent-spinner-dot {") + scss.indexOf("&--loading {"));
        expect(dot).toContain(`background: ${IDENTITY};`);
        expect(dot).toContain(`color-mix(in srgb, ${IDENTITY} 60%, transparent)`);
    });

    it("the shared dot keeps the accent, for the pending-messages header", () => {
        const shared = block(scss.indexOf("\n    .agent-spinner-dot {"));
        expect(shared).toContain("background: var(--accent-color);");
    });
});
