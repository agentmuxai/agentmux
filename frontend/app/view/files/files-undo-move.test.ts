// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** Undo for a move: when a move is certain enough to undo, and what goes back (files-undo-move.ts). */

import { describe, expect, it } from "vitest";
import { movedOk, planMoveUndo, restorable } from "./files-undo-move";

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
