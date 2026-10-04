// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Every shortcut the UI shows comes from the shortcut table (`shortcutFor`)
// or the key formatter (`keyLabel`), so a hint can't disagree with the key or
// show another platform's modifiers
// (docs/reports/REPORT_KEYBINDINGS_AUDIT_AND_CONSOLIDATION_2026_10_04.md §7.1).
// This scans the frontend for shortcut text written by hand outside comments.
// A line that really needs a literal glyph says so with `keyhint-ok`.

import { readdirSync, readFileSync } from "node:fs";
import { join, relative } from "node:path";
import { describe, expect, it } from "vitest";

const FRONTEND = join(__dirname, "..", "..");
const HINT = /(Ctrl|Cmd|Alt|Mod|Meta|Option)\+[A-Za-z0-9←→↑↓,./=-]|[⌘⌥⌃]/;
const SKIP = /(\.test\.tsx?$|[\\/]keybindings[\\/]|[\\/]util[\\/]keyutil\.ts$)/;

function sources(dir: string, out: string[] = []): string[] {
    for (const entry of readdirSync(dir, { withFileTypes: true })) {
        if (entry.name === "node_modules" || entry.name.startsWith(".")) continue;
        const p = join(dir, entry.name);
        if (entry.isDirectory()) sources(p, out);
        else if (/\.tsx?$/.test(entry.name) && !SKIP.test(p)) out.push(p);
    }
    return out;
}

describe("shortcut hints", () => {
    it("are generated, not written by hand", () => {
        const offenders: string[] = [];
        for (const file of sources(FRONTEND)) {
            let inBlockComment = false;
            readFileSync(file, "utf8")
                .split("\n")
                .forEach((line, i) => {
                    const t = line.trim();
                    if (inBlockComment) {
                        if (t.includes("*/")) inBlockComment = false;
                        return;
                    }
                    if (t.startsWith("/*") || t.startsWith("{/*")) {
                        if (!t.includes("*/")) inBlockComment = true;
                        return;
                    }
                    if (t.startsWith("//") || t.startsWith("*") || line.includes("keyhint-ok")) return;
                    const code = line.replace(/\s\/\/.*$/, "");
                    if (HINT.test(code)) offenders.push(`${relative(FRONTEND, file)}:${i + 1}: ${t}`);
                });
        }
        expect(offenders).toEqual([]);
    });
});
