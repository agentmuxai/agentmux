// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The temp-tree guard verify-shortcuts.mjs relies on before every row that
// changes files on disk (PLAN_SHORTCUT_KINKS_2026_10_10.md, L2).

import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { insideTree, isWithinPath, makeTempTree, removeTempTree, TEMP_PREFIX } from "./verify-shortcuts-temp-tree.mjs";

describe("isWithinPath", () => {
    it("is true for the folder itself and anything below it", () => {
        expect(isWithinPath("/tmp/t", "/tmp/t", "linux")).toBe(true);
        expect(isWithinPath("/tmp/t/l1/l2/start", "/tmp/t", "linux")).toBe(true);
        expect(isWithinPath("/tmp/t/l1/", "/tmp/t/", "linux")).toBe(true);
    });

    it("is false above it, beside it, and for a sibling sharing its prefix", () => {
        expect(isWithinPath("/tmp", "/tmp/t", "linux")).toBe(false);
        expect(isWithinPath("/home/u", "/tmp/t", "linux")).toBe(false);
        expect(isWithinPath("/tmp/t2", "/tmp/t", "linux")).toBe(false);
        expect(isWithinPath("/tmp/t/../u", "/tmp/t", "linux")).toBe(false);
    });

    it("compares case-insensitively on Windows and macOS, exactly on Linux", () => {
        expect(isWithinPath("C:\\Users\\A\\Temp\\T\\l1", "c:\\users\\a\\temp\\t", "win32")).toBe(true);
        expect(isWithinPath("C:\\Users\\A\\Temp\\T2", "C:\\Users\\A\\Temp\\T", "win32")).toBe(false);
        expect(isWithinPath("/private/var/T/l1", "/private/var/t", "darwin")).toBe(true);
        expect(isWithinPath("/tmp/T/l1", "/tmp/t", "linux")).toBe(false);
    });
});

describe("the temp tree", () => {
    const made = [];
    afterEach(() => {
        for (const root of made.splice(0)) fs.rmSync(root, { recursive: true, force: true });
    });

    it("is three folders deep under the OS temp folder, with files to act on", () => {
        const { root, start } = makeTempTree();
        made.push(root);
        expect(path.basename(root).startsWith(TEMP_PREFIX)).toBe(true);
        expect(path.relative(root, start).split(path.sep)).toEqual(["l1", "l2", "start"]);
        expect(fs.readdirSync(start).sort()).toEqual(["b.txt", "notes.md", "notes.txt", "sub"]);
        // Up twice from start is still inside.
        expect(insideTree(path.dirname(path.dirname(start)), root)).toBe(true);
        expect(insideTree(path.dirname(root), root)).toBe(false);
        expect(insideTree("", root)).toBe(false);
    });

    it("sees through a symlink to the tree (macOS's /var is /private/var)", () => {
        const { root, start } = makeTempTree();
        made.push(root);
        const link = path.join(os.tmpdir(), `${TEMP_PREFIX}link-${process.pid}`);
        // A junction on Windows: a directory symlink there needs admin rights.
        fs.symlinkSync(root, link, process.platform === "win32" ? "junction" : "dir");
        try {
            expect(insideTree(path.join(link, "l1", "l2", "start"), root)).toBe(true);
            expect(insideTree(start, link)).toBe(true);
        } finally {
            fs.unlinkSync(link);
        }
    });

    it("removes only a tree it made", () => {
        const { root } = makeTempTree();
        removeTempTree(root);
        expect(fs.existsSync(root)).toBe(false);
        const other = fs.mkdtempSync(path.join(os.tmpdir(), "not-ours-"));
        made.push(other);
        expect(() => removeTempTree(other)).toThrow(/refusing/);
        expect(() => removeTempTree(os.homedir())).toThrow(/refusing/);
        expect(fs.existsSync(other)).toBe(true);
    });
});
