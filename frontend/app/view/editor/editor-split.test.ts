// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** A split of an editor: an empty editor with the source's settings.
 *  SPEC_EDITOR_MEDIA_SPLIT_OPENS_EMPTY_2026_10_10.md. */

import { describe, expect, it } from "vitest";
import { editorSplitBlockDef } from "./editor-split";

const block = (meta: Record<string, unknown>) => ({ oid: "b1", meta }) as unknown as Block;

const SETTINGS = {
    "editor:tree_width": 320,
    "editor:tree_expanded": ["/home/me/src"],
    "editor:show_hidden": true,
    "editor:word_wrap": false,
    "editor:preview_height": 400,
};

describe("editorSplitBlockDef", () => {
    it("keeps the settings and none of the documents", () => {
        const source = block({
            view: "editor",
            ...SETTINGS,
            doctabs: { tabs: [{ path: "/home/me/src/a.ts" }], active: 0 },
            file: "/home/me/src/a.ts",
            "editor:line": 42,
            "editor:pending_open_files": ["/home/me/src/b.ts"],
            "editor:scratch": true,
            "frame:title": "My docs",
        });
        expect(editorSplitBlockDef(source)).toEqual({ meta: { view: "editor", ...SETTINGS } });
    });

    it("a source with no settings gives a bare editor", () => {
        expect(editorSplitBlockDef(block({ view: "editor", file: "/a.ts" }))).toEqual({ meta: { view: "editor" } });
    });

    it("keeps an SSH host, not a local or WSL connection", () => {
        expect(editorSplitBlockDef(block({ view: "editor", connection: "me@build-box" })).meta?.connection).toBe("me@build-box");
        expect(editorSplitBlockDef(block({ view: "editor", connection: "local" })).meta).not.toHaveProperty("connection");
        expect(editorSplitBlockDef(block({ view: "editor", connection: "wsl://Ubuntu" })).meta).not.toHaveProperty("connection");
    });
});
