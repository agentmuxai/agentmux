// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// A pane opened from a click must appear in the window the click was in.
// A raw `PaneOpenCommand` / `AgentOpenCommand` with no `tab_id` falls back to the first workspace with an
// active tab, which can be another window's. srv derives the tab from `split_reference_block_id` or
// `stack_onto_block_id` (app_api/pane.rs `open_pane`), so a call anchored to a pane of this window is
// fine too. A singleton view goes through `openOrFocusPaneByView` instead.

import { readdirSync, readFileSync } from "node:fs";
import { join, relative, resolve, sep } from "node:path";
import { describe, expect, it } from "vitest";

const repoRoot = resolve(__dirname, "../../..");
const ANCHORS = ["tab_id", "split_reference_block_id", "stack_onto_block_id"];
/** Call sites that pass no anchor on purpose, as `file:N` (the Nth call in that file); the list only shrinks. */
const ALLOWED: string[] = [];

function walk(dir: string, out: string[] = []): string[] {
    for (const e of readdirSync(dir, { withFileTypes: true })) {
        const f = join(dir, e.name);
        if (e.isDirectory()) walk(f, out);
        else if (/\.tsx?$/.test(e.name) && !/\.test\.tsx?$/.test(e.name) && !/\.d\.ts$/.test(e.name)) out.push(f);
    }
    return out;
}

/** Whether the arguments name one of ANCHORS as a key (a whole word). */
function isAnchored(args: string): boolean {
    return ANCHORS.some((a) => new RegExp("(^|[^a-z_])" + a + "([^a-z_]|$)").test(args));
}

/** The text between the parentheses of the call whose `(` is at `open`. */
function callArgs(text: string, open: number): string {
    let depth = 0;
    for (let i = open; i < text.length; i++) {
        if (text[i] === "(") depth++;
        else if (text[i] === ")" && --depth === 0) return text.slice(open + 1, i);
    }
    return text.slice(open + 1);
}

describe("raw pane and agent opens", () => {
    it("each names the tab it opens in, or a pane of this window to anchor to", () => {
        const unanchored: string[] = [];
        let calls = 0;
        for (const file of walk(join(repoRoot, "frontend"))) {
            const path = relative(repoRoot, file).split(sep).join("/");
            if (path.endsWith("rpc-api/generated.ts")) continue;
            const text = readFileSync(file, "utf8");
            let n = 0;
            for (const m of text.matchAll(/\b(?:PaneOpenCommand|AgentOpenCommand)\(/g)) {
                n++;
                calls++;
                const args = callArgs(text, m.index! + m[0].length - 1);
                if (!isAnchored(args)) unanchored.push(`${path}:${n}`);
            }
        }
        expect(calls).toBeGreaterThan(10); // the scan still finds the calls it guards
        expect(unanchored.filter((u) => !ALLOWED.includes(u))).toEqual([]);
    });

    it("flags a call with no anchor", () => {
        const bad = "RpcApi.PaneOpenCommand(TabRpcClient, { view: \"settings\", meta }, {})";
        expect(isAnchored(callArgs(bad, bad.indexOf("(")))).toBe(false);
    });
});
