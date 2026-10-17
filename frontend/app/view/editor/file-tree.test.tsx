// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";
import { FileTree } from "./file-tree";
import { FileTreeModel } from "./file-tree-model";

const h = vi.hoisted(() => ({ listings: new Map<string, { name: string; is_dir: boolean; is_symlink: boolean }[]>() }));

vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        ListEditorDirCommand: vi.fn(async (_c: unknown, req: { path: string }) => ({ path: req.path, entries: h.listings.get(req.path) ?? [] })),
    },
}));

const model = { rootsAtom: () => [], collapseAll: vi.fn(), refresh: vi.fn(), selectedPath: () => null } as unknown as FileTreeModel;

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

const entry = (name: string, isDir = false) => ({ name, is_dir: isDir, is_symlink: false });

/** C:\Users\a: src\ (main.ts, util.ts, zeta.ts), docs\ (guide.md), README.md. */
async function windowsTree(): Promise<FileTreeModel> {
    h.listings.clear();
    h.listings.set("C:\\Users\\a", [entry("docs", true), entry("src", true), entry("README.md")]);
    h.listings.set("C:\\Users\\a\\src", [entry("main.ts"), entry("util.ts"), entry("zeta.ts")]);
    h.listings.set("C:\\Users\\a\\docs", [entry("guide.md")]);
    const tree = new FileTreeModel();
    await tree.setRootsAndLoad("C:\\Users\\a", []);
    return tree;
}

function renderTree(tree: FileTreeModel, activeFilePath = "") {
    const calls = { preview: vi.fn(), keep: vi.fn(), focusEditor: vi.fn(), rename: vi.fn() };
    render(() => (
        <FileTree
            model={tree}
            activeFilePath={activeFilePath}
            showHidden={false}
            onFileClick={calls.preview}
            onFileDblClick={calls.keep}
            onToggleHidden={() => {}}
            onStartRename={calls.rename}
            onFocusEditor={calls.focusEditor}
        />
    ));
    return calls;
}

const item = (name: string) => screen.getByRole("treeitem", { name });
const press = (key: string) => fireEvent.keyDown(screen.getByRole("tree"), { key });

describe("the file tree's keyboard", () => {
    afterEach(() => {
        cleanup();
        vi.useRealTimers();
    });

    it("is a tree of treeitems with their place, level and state, and one tab stop", async () => {
        const tree = await windowsTree();
        await tree.expandFolder("C:\\Users\\a\\src");
        renderTree(tree);
        expect(screen.getByRole("tree")).toBeTruthy();
        const src = item("src");
        expect(src.getAttribute("aria-level")).toBe("2");
        expect(src.getAttribute("aria-expanded")).toBe("true");
        expect(src.getAttribute("aria-posinset")).toBe("2");
        expect(src.getAttribute("aria-setsize")).toBe("3");
        expect(item("docs").getAttribute("aria-expanded")).toBe("false");
        expect(item("util.ts").getAttribute("aria-level")).toBe("3");
        expect(item("util.ts").hasAttribute("aria-expanded")).toBe(false);
        expect(src.querySelector('[role="group"]')).toBeTruthy();
        // Nothing selected: the first row is the tab stop.
        const stops = [...document.querySelectorAll('[role="treeitem"][tabindex="0"]')];
        expect(stops).toEqual([item("a")]);
        tree.setSelectedPath("C:\\Users\\a\\src\\util.ts");
        expect([...document.querySelectorAll('[role="treeitem"][tabindex="0"]')]).toEqual([item("util.ts")]);
        expect(item("util.ts").getAttribute("aria-selected")).toBe("true");
    });

    it("previews the file ↓ lands on after a short pause, not each one it passes", async () => {
        const tree = await windowsTree();
        await tree.expandFolder("C:\\Users\\a\\src");
        const calls = renderTree(tree);
        tree.setSelectedPath("C:\\Users\\a\\src");
        vi.useFakeTimers();
        press("ArrowDown");
        press("ArrowDown");
        press("ArrowDown");
        expect(calls.preview).not.toHaveBeenCalled();
        expect(document.activeElement).toBe(item("zeta.ts"));
        vi.advanceTimersByTime(100);
        expect(calls.preview).toHaveBeenCalledExactlyOnceWith("C:\\Users\\a\\src\\zeta.ts");
        // Back up onto a folder: it's selected, nothing opens.
        press("Home");
        vi.advanceTimersByTime(100);
        expect(calls.preview).toHaveBeenCalledOnce();
        expect(tree.selectedPath()).toBe("C:\\Users\\a");
    });

    it("→ and ← open, enter, leave and close folders", async () => {
        const tree = await windowsTree();
        renderTree(tree);
        tree.setSelectedPath("C:\\Users\\a\\docs");
        press("ArrowRight");
        await waitFor(() => expect(item("guide.md")).toBeTruthy());
        press("ArrowRight");
        expect(tree.selectedPath()).toBe("C:\\Users\\a\\docs\\guide.md");
        press("ArrowLeft");
        expect(tree.selectedPath()).toBe("C:\\Users\\a\\docs");
        press("ArrowLeft");
        expect(tree.isExpanded("C:\\Users\\a\\docs")).toBe(false);
    });

    it("Enter keeps a file's tab and moves to the editor; Space keeps it and stays", async () => {
        const tree = await windowsTree();
        const calls = renderTree(tree);
        tree.setSelectedPath("C:\\Users\\a\\README.md");
        press(" ");
        expect(calls.keep).toHaveBeenCalledWith("C:\\Users\\a\\README.md");
        expect(calls.focusEditor).not.toHaveBeenCalled();
        press("Enter");
        expect(calls.keep).toHaveBeenCalledTimes(2);
        expect(calls.focusEditor).toHaveBeenCalledOnce();
        // On a folder, Enter opens it.
        tree.setSelectedPath("C:\\Users\\a\\docs");
        press("Enter");
        expect(tree.isExpanded("C:\\Users\\a\\docs")).toBe(true);
    });

    it("Escape moves to the editor; F2 renames the selected row but never a root", async () => {
        const tree = await windowsTree();
        const calls = renderTree(tree);
        press("Escape");
        expect(calls.focusEditor).toHaveBeenCalledOnce();
        press("F2");
        expect(calls.rename).not.toHaveBeenCalled();
        tree.setSelectedPath("C:\\Users\\a\\README.md");
        press("F2");
        expect(calls.rename).toHaveBeenCalledWith("C:\\Users\\a\\README.md");
    });

    it("type-ahead selects the next row starting with the typed letters", async () => {
        const tree = await windowsTree();
        renderTree(tree);
        press("r");
        expect(tree.selectedPath()).toBe("C:\\Users\\a\\README.md");
    });

    it("leaves modified keys to the app's shortcuts", async () => {
        const tree = await windowsTree();
        renderTree(tree);
        const ev = new KeyboardEvent("keydown", { key: "ArrowDown", ctrlKey: true, shiftKey: true, bubbles: true, cancelable: true });
        screen.getByRole("tree").dispatchEvent(ev);
        expect(ev.defaultPrevented).toBe(false);
        expect(tree.selectedPath()).toBeNull();
    });
});

describe("the open file in the tree", () => {
    afterEach(() => cleanup());

    it("is marked when its path differs only in separators and the drive letter's case", async () => {
        const tree = await windowsTree();
        renderTree(tree, "c:/Users/a/README.md");
        const row = item("README.md").querySelector(".file-tree-row")!;
        expect(row.classList.contains("file-tree-row--open")).toBe(true);
        expect(item("docs").querySelector(".file-tree-row")!.classList.contains("file-tree-row--open")).toBe(false);
        // It keeps its file type's icon.
        expect(row.querySelector(".file-tree-icon")!.className).toContain("fa-circle-info");
    });

    it("is revealed: its folders open and it becomes the selected row", async () => {
        const tree = await windowsTree();
        renderTree(tree, "c:/Users/a/src/util.ts");
        await waitFor(() => expect(tree.selectedPath()).toBe("C:\\Users\\a\\src\\util.ts"));
        expect(tree.isExpanded("C:\\Users\\a\\src")).toBe(true);
        expect(item("util.ts").querySelector(".file-tree-row")!.classList.contains("file-tree-row--open")).toBe(true);
    });

    it("isn't revealed when it is outside every root", async () => {
        const tree = await windowsTree();
        renderTree(tree, "d:/elsewhere/x.ts");
        await Promise.resolve();
        expect(tree.selectedPath()).toBeNull();
    });
});
