// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** A Terminal or Hangar tile starts where the pane that was split was.
 *  docs/specs/SPEC_SHORTCUTS_PANE_FOR_SPLITS_2026_10_28.md §2.4. */

import { describe, expect, it } from "vitest";
import { blockDefFromSource } from "./shortcuts-from";

const term: BlockDef = { meta: { view: "term", controller: "shell" } };
const files: BlockDef = { meta: { view: "files" } };

describe("blockDefFromSource", () => {
    it("a Terminal tile gets the source's directory and a remote connection", () => {
        expect(blockDefFromSource(term, { view: "term", "cmd:cwd": "C:/work", connection: "wsl://Ubuntu" })).toEqual({
            meta: { view: "term", controller: "shell", "cmd:cwd": "C:/work", connection: "wsl://Ubuntu" },
        });
    });

    it("a Hangar tile gets the directory too, as the folder it shows, whichever pane it came from", () => {
        expect(blockDefFromSource(files, { view: "term", "cmd:cwd": "C:/work" })).toEqual({ meta: { view: "files", "files:path": "C:/work" } });
        expect(blockDefFromSource(files, { view: "files", "files:path": "D:/docs" })).toEqual({ meta: { view: "files", "files:path": "D:/docs" } });
        expect(blockDefFromSource(term, { view: "files", "files:path": "D:/docs" })).toEqual({
            meta: { view: "term", controller: "shell", "cmd:cwd": "D:/docs" },
        });
    });

    it("a local connection is not carried (it would show the connection overlay)", () => {
        expect(blockDefFromSource(term, { "cmd:cwd": "C:/work", connection: "local" }).meta).not.toHaveProperty("connection");
    });

    it("the tile's own setting wins over the source's", () => {
        const own: BlockDef = { meta: { view: "term", "cmd:cwd": "C:/own" } };
        expect(blockDefFromSource(own, { "cmd:cwd": "C:/work" })).toBe(own);
    });

    it("any other tile, and a source that has none of it, is the tile as it is", () => {
        const browser: BlockDef = { meta: { view: "browser" } };
        expect(blockDefFromSource(browser, { "cmd:cwd": "C:/work", connection: "wsl://Ubuntu" })).toBe(browser);
        expect(blockDefFromSource(term, { view: "agent" })).toBe(term);
        expect(blockDefFromSource(term, undefined)).toBe(term);
    });

    it("does not change the tile it was given", () => {
        const def: BlockDef = { meta: { view: "term" } };
        blockDefFromSource(def, { "cmd:cwd": "C:/work" });
        expect(def).toEqual({ meta: { view: "term" } });
    });
});
