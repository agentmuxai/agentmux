// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * No `font-family:` may use the `--fixed-font` / `--base-font` tokens.
 *
 * Those are `font` SHORTHAND values (`normal <size> / normal <family>`,
 * theme.scss). In `font-family:` the declaration is invalid at computed-value
 * time, so the browser drops it — the `var()` fallback does not apply, because
 * the variable IS defined — and the element silently inherits whatever font its
 * ancestor has. Inside `.agent-view` that happened to be mono, which hid the bug;
 * outside it (status bar, modals, memory editor, Portal-rendered popovers) the
 * text came out in the app's sans font. Use `var(--font-mono)` /
 * `var(--font-sans)`.
 *
 * The stylelint rule in .stylelintrc.json says the same, but stylelint isn't
 * run in CI and skips .stylelintignore'd files; this test runs in CI and scans
 * every stylesheet.
 * SPEC_PEEK_PANEL_META_ROW_AND_MONO_COMMAND_2026_09_27.md §7.
 */

import { readdirSync, readFileSync } from "node:fs";
import { join, relative } from "node:path";
import { describe, expect, it } from "vitest";

const FRONTEND = join(__dirname, "..");

function stylesheets(dir: string, out: string[] = []): string[] {
    for (const entry of readdirSync(dir, { withFileTypes: true })) {
        if (entry.name === "node_modules" || entry.name.startsWith(".")) continue;
        const p = join(dir, entry.name);
        if (entry.isDirectory()) stylesheets(p, out);
        else if (/\.(s?css)$/.test(entry.name)) out.push(p);
    }
    return out;
}

describe("font-family tokens", () => {
    it("no stylesheet passes a font shorthand token to font-family", () => {
        const offenders: string[] = [];
        for (const file of stylesheets(FRONTEND)) {
            const lines = readFileSync(file, "utf8").split("\n");
            lines.forEach((line, i) => {
                const code = line.replace(/\/\/.*$/, "");
                if (/font-family\s*:[^;]*var\(\s*--(fixed|base)-font\b/.test(code)) {
                    offenders.push(`${relative(FRONTEND, file)}:${i + 1}: ${line.trim()}`);
                }
            });
        }
        expect(offenders).toEqual([]);
    });
});
