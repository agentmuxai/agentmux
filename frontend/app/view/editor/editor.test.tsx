// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The editor as a native pane tab (Pane Tab contract Phase 2c). */

import { describe, expect, it, vi } from "vitest";

const calls: string[] = [];
vi.mock("./editor-model", () => ({
    EditorViewModel: class {
        constructor(public ctx: any) {}
        viewName = () => "notes.md";
        viewText = () => "~/notes";
        getBodyContextMenuItems = () => [{ label: "Copy Path" }];
        giveFocus = () => (calls.push("focus"), true);
        dispose = () => calls.push("dispose");
    },
}));
vi.mock("./editor-view", () => ({ EditorViewComponent: () => null }));

import { editorPaneTab } from "./editor";

describe("editorPaneTab", () => {
    it("is native, kept alive, zooms from 13px, full-bleed", () => {
        expect(editorPaneTab.view).toBe("editor");
        expect(editorPaneTab.create).toBeTypeOf("function");
        expect(editorPaneTab.capabilities).toEqual({ lifecycle: "keepAlive", paneZoom: { baseFontSize: 13 }, noPadding: true });
    });

    it("hands the host its title, header text, menu, focus and dispose", () => {
        const inst = editorPaneTab.create!({ blockId: "e1" } as any);
        expect(inst.liveTitle!().text).toBe("notes.md");
        expect(inst.headerText!()).toBe("~/notes");
        // The header shows the manifest's plain icon; the file tree's toggle
        // is on the editor's own document bar.
        expect(inst.headerIcon).toBeUndefined();
        expect(editorPaneTab.icon).toBe("file-lines");
        expect(inst.contextMenu!()).toEqual([{ label: "Copy Path" }]);
        expect(inst.focus!()).toBe(true);
        inst.dispose!();
        expect(calls).toEqual(["focus", "dispose"]);
    });
});
