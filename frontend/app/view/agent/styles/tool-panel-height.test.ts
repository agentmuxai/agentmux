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
    it("the shared cap is one third of the old 50vh", () => {
        expect(read("_document-nodes.scss")).toMatch(
            /^\$transcript-preview-max-height:\s*calc\(50vh \/ 3\);/m,
        );
    });

    it("tool previews use the shared cap, not the old 50vh", () => {
        const body = ruleBody(read("_document-nodes.scss"), /^ {8}\.agent-tool-panel\s*\{/m);
        expect(body).toMatch(/^\s*max-height:\s*\$transcript-preview-max-height;/m);
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

// SPEC_COMPOSER_ACCOUNT_SWITCH_AND_JEKT_HEIGHT_CAP_2026_09_26.md §3.
describe("jekt message height", () => {
    const scss = read("_document-nodes.scss");

    it("the expanded jekt body takes the same cap as tool previews and scrolls", () => {
        const body = ruleBody(scss, /^ {12}\.agent-jekt-body\s*\{/m);
        expect(body).toMatch(/^\s*max-height:\s*\$transcript-preview-max-height;/m);
        expect(body).toMatch(/^\s*overflow-y:\s*auto;/m);
    });

    it("the raw payload block takes the same cap", () => {
        const fromRaw = scss.slice(scss.indexOf(".agent-jekt-raw {"));
        const body = ruleBody(fromRaw, /^ {16}pre\s*\{/m);
        expect(body).toMatch(/^\s*max-height:\s*\$transcript-preview-max-height;/m);
        expect(body).toMatch(/^\s*overflow-y:\s*auto;/m);
    });

    it("the jekt body never sets overscroll-behavior (native wheel chaining to the pane)", () => {
        const body = ruleBody(scss, /^ {12}\.agent-jekt-body\s*\{/m);
        expect(body).not.toMatch(/overscroll-behavior\s*:/);
    });
});
