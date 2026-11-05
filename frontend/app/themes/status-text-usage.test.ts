// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A stylesheet colours text with the status text tokens, never with the status colours themselves. The colours
 * (`--error-color`, `--warning-color`, `--success-color`) are for washes, lines and icons: on several themes
 * they are 2.3 to 4.3:1 as text. `--error-text-color` and its siblings are the same colour moved until it
 * passes AA on every theme (status-text.test.ts checks them), so a rule that reads text from the colour
 * itself is a contrast bug that no theme test would see.
 */

import { readdirSync, readFileSync } from "node:fs";
import { join, relative } from "node:path";
import { describe, expect, it } from "vitest";

const root = new URL("../../", import.meta.url).pathname.replace(/^\/([A-Za-z]:)/, "$1");

function stylesheets(dir: string, out: string[] = []): string[] {
    for (const e of readdirSync(dir, { withFileTypes: true })) {
        if (e.name === "node_modules") continue;
        const p = join(dir, e.name);
        if (e.isDirectory()) stylesheets(p, out);
        else if (/\.s?css$/.test(e.name)) out.push(p);
    }
    return out;
}

/** `color: var(--error-color)` as the property `color` (not `border-color`, `background-color`...). */
export const STATUS_COLOUR_AS_TEXT = /(^|[^-\w])color:\s*var\(--(error|warning|success)-color\)/;

describe("status colours as text", () => {
    it("recognises the rule it forbids", () => {
        expect(STATUS_COLOUR_AS_TEXT.test("    color: var(--error-color);")).toBe(true);
        expect(STATUS_COLOUR_AS_TEXT.test("    color: var(--warning-color) !important;")).toBe(true);
        expect(STATUS_COLOUR_AS_TEXT.test("    color: var(--error-text-color);")).toBe(false);
        expect(STATUS_COLOUR_AS_TEXT.test("    border-color: var(--error-color);")).toBe(false);
        expect(STATUS_COLOUR_AS_TEXT.test("    background-color: var(--success-color);")).toBe(false);
    });

    it("no stylesheet sets `color` from a status colour", () => {
        const found: string[] = [];
        for (const file of stylesheets(join(root, "app")).concat(stylesheets(join(root, "layout")))) {
            readFileSync(file, "utf8")
                .split(/\r?\n/)
                .forEach((line, i) => {
                    if (STATUS_COLOUR_AS_TEXT.test(line)) found.push(`${relative(root, file)}:${i + 1}: ${line.trim()}`);
                });
        }
        expect(found, "use --error-text-color, --warning-text-color or --success-text-color for text").toEqual([]);
    });
});
