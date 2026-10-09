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
    // The cap, the scroll and `contain` come from one mixin every capped
    // message box includes (REPORT_JEKT_COLLAPSE_AND_PREVIEW_SKID_2026_10_07.md §2.3).
    const PREVIEW_BOX = /^\s*@include transcript-preview-box;/m;

    it("the preview-box mixin caps like a tool preview, scrolls and contains", () => {
        const mixin = ruleBody(scss, /^@mixin transcript-preview-box\s*\{/m);
        expect(mixin).toMatch(/^\s*max-height:\s*\$transcript-preview-max-height;/m);
        expect(mixin).toMatch(/^\s*overflow-y:\s*auto;/m);
        expect(mixin).toMatch(/^\s*overscroll-behavior:\s*contain;/m);
    });

    it("the expanded jekt body takes the same cap as tool previews and scrolls", () => {
        expect(ruleBody(scss, /^ {12}\.agent-jekt-body\s*\{/m)).toMatch(PREVIEW_BOX);
    });

    it("the raw payload block takes the same cap", () => {
        const fromRaw = scss.slice(scss.indexOf(".agent-jekt-raw {"));
        expect(ruleBody(fromRaw, /^ {16}\.agent-jekt-raw-body\s*\{/m)).toMatch(PREVIEW_BOX);
    });

    // Reversed from #3861's native chaining: native chaining latches a wheel
    // gesture to the inner box, which is what
    // SPEC_TOOL_PREVIEW_SCROLL_CHAINING_2026_07_03 replaced with a JS
    // hand-off. Both jekt boxes now contain and hand off like a tool preview
    // (SPEC_AGENT_PANE_PREVIEW_CLEANUPS_2026_09_26.md §2; scroll-handoff.ts).
    it("the jekt body and raw payload contain their scroll (the JS hand-off moves the pane)", () => {
        expect(ruleBody(scss, /^ {12}\.agent-jekt-body\s*\{/m)).toMatch(PREVIEW_BOX);
        const fromRaw = scss.slice(scss.indexOf(".agent-jekt-raw {"));
        expect(ruleBody(fromRaw, /^ {16}\.agent-jekt-raw-body\s*\{/m)).toMatch(PREVIEW_BOX);
    });
});
