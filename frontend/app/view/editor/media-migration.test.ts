// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A saved Media pane reopens as an Editor
 * (SPEC_EDITOR_REVAMP_UNIFIED_VIEWERS_2026_10_15.md §4.12.2): each historic
 * Media meta shape.
 */

import { describe, expect, it } from "vitest";
import { deserializeEditorBuffer } from "@/app/store/editor-pane-state-store";
import { applyMetaPatch, migrateMediaMeta } from "./media-migration";

describe("migrateMediaMeta", () => {
    it("leaves an Editor's meta alone", () => {
        expect(migrateMediaMeta({ view: "editor", file: "C:/a.ts" })).toBeNull();
        expect(migrateMediaMeta(undefined)).toBeNull();
    });

    it("a pane from before tabs: its one file, following its folder", () => {
        const patch = migrateMediaMeta({ view: "media", "media:path": "C:\\renders\\out.png" })!;
        expect(patch).toMatchObject({
            view: "editor",
            file: "C:\\renders\\out.png",
            "editor:follow_folder": true,
            "media:path": null,
            "editor:tree_expanded": false,
        });
    });

    it("doc tabs with files and an empty tab: the files as Editor tabs, the empty one gone", () => {
        const patch = migrateMediaMeta({
            view: "media",
            "media:path": "C:\\r\\b.mp4",
            doctabs: {
                v: 1,
                active: 2,
                tabs: [
                    { key: "C:\\r\\a.png", title: "a.png", icon: "image", state: "C:\\r\\a.png" },
                    { key: "blank:1", title: "New Tab", state: "" },
                    { key: "C:\\r\\b.mp4", title: "b.mp4", icon: "film", state: "C:\\r\\b.mp4" },
                ],
            },
        })!;
        const record = patch.doctabs as { tabs: { key: string; state: unknown }[]; active: number };
        expect(record.tabs.map((t) => t.key)).toEqual(["c:/r/a.png", "c:/r/b.mp4"]);
        expect(record.active).toBe(1);
        // With tabs, media:path doesn't add a second copy of a file.
        expect(patch.file).toBeUndefined();
        // Each state reads back as a buffer that follows its folder.
        const buffer = deserializeEditorBuffer(record.tabs[0].state)!;
        expect(buffer.filePath).toBe("c:/r/a.png");
        expect(buffer.followFolder).toBe(true);
    });

    it("a pending media:open moves to the Editor's queue, following the folder", () => {
        const patch = migrateMediaMeta({ view: "media", "media:open": [{ id: "r1", path: "C:/r/c.png" }, { id: "bad" }] })!;
        expect(patch["editor:pending_open_files"]).toEqual([{ path: "C:/r/c.png", follow_folder: true }]);
        expect(patch["media:open"]).toBeNull();
    });

    it("applies a patch, removing the keys it nulls", () => {
        const meta = { view: "media", "media:path": "C:/x.png", "frame:title": "Renders" };
        expect(applyMetaPatch(meta, migrateMediaMeta(meta)!)).toEqual({
            view: "editor",
            file: "C:/x.png",
            "editor:follow_folder": true,
            "editor:tree_expanded": false,
            "frame:title": "Renders",
        });
    });
});
