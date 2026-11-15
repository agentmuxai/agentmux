// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * "Show As" in a document tab's menu: the viewers the tab's file can be
 * shown in. SPEC_EDITOR_REVAMP_UNIFIED_VIEWERS_2026_10_15.md §4.2.
 */

import type { DocTab } from "@/app/doc-tabs/doc-tabs";
import type { EditorBuffer } from "@/app/store/editor-pane-state-store";
import { describe, expect, it, vi } from "vitest";
import { showAsMenuItems } from "./editor-tab-show-as";
import type { ViewerId } from "./viewers/viewer-choice";

function tab(over: Partial<DocTab<EditorBuffer>> = {}, payload: Partial<EditorBuffer> = {}): DocTab<EditorBuffer> {
    return {
        id: "t1",
        key: "C:/x/a.ts",
        title: "a.ts",
        preview: false,
        pinned: false,
        payload: { filePath: "C:/x/a.ts", ...payload } as EditorBuffer,
        ...over,
    } as DocTab<EditorBuffer>;
}

function model(options: ViewerId[]) {
    return { viewerTabs: { options: vi.fn(() => options) }, reopenWith: vi.fn() };
}

describe("Show As on a document tab's menu", () => {
    it("lists the viewers the file can be shown in, the one showing it checked, and shows the pick", () => {
        const m = model(["text", "info", "hex"]);
        const [item] = showAsMenuItems(m as never, tab());
        expect(item.label).toBe("Show As");
        expect(item.enabled).toBe(true);
        expect(item.submenu!.map((i) => [i.label, i.checked])).toEqual([
            ["Text", true],
            ["File info", false],
            ["Hex", false],
        ]);
        // A tab not yet given a viewer shows text.
        expect(m.viewerTabs.options).toHaveBeenCalledWith("t1", "text", undefined);
        item.submenu![2].click!();
        expect(m.reopenWith).toHaveBeenCalledWith("t1", "hex");
    });

    it("checks the viewer the tab is in", () => {
        const [item] = showAsMenuItems(model(["image", "info", "text", "hex"]) as never, tab({}, { viewer: "hex" }));
        expect(item.submenu!.filter((i) => i.checked).map((i) => i.label)).toEqual(["Hex"]);
    });

    it("is off while the tab has unsaved changes", () => {
        const [item] = showAsMenuItems(model(["text", "info", "hex"]) as never, tab({ dirty: true }));
        expect(item.enabled).toBe(false);
    });

    it("isn't there for a file with one viewer, or one not read yet", () => {
        expect(showAsMenuItems(model(["text"]) as never, tab())).toEqual([]);
    });
});
