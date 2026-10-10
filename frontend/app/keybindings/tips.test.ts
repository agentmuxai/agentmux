// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Every Help tip anchors to the code it describes
// (docs/specs/SPEC_HELP_HIDDEN_TIPS_2026_10_10.md §4): when that code
// changes, this fails, and whoever changed the gesture updates the tip too.

import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { gestureLabels, TIPS, tipsFor } from "./tips";

const ROOT = join(__dirname, "..", "..", "..");

describe("Help tips", () => {
    it("have unique ids", () => {
        const ids = TIPS.map((t) => t.id);
        expect(new Set(ids).size).toBe(ids.length);
    });

    it.each(TIPS.map((t) => [t.id, t] as const))("%s still matches its code", (_id, tip) => {
        const text = readFileSync(join(ROOT, tip.source.file), "utf8");
        expect(text.includes(tip.source.anchor), `${tip.source.file} no longer contains ${JSON.stringify(tip.source.anchor)}: update or remove the tip`).toBe(true);
    });

    it("name modifiers per platform", () => {
        const select = TIPS.find((t) => t.id === "files:select")!;
        expect(gestureLabels(select, "mac")).toEqual([["⌘", "click"], ["⇧", "click"]]);
        expect(gestureLabels(select, "other")).toEqual([["Ctrl", "click"], ["Shift", "click"]]);
        const numbers = TIPS.find((t) => t.id === "pane:numbers")!;
        expect(gestureLabels(numbers, "other")).toEqual([["hold", "Ctrl", "Shift"]]);
    });

    it("leave out tips for another operating system", () => {
        expect(tipsFor("win32").some((t) => t.id === "window:edge:shiftDrag")).toBe(true);
        expect(tipsFor("darwin").some((t) => t.id === "window:edge:shiftDrag")).toBe(false);
        expect(tipsFor("darwin").some((t) => t.id === "term:ctrlF")).toBe(false);
    });
});
