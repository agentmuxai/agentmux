// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The task drawer paints over the board. In the narrow list each section's head is sticky at
 * `--z-tab-name`; a drawer with no z-index of its own was cut across by those heads. jsdom does not
 * lay out SCSS, so this reads the two rules and the token values they name.
 */
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

const appDir = join(__dirname, "..", "..");
const kanban = readFileSync(join(__dirname, "kanban.scss"), "utf8");
const theme = readFileSync(join(appDir, "theme.scss"), "utf8");

/** The `z-index` declared in the rule that starts with `selector` (first occurrence). */
function zIndexOf(css: string, selector: string): string | null {
    const start = css.indexOf(selector);
    if (start < 0) return null;
    const body = css.slice(css.indexOf("{", start) + 1, css.indexOf("}", start));
    return /z-index:\s*([^;]+);/.exec(body)?.[1].trim() ?? null;
}

function tokenValue(value: string | null): number | null {
    const name = /^var\((--z-[a-z-]+)\)$/.exec(value ?? "")?.[1];
    if (!name) return null;
    const declared = new RegExp(`${name}:\\s*(-?\\d+)`).exec(theme)?.[1];
    return declared === undefined ? null : Number(declared);
}

describe("the Kanban's layers", () => {
    it("paints the drawer over the sticky section heads, both from tokens", () => {
        const drawer = tokenValue(zIndexOf(kanban, ".kanban-drawer {"));
        const head = tokenValue(zIndexOf(kanban, ".kanban-column__head {\n        position: sticky"));
        expect(head).not.toBeNull();
        expect(drawer).not.toBeNull();
        expect(drawer!).toBeGreaterThan(head!);
    });

    it("keeps the board's layers inside the Kanban, so nothing in it is ordered against the pane", () => {
        expect(/\.kanban \{[^}]*isolation:\s*isolate/.test(kanban)).toBe(true);
    });

    it("declares no raw z-index", () => {
        expect(/z-index:\s*-?\d/.test(kanban)).toBe(false);
    });
});
