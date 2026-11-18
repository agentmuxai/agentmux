// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
// @vitest-environment node

/**
 * The Tailwind colour tokens inside a pane's scope. Every theme file ends with a "Tailwind @theme token sync" block
 * that points the `--color-*` tokens at the app's own, on `:root`. A pane with a backdrop has to say the same again
 * on its own element (backdrop.scss), or the utility classes keep the window's colours there. This keeps the pane's
 * list the same as the themes', so a token synced in the themes later is synced in a pane too.
 */

import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

const APP = join(__dirname, "..");

/** The `--color-*: var(--…)` declarations of a stylesheet, in order of name. */
function sync(path: string): string[] {
    const text = readFileSync(path, "utf8");
    return [...text.matchAll(/^\s*(--color-[\w-]+):\s*(var\(--[\w-]+\));/gm)].map((m) => `${m[1]}: ${m[2]}`).sort();
}

describe("the Tailwind token sync in a pane's scope", () => {
    const pane = sync(join(APP, "backdrop/backdrop.scss"));
    const themes = readdirSync(join(APP, "themes")).filter((f) => f.endsWith(".scss") && f !== "index.scss");

    it("is not empty, and there are themes to compare it with", () => {
        expect(pane.length).toBeGreaterThan(10);
        expect(themes.length).toBeGreaterThan(5);
    });

    it.each(themes)("says what %s says", (file) => {
        expect(pane).toEqual(sync(join(APP, "themes", file)));
    });
});
