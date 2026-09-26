// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Swarm agent-card hover — stylesheet contract.
 * SPEC_COMPOSER_ACCOUNT_SWITCH_AND_JEKT_HEIGHT_CAP_2026_09_26.md Part C.
 *
 * jsdom has no :hover cascade or specificity, so this reads the SCSS source, the
 * same way styles/tool-panel-height.test.ts does.
 */

import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

const scss = readFileSync(join(__dirname, "swarm-view.scss"), "utf8");

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

describe("Swarm agent card hover", () => {
    const hover = ruleBody(scss, /^ {4}&:hover:not\(&--active\)\s*\{/m);

    it("draws the selection outline (1px inset box-shadow) in the hover colour", () => {
        expect(hover).toMatch(
            /^\s*box-shadow:\s*inset 0 0 0 1px var\(--swarm-agent-hover-border, var\(--border-color\)\);/m
        );
    });

    it("does not fill the card", () => {
        expect(hover).not.toMatch(/background/);
    });

    it("has the same shape as the selected card, so selecting does not change the outline's corners", () => {
        const active = ruleBody(scss, /^ {4}&--active\s*\{/m);
        expect(hover).toMatch(/border-radius:\s*0;/);
        expect(active).toMatch(/border-radius:\s*0;/);
    });

    it("never applies to the selected card (its border must stay full strength when hovered)", () => {
        expect(scss).toMatch(/&:hover:not\(&--active\)/);
        const card = scss.slice(scss.indexOf(".swarm-agent-card {"), scss.indexOf("// ── Agent root row"));
        expect(card).not.toMatch(/^ {4}&:hover\s*\{/m);
    });

    it("the old hover fill variable is gone", () => {
        expect(scss).not.toContain("--swarm-agent-hover-bg");
    });
});
