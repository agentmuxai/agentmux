// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The pure parts of renaming from a document tab: the first selection, what
 * the box refuses by itself, an Untitled document's destination, and how two
 * spellings of a path compare.
 * docs/specs/SPEC_EDITOR_TAB_RENAME_2026_11_14.md §11.4, §11.5, §11.6.
 */

import { describe, expect, it } from "vitest";
import {
    baseName,
    dirName,
    errorSentence,
    nameStemEnd,
    nextNameSelection,
    pathAfterRename,
    samePath,
    typedNameProblem,
    untitledDestination,
} from "./editor-rename-names";

describe("what the name box selects", () => {
    it("stops at the last dot, and takes a name whole when it has no extension to keep", () => {
        expect(nameStemEnd("notes.md")).toBe(5);
        expect(nameStemEnd("archive.tar.gz")).toBe("archive.tar".length);
        expect(nameStemEnd(".env")).toBe(4);
        expect(nameStemEnd("Makefile")).toBe(8);
        expect(nameStemEnd("Untitled-1")).toBe(10);
    });

    it("F2 moves on: the name, the whole name, the extension, and round again", () => {
        const name = "notes.md";
        expect(nextNameSelection(name, 0, 5)).toEqual([0, 8]);
        expect(nextNameSelection(name, 0, 8)).toEqual([6, 8]);
        expect(nextNameSelection(name, 6, 8)).toEqual([0, 5]);
        // A caret somewhere in the text starts over at the name.
        expect(nextNameSelection(name, 2, 2)).toEqual([0, 5]);
        expect(nextNameSelection(".env", 0, 4)).toEqual([0, 4]);
    });
});

describe("what the box refuses by itself", () => {
    it("a control character, anywhere and on every platform", () => {
        expect(typedNameProblem("a" + String.fromCharCode(9) + "b.txt")).toBe("A name can't contain control characters.");
        expect(typedNameProblem("a" + String.fromCharCode(0x7f))).toBe("A name can't contain control characters.");
    });

    it("a hidden direction character, which can make a name read as another", () => {
        for (const code of [0x202e, 0x202a, 0x2066, 0x2069, 0x200f]) {
            expect(typedNameProblem("gnp" + String.fromCharCode(code) + ".exe"), code.toString(16)).toBe(
                "A name can't contain hidden direction characters."
            );
        }
    });

    it("leaves the rest to srv, which knows the platform", () => {
        for (const name of ["notes.md", "nul.txt", "a:b", "über café.txt", "日本語.md", "a" + String.fromCharCode(0x200d) + "b"]) {
            expect(typedNameProblem(name), name).toBeNull();
        }
    });
});

describe("where an Untitled document goes", () => {
    it("a name goes into the folder, and the name is kept as typed", () => {
        expect(untitledDestination("notes.md", "c:/repo/docs")).toEqual({ path: "c:/repo/docs/notes.md" });
        expect(untitledDestination("  notes  ", "c:/repo/docs")).toEqual({ path: "c:/repo/docs/notes" });
        expect(untitledDestination("notes.md", "~")).toEqual({ path: "~/notes.md" });
        expect(untitledDestination("notes.md", "")).toEqual({ path: "~/notes.md" });
        expect(untitledDestination("notes.md", "c:/")).toEqual({ path: "c:/notes.md" });
    });

    it("a relative path goes under the folder; a home or a full path is used as given", () => {
        expect(untitledDestination("drafts/notes.md", "/home/a/site")).toEqual({ path: "/home/a/site/drafts/notes.md" });
        expect(untitledDestination("~/x/notes.md", "/home/a/site")).toEqual({ path: "~/x/notes.md" });
        expect(untitledDestination("/tmp/notes.md", "/home/a/site")).toEqual({ path: "/tmp/notes.md" });
        expect(untitledDestination("C:\\x\\notes.md", "c:/repo")).toEqual({ path: "C:\\x\\notes.md" });
        expect(untitledDestination("\\\\nas\\share\\notes.md", "c:/repo")).toEqual({ path: "\\\\nas\\share\\notes.md" });
    });

    it("refuses a path that climbs, a path with no name, and what a name may not hold", () => {
        expect(untitledDestination("../notes.md", "c:/repo")).toEqual({ problem: "A path can't contain “..”." });
        expect(untitledDestination("a/../../notes.md", "c:/repo")).toEqual({ problem: "A path can't contain “..”." });
        expect(untitledDestination("drafts/", "c:/repo")).toEqual({ problem: "A name can't be empty." });
        expect(untitledDestination("a" + String.fromCharCode(0x202e) + "b", "c:/repo")).toEqual({
            problem: "A name can't contain hidden direction characters.",
        });
        // Two dots inside a name are a name.
        expect(untitledDestination("notes..md", "c:/repo")).toEqual({ path: "c:/repo/notes..md" });
    });
});

describe("paths", () => {
    it("the last part and the folder", () => {
        expect(baseName("c:/repo/a.ts")).toBe("a.ts");
        expect(baseName("C:\\repo\\a.ts")).toBe("a.ts");
        expect(dirName("c:/repo/a.ts")).toBe("c:/repo");
        expect(dirName("/a.ts")).toBe("/");
        expect(dirName("c:/a.ts")).toBe("c:/");
        expect(dirName("a.ts")).toBe("");
    });

    it("two spellings of one Windows path are one path; case counts elsewhere", () => {
        expect(samePath("C:\\Repo\\A.ts", "c:/repo/a.ts")).toBe(true);
        expect(samePath("\\\\?\\C:\\repo\\a.ts", "c:/repo/a.ts")).toBe(true);
        expect(samePath("/home/a/Readme.md", "/home/a/readme.md")).toBe(false);
        expect(samePath("/home/a/readme.md", "/home/a/readme.md")).toBe(true);
    });

    it("a renamed file takes the new path, and a file inside a renamed folder moves with it", () => {
        expect(pathAfterRename("c:/repo/a.ts", "C:\\repo\\a.ts", "C:\\repo\\b.ts")).toBe("c:/repo/b.ts");
        expect(pathAfterRename("c:/repo/src/x/a.ts", "C:\\repo\\src", "C:\\repo\\lib")).toBe("c:/repo/lib/x/a.ts");
        // A folder that only starts with the same letters is another folder.
        expect(pathAfterRename("c:/repo/src2/a.ts", "c:/repo/src", "c:/repo/lib")).toBeNull();
        expect(pathAfterRename("c:/repo/other.ts", "c:/repo/a.ts", "c:/repo/b.ts")).toBeNull();
        // A change of case only is still a new path.
        expect(pathAfterRename("c:/repo/readme.md", "c:/repo/readme.md", "c:/repo/README.md")).toBe("c:/repo/README.md");
    });
});

describe("a failure as srv's sentence", () => {
    it("drops the wrappers and keeps the sentence", () => {
        expect(errorSentence(new Error("Something named “b.txt” already exists here."))).toBe("Something named “b.txt” already exists here.");
        expect(errorSentence(new Error("Error: fs.rename: It doesn't exist anymore."))).toBe("It doesn't exist anymore.");
        expect(errorSentence(new Error("movescratchfile: scratch file not found"))).toBe("scratch file not found");
        // A colon inside a sentence is part of it.
        expect(errorSentence(new Error("Windows doesn't allow : in a name."))).toBe("Windows doesn't allow : in a name.");
        expect(errorSentence("Couldn't rename it: disk error")).toBe("Couldn't rename it: disk error");
    });
});
