// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";
import { FileTree } from "./file-tree";

const model = { rootsAtom: () => [], collapseAll: vi.fn(), refresh: vi.fn() } as any;

describe("the file tree's toolbar", () => {
    afterEach(() => cleanup());

    it("leads with the button that hides the tree", () => {
        const onHideTree = vi.fn();
        render(() => (
            <FileTree model={model} activeFilePath="" showHidden={false} onFileClick={() => {}} onToggleHidden={() => {}} onHideTree={onHideTree} />
        ));
        const buttons = [...document.querySelectorAll(".file-tree-toolbar button")].map((b) => b.getAttribute("aria-label"));
        expect(buttons).toEqual(["Hide file tree", "Show hidden files", "Collapse all folders", "Refresh tree"]);
        fireEvent.click(screen.getByRole("button", { name: "Hide file tree" }));
        expect(onHideTree).toHaveBeenCalledOnce();
    });

    it("has no hide button when nothing can hide it", () => {
        render(() => <FileTree model={model} activeFilePath="" showHidden={false} onFileClick={() => {}} onToggleHidden={() => {}} />);
        expect(screen.queryByRole("button", { name: "Hide file tree" })).toBeNull();
    });
});
