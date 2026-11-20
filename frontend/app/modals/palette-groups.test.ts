// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import type { CommandEntry } from "@/app/store/command-registry";
import { paletteRows, sortCommands } from "./palette-groups";

const cmd = (id: string, label: string, category: string) => ({ id, label, category, execute: () => {} }) as CommandEntry;

describe("sortCommands", () => {
    it("lists the known categories in their order, then by name", () => {
        const sorted = sortCommands([cmd("a", "Zed", "Tab"), cmd("b", "Alpha", "Tab"), cmd("c", "Mid", "Open")]);
        expect(sorted.map((c) => c.label)).toEqual(["Mid", "Alpha", "Zed"]);
    });

    it("keeps the rows of a category outside the order together, after the known ones, by category name", () => {
        const sorted = sortCommands([
            cmd("1", "B one", "View"),
            cmd("2", "A one", "Help"),
            cmd("3", "A two", "View"),
            cmd("4", "Z", "Open"),
            cmd("5", "A three", "Help"),
        ]);
        expect(sorted.map((c) => c.label)).toEqual(["Z", "A one", "A three", "A two", "B one"]);
    });
});

describe("paletteRows", () => {
    const cmds = sortCommands([cmd("1", "A", "Open"), cmd("2", "B", "Open"), cmd("3", "C", "Tab"), cmd("4", "D", "Dev")]);

    it("browsing, names each group once, on its first row, in lower case", () => {
        expect(paletteRows(cmds, false).map((r) => r.group)).toEqual(["open", undefined, "tab", "dev"]);
    });

    it("browsing, puts a hairline above every group but the first", () => {
        expect(paletteRows(cmds, false).map((r) => r.divider)).toEqual([false, false, true, true]);
    });

    it("typing, names no group and draws no hairline", () => {
        const rows = paletteRows(cmds, true);
        expect(rows.map((r) => r.group)).toEqual([undefined, undefined, undefined, undefined]);
        expect(rows.some((r) => r.divider)).toBe(false);
    });

    it("keeps the order it is given", () => {
        expect(paletteRows(cmds, false).map((r) => r.cmd.id)).toEqual(["1", "2", "3", "4"]);
    });

    it("gives nothing for no commands", () => {
        expect(paletteRows([], false)).toEqual([]);
    });
});
