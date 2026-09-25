// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Tool-preview height cap — stylesheet contract.
 * SPEC_TOOL_PREVIEW_HEIGHT_THIRD_AND_FOLLOW_LATEST_2026_09_25.md §2.
 *
 * jsdom has no layout (no vh, no container queries), so this reads the SCSS
 * source, the same way tileLayoutFocusScroll.test.ts does.
 */

import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

const read = (name: string) => readFileSync(join(__dirname, name), "utf8");

/** The declarations directly inside the first rule block whose selector matches `selector`. */
function ruleBody(scss: string, selector: RegExp): string {
    const m = selector.exec(scss);
    if (!m) throw new Error(`selector not found: ${selector}`);
    let depth = 0;
    let out = "";
    for (let i = scss.indexOf("{", m.index); i < scss.length; i++) {
        const c = scss[i];
        if (c === "{") depth++;
        if (c === "}") depth--;
        if (depth === 0) break;
        if (depth === 1 && c !== "{") out += c;
    }
    return out;
}

describe("tool preview panel height", () => {
    it("tool previews are capped at one third of the old 50vh", () => {
        const body = ruleBody(read("_document-nodes.scss"), /^ {8}\.agent-tool-panel\s*\{/m);
        expect(body).toMatch(/^\s*max-height:\s*calc\(50vh \/ 3\);/m);
        expect(body).not.toMatch(/^\s*max-height:\s*50vh;/m);
    });

    it("no container-query rule re-raises the tool panel cap", () => {
        // The old Tier 4 `.agent-tool-panel { max-height: 60vh }` never applied
        // (lower specificity) and was removed; nothing should bring it back.
        const code = read("_responsive.scss").replace(/\/\/.*$/gm, "");
        expect(code).not.toMatch(/\.agent-tool-panel\s*\{/);
    });

    it("the persistent-shell log keeps its own 50vh cap", () => {
        const body = ruleBody(read("_shell-node.scss"), /^\.agent-shell-block \.agent-tool-panel\s*\{/m);
        expect(body).toMatch(/^\s*max-height:\s*50vh;/m);
    });
});
