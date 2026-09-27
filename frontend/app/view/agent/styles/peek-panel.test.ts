// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Hover peek panel — stylesheet contract.
 * SPEC_PEEK_PANEL_META_ROW_AND_MONO_COMMAND_2026_09_27.md §4.
 *
 * jsdom has no layout or computed fonts, so this reads the SCSS source, the same
 * way tool-panel-height.test.ts does.
 */

import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

const scss = readFileSync(join(__dirname, "_document-nodes.scss"), "utf8");

/** The declarations directly inside the first rule block whose selector matches `selector`. */
function ruleBody(src: string, selector: RegExp): string {
    const m = selector.exec(src);
    if (!m) throw new Error(`selector not found: ${selector}`);
    let depth = 0;
    let out = "";
    for (let i = src.indexOf("{", m.index); i < src.length; i++) {
        const c = src[i];
        if (c === "{") depth++;
        if (c === "}") depth--;
        if (depth === 0) break;
        if (depth === 1 && c !== "{") out += c;
    }
    return out;
}

describe("peek panel: time + tokens line", () => {
    const meta = ruleBody(scss, /^\.agent-node-peek-tooltip-meta\s*\{/m);

    it("is one flex row pinned to the right", () => {
        expect(meta).toMatch(/^\s*display:\s*flex;/m);
        expect(meta).toMatch(/^\s*justify-content:\s*flex-end;/m);
        expect(meta).toMatch(/^\s*white-space:\s*nowrap;/m);
    });
});

describe("peek panel: command", () => {
    const body = ruleBody(scss, /^\.agent-node-peek-tooltip-body\s*\{/m);

    it("uses the monospace family token, not the --fixed-font shorthand (invalid in font-family)", () => {
        expect(body).toMatch(/^\s*font-family:\s*var\(--font-mono\);/m);
        expect(body).not.toContain("--fixed-font");
    });

    it("breaks a long path at any character", () => {
        expect(body).toMatch(/^\s*word-break:\s*break-all;/m);
        expect(body).not.toMatch(/overflow-wrap:\s*break-word/);
    });

    it("takes the per-tool colour from the same table as the tool row", () => {
        // Both consumers are generated from $tool-name-colors, so they can't drift.
        expect(scss).toMatch(/^\$tool-name-colors:\s*\(/m);
        expect(scss).toMatch(
            /@each \$tool, \$color in \$tool-name-colors \{\s*\.agent-tool-block\[data-tool="#\{\$tool\}"\] \.agent-tool-name \{ color: \$color; \}/
        );
        expect(scss).toMatch(
            /@each \$tool, \$color in \$tool-name-colors \{\s*&\[data-tool="#\{\$tool\}"\] \{ color: \$color; \}/
        );
        // ...and no hand-written per-tool row colour rules remain beside the table.
        expect(scss).not.toMatch(/\.agent-tool-block\[data-tool="bash"\]\s+\.agent-tool-name\s*\{/);
    });

    it("the table keeps the row's colours: Bash green, Write/Edit yellow", () => {
        const table = scss.slice(scss.indexOf("$tool-name-colors: ("), scss.indexOf(");", scss.indexOf("$tool-name-colors: (")));
        expect(table).toMatch(/"bash":\s*var\(--term-bright-green\)/);
        expect(table).toMatch(/"write":\s*var\(--warning-color\)/);
        expect(table).toMatch(/"edit":\s*var\(--warning-color\)/);
        expect(table).toMatch(/"read":\s*var\(--accent-color\)/);
    });
});
