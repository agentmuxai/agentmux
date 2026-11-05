// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Files pane's keyboard (spec §5.3.1): command keys from anywhere in the pane, and the list's own
 * keys. Split out of FilesView.
 */

import { isGlobalKey, paneCommandFor } from "@/app/keybindings";
import { isEditableTarget } from "@/util/focusutil";
import { isMacOS } from "@/util/platformutil";
import { ROW_HEIGHT, TILE_H } from "./files-rows";
import { moveFocus, toggleFocused } from "./files-selection";
import type { FilesOpening } from "./files-view-opening";
import type { FilesLocal } from "./files-view-state";
import type { FilesWindow } from "./files-view-window";

export type FilesKeys = ReturnType<typeof useFilesKeys>;

export const isMod = (e: KeyboardEvent | MouseEvent): boolean => (isMacOS() ? e.metaKey : e.ctrlKey);

export function useFilesKeys(fv: FilesLocal, win: FilesWindow, opening: FilesOpening, runFilesCommand: (command: string) => boolean) {
    const { model, refs, editingPath, typeahead } = fv;
    const { order, grid, gridCols, reveal } = win;

    // Files commands from anywhere in the pane (toolbar, sidebar), not only
    // the list: the global shortcuts that share these keys stand aside in
    // this pane. Text fields (filter, rename, path) keep their own keys, and
    // the list handles its own focus first.
    const onPaneKeyDown = (e: KeyboardEvent): void => {
        if (e.defaultPrevented || model.renaming() || editingPath() || isEditableTarget(e.target)) return;
        if (refs.list?.contains(e.target as Node)) return;
        const command = paneCommandFor(e, "files");
        if (command && runFilesCommand(command)) {
            e.preventDefault();
            e.stopPropagation();
        }
    };

    const onKeyDown = (e: KeyboardEvent): void => {
        if (model.renaming() || editingPath()) return;
        const sel = model.selection();
        // In the grid, up and down move a row of tiles, left and right one.
        const step = grid() ? gridCols() : 1;
        const page = grid()
            ? Math.max(1, Math.floor((refs.list?.clientHeight ?? 400) / TILE_H) - 1) * gridCols()
            : Math.max(1, Math.floor((refs.list?.clientHeight ?? 400) / ROW_HEIGHT) - 2);
        const move = (m: { delta?: number; to?: number }) => {
            model.setSelection(moveFocus(sel, order(), m, { extend: e.shiftKey, focusOnly: isMod(e) && !e.shiftKey }));
            reveal(order().indexOf(model.selection().focus ?? ""));
        };
        const listKey = (): boolean => {
            if (e.key === "ArrowDown") move({ delta: step });
            else if (e.key === "ArrowUp") move({ delta: -step });
            else if (grid() && e.key === "ArrowRight" && !e.altKey) move({ delta: 1 });
            else if (grid() && e.key === "ArrowLeft" && !e.altKey) move({ delta: -1 });
            else if (e.key === "PageDown") move({ delta: page });
            else if (e.key === "PageUp") move({ delta: -page });
            else if (e.key === "Home") move({ to: 0 });
            else if (e.key === "End") move({ to: order().length - 1 });
            else if (e.key === "Enter") {
                const list = model.selectedEntries();
                if (list.length === 1) opening.openEntry(list[0]);
                else list.filter((x) => !x.is_dir).forEach(opening.openEntry);
            } else if (e.key === "Backspace") model.goBack();
            else if (e.key === "/" && !typeahead.active()) fv.openFilter();
            else if (e.key === "Escape" && model.filter()) fv.closeFilter();
            // Ctrl+Space toggles the focused row (as in Explorer); Space alone is
            // quick look, the preview panel (spec §5.3.1).
            else if (e.key === " " && isMod(e)) model.setSelection(toggleFocused(sel));
            else if (e.key === " " && !typeahead.active()) model.togglePreview();
            else if (e.key.length === 1 && !e.ctrlKey && !e.metaKey && !e.altKey) {
                const hit = typeahead.next(e.key, order(), sel.focus);
                if (hit) {
                    model.setSelection({ names: new Set([hit]), focus: hit, anchor: hit });
                    reveal(order().indexOf(hit));
                }
            } else return false;
            return true;
        };
        let handled = true;
        // Command keys are the `files:*` rows of the shortcut table, so they show in
        // the help pane. The list's own keys (arrows, paging, Enter, type-ahead, Esc)
        // stay here; with a modifier besides Shift, a global command wins (⌃⇧↑, ⌘/).
        const command = paneCommandFor(e, "files");
        handled = command ? runFilesCommand(command) : !((e.ctrlKey || e.metaKey || e.altKey) && isGlobalKey(e)) && listKey();
        if (handled) {
            e.preventDefault();
            e.stopPropagation();
        }
    };

    return { onPaneKeyDown, onKeyDown };
}
