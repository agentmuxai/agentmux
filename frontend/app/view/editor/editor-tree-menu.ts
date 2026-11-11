// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { ContextMenuItem } from "@/app/components/context-menu";
import { revealInFileManagerLabel } from "@/app/util/file-manager-label";
import { MENU_ICON } from "@/app/util/menu-icons";
import type { EditorViewModel } from "./editor-model";

/** The question asked before something is thrown away. */
export interface ConfirmRequest {
    title: string;
    message: string;
    confirmLabel?: string;
    onConfirm: () => void;
}

/** What a tree menu entry asks the editor view to show; the model does the rest. */
export interface TreeMenuActions {
    startNewEntry(parentPath: string, kind: "file" | "dir"): void;
    startRename(path: string): void;
    confirm(request: ConfirmRequest): void;
    copy(text: string): void;
}

type TreeMenuModel = Pick<
    EditorViewModel,
    "treeModel" | "openInTerminal" | "revealInExplorer" | "deleteFile" | "openFile" | "openToTheSide" | "openInNewTab"
>;

/** The right-click menu of the file tree: its background (`path` null), a folder, or a file. */
export function buildTreeContextMenuItems(
    model: TreeMenuModel,
    actions: TreeMenuActions,
    path: string | null,
    isDir: boolean,
): ContextMenuItem[] {
    if (!path) {
        // Background right-click → tree-level actions.
        const homeRoot = model.treeModel.rootsAtom().find((r) => r.isHome)?.path;
        if (!homeRoot) {
            return [{ type: "action", label: "Refresh", icon: MENU_ICON.refresh, onSelect: () => void model.treeModel.refresh() }];
        }
        return [
            { type: "action", label: "New File…", icon: MENU_ICON.newFile, onSelect: () => { void model.treeModel.expandFolder(homeRoot); actions.startNewEntry(homeRoot, "file"); } },
            { type: "action", label: "New Folder…", icon: MENU_ICON.newFolder, onSelect: () => { void model.treeModel.expandFolder(homeRoot); actions.startNewEntry(homeRoot, "dir"); } },
            { type: "separator" },
            { type: "action", label: "Refresh", icon: MENU_ICON.refresh, onSelect: () => void model.treeModel.refresh() },
        ];
    }
    const name = path.split(/[/\\]/).pop() ?? path;
    if (isDir) {
        const isRoot = model.treeModel.rootsAtom().some((r) => r.path === path);
        const items: ContextMenuItem[] = [
            { type: "action", label: "New File…", icon: MENU_ICON.newFile, onSelect: () => { void model.treeModel.expandFolder(path); actions.startNewEntry(path, "file"); } },
            { type: "action", label: "New Folder…", icon: MENU_ICON.newFolder, onSelect: () => { void model.treeModel.expandFolder(path); actions.startNewEntry(path, "dir"); } },
            { type: "separator" },
            { type: "action", label: "Open in Terminal", icon: MENU_ICON.openTerminal, onSelect: () => void model.openInTerminal(path) },
            { type: "action", label: revealInFileManagerLabel(), icon: MENU_ICON.reveal, onSelect: () => void model.revealInExplorer(path) },
            { type: "action", label: "Collapse Folder", onSelect: () => model.treeModel.collapseFolder(path) },
        ];
        if (!isRoot) {
            items.push({ type: "separator" });
            items.push({ type: "action", label: "Rename…", icon: MENU_ICON.rename, shortcut: "F2", onSelect: () => actions.startRename(path) });
            items.push({
                type: "action",
                label: "Delete",
                icon: MENU_ICON.delete,
                danger: true,
                onSelect: () => void model.deleteFile(path, true, (proceed) => {
                    actions.confirm({
                        title: `Delete folder "${name}"?`,
                        message: `This will permanently delete "${name}" and all its contents. This cannot be undone.`,
                        onConfirm: proceed,
                    });
                }),
            });
        }
        return items;
    }
    return [
        { type: "action", label: "Open", icon: MENU_ICON.open, onSelect: () => void model.openFile(path) },
        { type: "action", label: "Open to the Side", icon: MENU_ICON.open, onSelect: () => void model.openToTheSide(path) },
        { type: "action", label: "Open in New Tab", icon: MENU_ICON.open, onSelect: () => void model.openInNewTab(path) },
        { type: "separator" },
        { type: "action", label: "Copy Path", icon: MENU_ICON.copy, onSelect: () => void actions.copy(path) },
        { type: "action", label: "Copy Relative Path", icon: MENU_ICON.copy, onSelect: () => {
            const root = model.treeModel.rootsAtom().find((r) => path === r.path || path.startsWith(r.path + "/") || path.startsWith(r.path + "\\"));
            const rel = root ? path.slice(root.path.length).replace(/^[/\\]/, "") : path;
            void actions.copy(rel);
        }},
        { type: "separator" },
        { type: "action", label: revealInFileManagerLabel(), icon: MENU_ICON.reveal, onSelect: () => void model.revealInExplorer(path) },
        { type: "separator" },
        { type: "action", label: "Rename…", icon: MENU_ICON.rename, shortcut: "F2", onSelect: () => actions.startRename(path) },
        {
            type: "action",
            label: "Delete",
            icon: MENU_ICON.delete,
            danger: true,
            onSelect: () => void model.deleteFile(path, false, (proceed) => {
                actions.confirm({
                    title: `Delete "${name}"?`,
                    message: `This will permanently delete "${name}". This cannot be undone.`,
                    onConfirm: proceed,
                });
            }),
        },
    ];
}
