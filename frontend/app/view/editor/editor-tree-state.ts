// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createSignal, onCleanup } from "solid-js";
import type { ContextMenuItem } from "@/app/components/context-menu";
import { writeText as clipboardWriteText } from "@/util/clipboard";
import type { EditorViewModel } from "./editor-model";
import { buildTreeContextMenuItems, type ConfirmRequest } from "./editor-tree-menu";
import { lspBannerShows } from "./editor-lsp-status";
import type { LspState } from "./lsp/lsp-client";

export function copyToClipboard(text: string): void {
    // Route through the CEF clipboard wrapper — navigator.clipboard is
    // blocked under CEF's Permissions-Policy. See SPEC_UNIFIED_CLIPBOARD_2026_05_18.md §3.3.
    void clipboardWriteText(text).catch(() => {
        // Clipboard might not be available — silently ignore
    });
}

/** The file tree's context menu, its in-place rename and new-entry boxes, and the confirmation dialog. */
export function useTreeMenu(model: EditorViewModel) {
    const [ctxMenu, setCtxMenu] = createSignal<{ items: ContextMenuItem[]; x: number; y: number } | null>(null);
    const [renamingPath, setRenamingPath] = createSignal<string | null>(null);
    const [newEntry, setNewEntry] = createSignal<{ parentPath: string; kind: "file" | "dir" } | null>(null);
    const [deleteConfirm, setDeleteConfirm] = createSignal<ConfirmRequest | null>(null);

    // Closing a tab with unsaved changes asks first.
    model.confirmDirtyClose = (tab, discard) =>
        setDeleteConfirm({
            title: `Close "${tab.displayName || tab.filePath.split(/[/\\]/).pop()}" without saving?`,
            message: "Your changes to this file will be lost.",
            confirmLabel: "Discard changes",
            onConfirm: discard,
        });
    onCleanup(() => {
        model.confirmDirtyClose = null;
    });

    const buildContextMenuItems = (path: string | null, isDir: boolean): ContextMenuItem[] =>
        buildTreeContextMenuItems(
            model,
            { startNewEntry: (parentPath, kind) => setNewEntry({ parentPath, kind }), startRename: setRenamingPath, confirm: setDeleteConfirm, copy: copyToClipboard },
            path,
            isDir,
        );

    const handleTreeContextMenu = (path: string | null, isDir: boolean, e: MouseEvent) => {
        setCtxMenu({ items: buildContextMenuItems(path, isDir), x: e.clientX, y: e.clientY });
    };

    const handleRenameConfirm = async (path: string, newName: string) => {
        setRenamingPath(null);
        await model.renameFile(path, newName);
    };

    const handleNewEntryConfirm = async (parentPath: string, name: string, kind: "file" | "dir") => {
        setNewEntry(null);
        try {
            if (kind === "file") await model.createFile(parentPath, name);
            else await model.createDir(parentPath, name);
        } catch {
            // Error already logged in model; tree will re-sync on next refresh.
        }
    };
    return {
        ctxMenu, setCtxMenu, renamingPath, setRenamingPath, newEntry, setNewEntry, deleteConfirm, setDeleteConfirm,
        handleTreeContextMenu, handleRenameConfirm, handleNewEntryConfirm,
    };
}

export type TreeMenu = ReturnType<typeof useTreeMenu>;

/**
 * The language server's install banner is dismissed for the session per language; it returns on
 * the next launch if the binary is still missing.
 */
export function useLspBanner(lspState: () => LspState | null) {
    const [dismissedLanguages, setDismissedLanguages] = createSignal<Set<string>>(new Set());
    const lspBannerVisible = (): boolean => lspBannerShows(lspState(), dismissedLanguages());
    const dismissBanner = (language: string) => {
        const next = new Set(dismissedLanguages());
        next.add(language);
        setDismissedLanguages(next);
    };
    return { lspBannerVisible, dismissBanner };
}
