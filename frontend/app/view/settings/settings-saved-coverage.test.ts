// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Every Settings row that writes a setting names it (`SettingRow`'s `settingKey`), so the "Saved" note shows on every
 * section, not only on some. A row whose control writes with `set("…")` or `trackSave(` and has no `settingKey` is a
 * row someone added without it. A row that is a button (a test notification, a reset) writes no setting and is left
 * alone. The scan reads the section sources, as the repo's other coverage tests do.
 */

import fs from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";

const SECTIONS = path.join(__dirname, "sections");

/** The end of the `<SettingRow … />` that starts at `start`: braces and strings tracked, JSX text apostrophes not. */
function blockEnd(src: string, start: number): number {
    let depth = 0;
    let quote: string | null = null;
    for (let i = start + "<SettingRow".length; i < src.length; i++) {
        const c = src[i];
        if (quote) {
            if (c === "\\") i++;
            else if (c === quote) quote = null;
            continue;
        }
        if (c === '"' || c === "'" || c === "`") {
            let j = i - 1;
            while (j > start && /\s/.test(src[j])) j--;
            if (/[=(,:?[{&|!+>]/.test(src[j])) quote = c;
            continue;
        }
        if (c === "{") depth++;
        else if (c === "}") depth--;
        else if (depth === 0 && c === "/" && src[i + 1] === ">") return i + 2;
    }
    return -1;
}

/** Each unkeyed row of a source that writes a setting: `file:line`. */
function unkeyedWriters(file: string, src: string): string[] {
    const found: string[] = [];
    for (let from = 0; ; ) {
        const start = src.indexOf("<SettingRow", from);
        if (start < 0) break;
        const end = blockEnd(src, start);
        if (end < 0) {
            found.push(`${file}: a <SettingRow> this scan cannot read to its end`);
            break;
        }
        from = end;
        const block = src.slice(start, end);
        if (/\bsettingKey=/.test(block)) continue;
        if (/\bset\(|\btrackSave\(/.test(block)) found.push(`${file}:${src.slice(0, start).split("\n").length}`);
    }
    return found;
}

describe("every Settings row that writes a setting names it", () => {
    it("has no <SettingRow> that writes without a settingKey", () => {
        const files = fs.readdirSync(SECTIONS).filter((n) => n.endsWith(".tsx") && !/\.(test|behavior)\./.test(n));
        const missing = files.flatMap((n) => unkeyedWriters(n, fs.readFileSync(path.join(SECTIONS, n), "utf8")));
        expect(missing, "add settingKey (the key the control writes) to these rows").toEqual([]);
    });

    it("sees an unkeyed row (the scan itself works)", () => {
        const row = `<SettingRow id="a" label="L" control={<Toggle onChange={(v) => set("a:b", v)} />} />`;
        expect(unkeyedWriters("x.tsx", row)).toEqual(["x.tsx:1"]);
        expect(unkeyedWriters("x.tsx", row.replace("<SettingRow", '<SettingRow settingKey="a:b"'))).toEqual([]);
        expect(unkeyedWriters("x.tsx", `<SettingRow label="Don't" control={<Button onClick={test}>Can't</Button>} />`)).toEqual([]);
    });
});
