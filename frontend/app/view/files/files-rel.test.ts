// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** A tree row's name as a path in the shown folder (joinRel), and the extension of a row's name (extensionOf). */

import { describe, expect, it } from "vitest";
import { joinRel } from "./files-path";
import { extensionOf } from "./files-sort";

describe("row names as paths", () => {
    it("turns a row's name into a path in the shown folder's own spelling", () => {
        expect(joinRel("C:\\Users\\a", "src/lib/a.ts")).toBe("C:\\Users\\a\\src\\lib\\a.ts");
        expect(joinRel("C:\\Users\\a", "b.txt")).toBe("C:\\Users\\a\\b.txt");
        expect(joinRel("/home/a", "src/a.ts")).toBe("/home/a/src/a.ts");
        expect(joinRel("/", "etc/hosts")).toBe("/etc/hosts");
    });

    it("takes the extension from the file's own name, not from a folder's dot", () => {
        expect(extensionOf("src.d/Makefile")).toBe("");
        expect(extensionOf("src.d/a.TS")).toBe("ts");
        expect(extensionOf("a.md")).toBe("md");
    });
});
