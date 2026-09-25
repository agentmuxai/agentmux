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
        viewIcon = () => ({ elemtype: "iconbutton", icon: "file-lines" });
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
        expect(editorPaneTab.viewModelClass).toBeUndefined();
        expect(editorPaneTab.capabilities).toEqual({ lifecycle: "keepAlive", paneZoom: { baseFontSize: 13 }, noPadding: true });
    });

    it("hands the host its title, header text and icon, menu, focus and dispose", () => {
        const inst = editorPaneTab.create!({ blockId: "e1" } as any);
        expect(inst.liveTitle!().text).toBe("notes.md");
        expect(inst.headerText!()).toBe("~/notes");
        expect(inst.headerIcon!()).toEqual({ elemtype: "iconbutton", icon: "file-lines" });
        expect(inst.contextMenu!()).toEqual([{ label: "Copy Path" }]);
        expect(inst.focus!()).toBe(true);
        inst.dispose!();
        expect(calls).toEqual(["focus", "dispose"]);
    });
});
