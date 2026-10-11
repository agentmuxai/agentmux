// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The working row's ASCII spinner (AsciiSpinner.tsx) and the compaction
 * progress bar are the status text's color: the pane's identity color,
 * falling back to the theme accent. jsdom has no cascade over
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

describe("working row indicator color", () => {
    const loading = block(scss.indexOf("&--loading {"));

    it("the status text uses the identity color", () => {
        expect(loading).toContain(`color: ${IDENTITY};`);
    });

    it("the spinner inside the loading row uses the same color, in one fixed cell", () => {
        const at = loading.indexOf(".agent-spinner-ascii {");
        expect(at).toBeGreaterThan(-1);
        const spinner = block(at + scss.indexOf("&--loading {"));
        expect(spinner).toContain(`color: ${IDENTITY};`);
        expect(spinner).toContain("width: 1ch;");
    });

    it("the compaction progress bar uses the same color, its track a tint of it", () => {
        const base = scss.indexOf("&--loading {");
        const track = block(base + loading.indexOf(".agent-working-row-progress {"));
        const fill = block(base + loading.indexOf(".agent-working-row-progress-fill {"));
        expect(track).toContain(`background: color-mix(in srgb, ${IDENTITY} 15%, transparent);`);
        expect(fill).toContain(`background: ${IDENTITY};`);
    });

    it("the loading row no longer styles the pulsing dot", () => {
        expect(loading).not.toContain(".agent-spinner-dot");
    });

    it("the shared dot keeps the accent, for the pending-messages header", () => {
        const shared = block(scss.indexOf("\n    .agent-spinner-dot {"));
        expect(shared).toContain("background: var(--accent-color);");
    });
});
