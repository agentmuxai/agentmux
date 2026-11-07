// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The editor file tree's state: the visible rows and the one tab stop, what a click or a key does to the
// selection and to the open file, revealing the open file, and the WAI-ARIA tree pattern's keys.
// The key logic itself is pure, in tree-keyboard.ts.

import { createEffect, createMemo, on, onCleanup, type Accessor } from "solid-js";
import type { FileTreeModel } from "./file-tree-model";
import { isOpenFile, treeMove, TreeTypeAhead, visibleRows, type TreeRow } from "./tree-keyboard";

export interface FileTreeProps {
    model: FileTreeModel;
    activeFilePath: string;
    showHidden: boolean;
    /** Called on file row single-click — VS Code-style "preview" open. */
    onFileClick: (path: string) => void;
    /** Called on file row double-click — pins the tab. */
    onFileDblClick?: (path: string) => void;
    onToggleHidden: () => void;
    /** Hides the file tree: the toolbar's first button. */
    onHideTree?: () => void;
    /** Right-click on a node or empty tree background.
     *  path=null means the background was right-clicked. */
    onContextMenu?: (path: string | null, isDir: boolean, e: MouseEvent) => void;
    /** When non-null, the node at this path renders an inline rename input. */
    renamingPath?: string | null;
    onRenameConfirm?: (path: string, newName: string) => void;
    onRenameCancel?: () => void;
    /** When set, renders an inline new-entry input inside the specified parent. */
    newEntry?: { parentPath: string; kind: "file" | "dir" } | null;
    onNewEntryConfirm?: (parentPath: string, name: string, kind: "file" | "dir") => void;
    onNewEntryCancel?: () => void;
    /** Called when F2 is pressed in the tree — trigger rename for the given path. */
    onStartRename?: (path: string) => void;
    /** Enter on a file, or Escape: the keyboard goes to the editor body. */
    onFocusEditor?: () => void;
}

export type FileTreeState = {
    tabStop: Accessor<string | null>;
    setTreeRef: (el: HTMLDivElement) => void;
    registerRow: (path: string, el: HTMLElement, mounted: boolean) => void;
    preview: (path: string) => void;
    keep: (path: string) => void;
    handleKeyDown: (e: KeyboardEvent) => void;
    handleBackgroundContextMenu: (e: MouseEvent) => void;
};

/** How long a file waits under the keyboard's selection before it previews,
 *  so holding ↓ doesn't load every file it passes (spec §5.3). */
const PREVIEW_DELAY_MS = 100;

export function createFileTreeState(props: FileTreeProps): FileTreeState {
    let treeRef: HTMLDivElement | undefined;
    const rowEls = new Map<string, HTMLElement>();
    const registerRow = (path: string, el: HTMLElement, mounted: boolean) => {
        if (mounted) rowEls.set(path, el);
        else if (rowEls.get(path) === el) rowEls.delete(path);
    };

    const rows = createMemo(() =>
        visibleRows(
            {
                roots: props.model.rootsAtom(),
                isExpanded: (p) => props.model.isExpanded(p),
                entries: (p) => props.model.getNodeData(p)?.entries,
            },
            props.showHidden
        )
    );
    // The one row in the Tab order (roving tabindex): the selected row while
    // it's visible, else the first.
    const tabStop = createMemo(() => {
        const selected = props.model.selectedPath();
        const list = rows();
        return list.find((r) => r.path === selected)?.path ?? list[0]?.path ?? null;
    });
    const treeHasFocus = () => !!treeRef?.contains(document.activeElement);

    // A file opened from the tree: the reveal below skips it, since the user
    // is already on its row. Only when it changes the open file, or nothing
    // would clear it.
    let openedHere: string | null = null;
    let previewTimer: ReturnType<typeof setTimeout> | undefined;
    onCleanup(() => clearTimeout(previewTimer));
    const openFrom = (open: (path: string) => void, path: string) => {
        clearTimeout(previewTimer);
        if (!isOpenFile(path, props.activeFilePath)) openedHere = path;
        open(path);
    };
    const preview = (path: string) => openFrom(props.onFileClick, path);
    const keep = (path: string) => openFrom(props.onFileDblClick ?? props.onFileClick, path);

    const scrollToRow = (path: string) => {
        // The row itself, not its item: an open folder's item holds its children.
        rowEls.get(path)?.querySelector(".file-tree-row")?.scrollIntoView?.({ block: "nearest" });
    };
    /** Select a row from the keyboard, and preview it if it's a file. */
    const select = (path: string) => {
        props.model.setSelectedPath(path);
        rowEls.get(path)?.focus({ preventScroll: true });
        scrollToRow(path);
        clearTimeout(previewTimer);
        if (rows().find((r) => r.path === path)?.isDir === false) {
            previewTimer = setTimeout(() => preview(path), PREVIEW_DELAY_MS);
        }
    };

    // Reveal the open file (VS Code's explorer.autoReveal): open the folders
    // down to it, select it unless the user is in the tree, and scroll it into
    // view if it's off screen. Again once the roots arrive.
    createEffect(
        on([() => props.activeFilePath, () => props.model.rootsAtom()], ([file]) => {
            if (!file) return;
            if (openedHere != null && isOpenFile(openedHere, file)) {
                openedHere = null;
                return;
            }
            void props.model.revealPath(file).then((row) => {
                if (row == null || !isOpenFile(row, props.activeFilePath)) return;
                if (!treeHasFocus()) props.model.setSelectedPath(row);
                scrollToRow(row);
            });
        })
    );

    const handleKeyDown = createTreeKeyHandler(props, { rows, tabStop, select, keep, treeRef: () => treeRef });

    const handleBackgroundContextMenu = (e: MouseEvent) => {
        if (props.onContextMenu) {
            e.preventDefault();
            e.stopPropagation();
            props.onContextMenu(null, false, e);
        }
    };

    return {
        tabStop,
        setTreeRef: (el) => {
            treeRef = el;
        },
        registerRow,
        preview,
        keep,
        handleKeyDown,
        handleBackgroundContextMenu,
    };
}

/** The WAI-ARIA tree pattern's keys (spec §5.2). Modified keys are left to the app's shortcuts. */
function createTreeKeyHandler(
    props: FileTreeProps,
    tree: {
        rows: Accessor<TreeRow[]>;
        tabStop: Accessor<string | null>;
        select: (path: string) => void;
        keep: (path: string) => void;
        treeRef: () => HTMLDivElement | undefined;
    }
): (e: KeyboardEvent) => void {
    const { rows, tabStop, select, keep } = tree;
    const typeAhead = new TreeTypeAhead();
    const pageSize = (): number => {
        const treeRef = tree.treeRef();
        const rowHeight = treeRef?.querySelector<HTMLElement>(".file-tree-row")?.offsetHeight ?? 0;
        if (!treeRef || rowHeight <= 0) return 10;
        return Math.max(1, Math.floor(treeRef.clientHeight / rowHeight) - 1);
    };

    return (e: KeyboardEvent) => {
        if (e.defaultPrevented || e.ctrlKey || e.metaKey || e.altKey) return;
        const list = rows();
        const current = tabStop();
        const row = list.find((r) => r.path === current);
        const handled = () => {
            e.preventDefault();
            e.stopPropagation();
        };
        const move = treeMove(list, current, e.key, pageSize());
        if (move) {
            handled();
            if (move.kind === "select") select(move.path);
            else if (move.kind === "expand") void props.model.expandFolder(move.path);
            else if (move.kind === "collapse") props.model.collapseFolder(move.path);
            return;
        }
        switch (e.key) {
            case "Enter":
                if (!row) return;
                handled();
                if (row.isDir) {
                    void props.model.toggleExpand(row.path);
                } else {
                    keep(row.path);
                    props.onFocusEditor?.();
                }
                return;
            case " ":
                // A space inside a name being typed belongs to it.
                if (typeAhead.active()) break;
                if (!row) return;
                handled();
                if (row.isDir) void props.model.toggleExpand(row.path);
                else keep(row.path);
                return;
            case "F2":
                // Not a root: those are HOME and the drives.
                if (!props.onStartRename || !row || row.depth === 0) return;
                handled();
                props.onStartRename(row.path);
                return;
            case "Escape":
                if (!props.onFocusEditor) return;
                handled();
                props.onFocusEditor();
                return;
        }
        if (e.key.length === 1) {
            handled();
            const hit = typeAhead.next(e.key, list, current);
            if (hit) select(hit);
        }
    };
}
