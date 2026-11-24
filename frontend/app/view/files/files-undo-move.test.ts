// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** Undo for a move: when a move is certain enough to undo, and what goes back (files-undo-move.ts). */

import { describe, expect, it } from "vitest";
import { landedAsPlanned, movedOk, planMoveUndo, restorable, type UndoPlan } from "./files-undo-move";

const names = (...n: string[]) => new Set(n.map((x) => x.toLowerCase()));

describe("planMoveUndo", () => {
    it("plans each item to land at <destination>/<its name>", () => {
        expect(planMoveUndo(["/a/x.txt", "/a/y"], "/b", names("other"))).toEqual([
            { from: "/a/x.txt", to: "/b/x.txt" },
            { from: "/a/y", to: "/b/y" },
        ]);
    });

    it("refuses when a name is already there, whatever its case, so a Replace or Keep both can't confuse it", () => {
        expect(planMoveUndo(["/a/X.txt"], "/b", names("x.TXT"))).toBeNull();
    });

    it("refuses two sources with one name, a source already in the destination, and no sources", () => {
        expect(planMoveUndo(["/a/x", "/c/x"], "/b", names())).toBeNull();
        expect(planMoveUndo(["/b/x"], "/b/", names())).toBeNull();
        expect(planMoveUndo([], "/b", names())).toBeNull();
    });

    it("spells Windows paths with their own separator", () => {
        expect(planMoveUndo(["C:\\Users\\a\\b.txt"], "C:\\Users\\a\\src", names())).toEqual([
            { from: "C:\\Users\\a\\b.txt", to: "C:\\Users\\a\\src\\b.txt" },
        ]);
    });
});

describe("landedAsPlanned", () => {
    const plan: UndoPlan = {
        items: [{ from: "/a/x", to: "/b/x" }],
        destDir: "/b",
        before: names("old"),
        children: new Map<string, ReadonlySet<string>>(),
    };

    it("is true when the destination gained exactly the names of the items that went through", () => {
        expect(landedAsPlanned(plan, plan.items, names("old", "x"))).toBe(true);
        expect(landedAsPlanned(plan, plan.items, names("OLD", "X"))).toBe(true);
    });

    it("is false when a conflict put the item under another name, or the destination gained anything else", () => {
        // Something took x after the check, so ours was kept as "x (2)".
        expect(landedAsPlanned(plan, plan.items, names("old", "x", "x (2)"))).toBe(false);
        expect(landedAsPlanned(plan, plan.items, names("old", "x", "other"))).toBe(false);
        expect(landedAsPlanned(plan, plan.items, names("old", "y"))).toBe(false);
    });

    it("is false when a folder met one of its name in the destination, and so was merged with what that held", () => {
        const folderPlan: UndoPlan = {
            items: [{ from: "/a/docs", to: "/b/docs" }],
            destDir: "/b",
            before: names("old"),
            children: new Map([["docs", names("a.md")]]),
        };
        const after = names("old", "docs");
        expect(landedAsPlanned(folderPlan, folderPlan.items, after, new Map([["docs", names("a.md")]]))).toBe(true);
        // Something created docs with its own files after the check: the folder was merged into it.
        expect(landedAsPlanned(folderPlan, folderPlan.items, after, new Map([["docs", names("a.md", "other.txt")]]))).toBe(false);
        expect(landedAsPlanned(folderPlan, folderPlan.items, after, new Map([["docs", null]]))).toBe(false);
        expect(landedAsPlanned(folderPlan, folderPlan.items, after, new Map())).toBe(false);
    });

    it("is false when the destination can't be listed or nothing went through", () => {
        expect(landedAsPlanned(plan, plan.items, null)).toBe(false);
        expect(landedAsPlanned(plan, [], names("old"))).toBe(false);
    });
});

describe("movedOk", () => {
    it("leaves out the sources srv reports as failed", () => {
        const items = [
            { from: "/a/x", to: "/b/x" },
            { from: "/a/y", to: "/b/y" },
        ];
        expect(movedOk(items, ["/a/y"])).toEqual([items[0]]);
        expect(movedOk(items, undefined)).toEqual(items);
    });
});

describe("restorable, on a volume that tells A from a", () => {
    it("returns the items to the folder as spelled, not to one that differs only in case", () => {
        const lists: Record<string, Set<string>> = { "/b": names("one", "two"), "/s/A": names(), "/s/a": names() };
        const groups = restorable(
            [
                { from: "/s/A/one", to: "/b/one" },
                { from: "/s/a/two", to: "/b/two" },
            ],
            (d) => lists[d] ?? null
        );
        expect(groups).toEqual([
            { parent: "/s/A", paths: ["/b/one"] },
            { parent: "/s/a", paths: ["/b/two"] },
        ]);
    });
});

describe("restorable", () => {
    const listing: Record<string, Set<string> | null> = {
        "/a": names("keep"),
        "/b": names("x", "y", "z"),
        "/c": null,
    };
    const lookup = (d: string) => listing[d] ?? null;

    it("returns the items still in the destination whose name is free where they came from, by the folder they return to", () => {
        const items = [
            { from: "/a/x", to: "/b/x" },
            { from: "/a/y", to: "/b/y" },
        ];
        expect(restorable(items, lookup)).toEqual([{ parent: "/a", paths: ["/b/x", "/b/y"] }]);
    });

    it("skips an item that is gone from the destination, or whose old place has been taken", () => {
        listing["/a"] = names("keep", "y");
        const items = [
            { from: "/a/x", to: "/b/x" },
            { from: "/a/y", to: "/b/y" },
            { from: "/a/gone", to: "/b/gone" },
        ];
        expect(restorable(items, lookup)).toEqual([{ parent: "/a", paths: ["/b/x"] }]);
        listing["/a"] = names("keep");
    });

    it("skips what it can't list", () => {
        expect(restorable([{ from: "/c/x", to: "/b/x" }], lookup)).toEqual([]);
    });
});
