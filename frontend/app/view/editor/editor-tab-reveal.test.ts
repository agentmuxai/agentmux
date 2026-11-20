// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * "Reveal in File Explorer" on a document tab's menu.
 * docs/specs/SPEC_EDITOR_TAB_REVEAL_IN_FILE_MANAGER_2026_11_17.md §3.1, §3.2, §5.
 */

import type { DocTab } from "@/app/doc-tabs/doc-tabs";
import type { EditorBuffer } from "@/app/store/editor-pane-state-store";
import { getPlatform, setPlatform } from "@/util/platformutil";
import { afterEach, describe, expect, it, vi } from "vitest";
import { revealMenuItems } from "./editor-tab-reveal";

function tab(payload: Partial<EditorBuffer> = {}): DocTab<EditorBuffer> {
    return {
        id: "t1",
        key: "C:/x/a.ts",
        title: "a.ts",
        preview: false,
        pinned: false,
        payload: { filePath: "C:/x/a.ts", ...payload } as EditorBuffer,
    } as DocTab<EditorBuffer>;
}

function model(connection = "") {
    return { connection: vi.fn(() => connection), revealInExplorer: vi.fn(async () => {}) };
}

describe("Reveal in the file manager on a document tab's menu", () => {
    const original = getPlatform();
    afterEach(() => setPlatform(original));

    it.each([
        ["win32", "Reveal in File Explorer"],
        ["darwin", "Reveal in Finder"],
        ["linux", "Open Containing Folder"],
    ] as const)("a saved local tab has the row, named for %s, and choosing it reveals the tab's file", (platform, label) => {
        setPlatform(platform);
        const m = model();
        const items = revealMenuItems(m, tab());
        expect(items).toHaveLength(1);
        expect(items[0].label).toBe(label);
        expect(items[0].enabled).toBe(true);
        expect(items[0].sublabel).toBeUndefined();
        items[0].click!();
        expect(m.revealInExplorer).toHaveBeenCalledOnce();
        expect(m.revealInExplorer).toHaveBeenCalledWith("C:/x/a.ts");
    });

    it("an Untitled tab has no row: its file does not exist yet", () => {
        expect(revealMenuItems(model(), tab({ isScratch: true, filePath: "" }))).toEqual([]);
        expect(revealMenuItems(model(), tab({ isScratch: true }))).toEqual([]);
    });

    it("a dropped file's tab has no row: it has no path on disk", () => {
        expect(revealMenuItems(model(), tab({ filePath: "agentmux-dropped:/1/a.png" }))).toEqual([]);
    });

    it("a tab on an SSH host has the row disabled, with the host as the reason, and calls nothing", () => {
        const m = model("dev@build-box");
        const [item] = revealMenuItems(m, tab());
        expect(item.enabled).toBe(false);
        expect(item.sublabel).toBe("The file is on dev@build-box");
        expect(m.revealInExplorer).not.toHaveBeenCalled();
    });
});
