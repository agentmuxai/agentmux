// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import type { CommandEntry } from "@/app/store/command-registry";
import { arrangeRecent, paletteRows, paletteSegments, RECENT_SHOWN, sortCommands } from "./palette-groups";

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

describe("paletteSegments", () => {
    const cmds = sortCommands([cmd("1", "A", "Open"), cmd("2", "B", "Open"), cmd("3", "C", "Tab"), cmd("4", "D", "Dev")]);

    it("cuts the list into its groups, each named", () => {
        const segments = paletteSegments(paletteRows(cmds, false));
        expect(segments.map((s) => [s.label, s.rows.map((r) => r.cmd.id)])).toEqual([
            ["open", ["1", "2"]],
            ["tab", ["3"]],
            ["dev", ["4"]],
        ]);
    });

    it("makes one unnamed segment of a typed list", () => {
        const segments = paletteSegments(paletteRows(cmds, true));
        expect(segments).toHaveLength(1);
        expect(segments[0].label).toBeUndefined();
        expect(segments[0].rows).toHaveLength(4);
    });

    it("gives each row its place in the whole list", () => {
        expect(paletteRows(cmds, false).map((r) => r.index)).toEqual([0, 1, 2, 3]);
    });

    it("gives nothing for no rows", () => {
        expect(paletteSegments([])).toEqual([]);
    });
});

describe("arrangeRecent", () => {
    const all = sortCommands([cmd("1", "A", "Open"), cmd("2", "B", "Open"), cmd("3", "C", "Tab"), cmd("4", "D", "Dev")]);

    it("puts the remembered commands first, newest first, and each once", () => {
        const out = arrangeRecent(all, ["3", "1"], false);
        expect(out.cmds.map((c) => c.id)).toEqual(["3", "1", "2", "4"]);
        expect(out.recent).toBe(2);
    });

    it("skips a remembered id that is not listed now, without a gap", () => {
        const out = arrangeRecent(all, ["gone", "4"], false);
        expect(out.cmds.map((c) => c.id)).toEqual(["4", "1", "2", "3"]);
        expect(out.recent).toBe(1);
    });

    it("browsing, shows the newest eight only; typing, every remembered one that matches", () => {
        const many = sortCommands(Array.from({ length: 12 }, (_, i) => cmd(`c${i}`, `Cmd ${String(i).padStart(2, "0")}`, "Open")));
        const ids = many.map((c) => c.id);
        expect(arrangeRecent(many, ids, false).recent).toBe(RECENT_SHOWN);
        expect(arrangeRecent(many, ids, true).recent).toBe(12);
    });

    it("changes nothing when nothing is remembered", () => {
        const out = arrangeRecent(all, [], false);
        expect(out.cmds).toEqual(all);
        expect(out.recent).toBe(0);
    });
});

describe("paletteRows with recently used", () => {
    const all = sortCommands([cmd("1", "A", "Open"), cmd("2", "B", "Open"), cmd("3", "C", "Tab"), cmd("4", "D", "Dev")]);

    it("browsing, makes the recent rows a group of their own, then one group per category", () => {
        const { cmds, recent } = arrangeRecent(all, ["3", "1"], false);
        const rows = paletteRows(cmds, false, recent);
        expect(rows.map((r) => r.group)).toEqual(["recently used", undefined, "open", "dev"]);
        expect(rows.map((r) => r.divider)).toEqual([false, false, true, true]);
    });

    it("typing, makes two groups: recently used, then other commands", () => {
        const { cmds, recent } = arrangeRecent(all, ["2"], true);
        expect(paletteRows(cmds, true, recent).map((r) => r.group)).toEqual(["recently used", "other commands", undefined, undefined]);
    });

    it("typing with no recent match, names no group", () => {
        const { cmds, recent } = arrangeRecent(all, [], true);
        expect(paletteRows(cmds, true, recent).map((r) => r.group)).toEqual([undefined, undefined, undefined, undefined]);
    });

    it("typing, when every match is recent, is one group and no 'other commands'", () => {
        const { cmds, recent } = arrangeRecent(all, ["1", "2", "3", "4"], true);
        expect(paletteRows(cmds, true, recent).map((r) => r.group)).toEqual(["recently used", undefined, undefined, undefined]);
    });
});
