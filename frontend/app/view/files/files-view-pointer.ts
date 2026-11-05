// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Files pane's pointer and its status line: row clicks, the row and folder menus, the summary and
 * the view toggle. Split out of FilesView.
 */

import type { ContextMenuItem } from "@/app/components/context-menu";
import { formatBytes } from "@/util/format-bytes";
import type { FsEntry } from "@/types/rpc/FsEntry";
import { createMemo } from "solid-js";
import { buildFolderMenu, buildRowMenu } from "./files-menus";
import { clickRow } from "./files-selection";
import { isMod } from "./files-view-keys";
import type { FilesOpening } from "./files-view-opening";
import type { FilesLocal } from "./files-view-state";
import type { FilesWindow } from "./files-view-window";

export function useFilesPointer(fv: FilesLocal, win: FilesWindow, opening: FilesOpening) {
    const { model, refs, setMenu, setScrollTop } = fv;
    const { openEntry, openInNewTab, copyPaths, askDeletePermanently } = opening;
    const { entries, order } = win;

    const onRowClick = (e: MouseEvent, entry: FsEntry): void => {
        refs.list?.focus();
        model.setSelection(clickRow(model.selection(), order(), entry.name, { toggle: isMod(e), range: e.shiftKey }));
    };

    const rowMenu = (entry: FsEntry): ContextMenuItem[] => buildRowMenu(model, entry, { openEntry, openInNewTab, copyPaths, askDeletePermanently });
    const folderMenu = (): ContextMenuItem[] => buildFolderMenu(model, { openInNewTab });

    const onRowContextMenu = (e: MouseEvent, entry: FsEntry): void => {
        e.preventDefault();
        e.stopPropagation();
        if (!model.selection().names.has(entry.name)) {
            model.setSelection({ names: new Set([entry.name]), focus: entry.name, anchor: entry.name });
        }
        setMenu({ x: e.clientX, y: e.clientY, items: rowMenu(entry) });
    };

    const onListContextMenu = (e: MouseEvent): void => {
        e.preventDefault();
        // Like a row: stop here, or the click bubbles to the pane frame and the
        // generic pane menu opens on top of this one.
        e.stopPropagation();
        setMenu({ x: e.clientX, y: e.clientY, items: folderMenu() });
    };

    // ── Status line ────────────────────────────────────────────────────────

    const summary = createMemo(() => {
        const all = entries();
        const sel = model.selectedEntries().filter((e) => model.selection().names.has(e.name));
        const total = model.rawEntries().filter((e) => model.showHidden() || !e.hidden).length;
        const count = model.filter()
            ? `${all.length} of ${total} items match`
            : `${all.length} item${all.length === 1 ? "" : "s"}${model.partial() ? "…" : ""}`;
        if (sel.length === 0) return count;
        const bytes = sel.reduce((n, e) => n + (e.size ?? 0), 0);
        const size = sel.some((e) => !e.is_dir) ? ` · ${formatBytes(bytes)}` : "";
        return `${sel.length} selected${size} · ${count}`;
    });

    const toggleViewMode = (): void => {
        model.toggleViewMode();
        if (refs.list) refs.list.scrollTop = 0;
        setScrollTop(0);
    };

    const placeLabel = (path: string): string => model.protectedPlace(path)?.label ?? path;

    return { onRowClick, onRowContextMenu, onListContextMenu, summary, toggleViewMode, placeLabel };
}
