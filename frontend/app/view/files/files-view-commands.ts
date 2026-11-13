// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Files pane's `files:*` commands, for its keys and RunCommand alike. Split out of FilesView.
 */

import { registerPaneCommandRunner } from "@/app/keybindings";
import { onCleanup } from "solid-js";
import { closeOwnTab } from "./files-open";
import { errorText } from "./files-model";
import { selectAll } from "./files-selection";
import type { FilesOpening } from "./files-view-opening";
import type { FilesLocal } from "./files-view-state";
import type { FilesWindow } from "./files-view-window";

export function useFilesCommands(fv: FilesLocal, win: FilesWindow, opening: FilesOpening): (command: string) => boolean {
    const { model } = fv;
    const { openEntry, openInNewTab, askDeletePermanently, copyPaths } = opening;

    /** Runs a `files:*` command, for its keys and RunCommand alike; false if unknown. */
    const runFilesCommand = (command: string): boolean => {
        const sel = model.selection();
        switch (command) {
            case "files:back":
                model.goBack();
                return true;
            case "files:forward":
                model.goForward();
                return true;
            case "files:up":
                model.goUp();
                return true;
            case "files:openInNewTab":
                // Folders in new tabs; files open as usual.
                for (const entry of model.selectedEntries()) {
                    if (entry.is_dir) openInNewTab(model.pathOf(entry.name));
                    else openEntry(entry);
                }
                return true;
            case "files:newTabHere":
                openInNewTab(model.path());
                return true;
            case "files:closeTab":
                void closeOwnTab(model.blockId).then(
                    (closed) => {
                        if (!closed) model.setStatus({ text: "This is the pane's only tab. Use the pane's × to close it.", tone: "info" }, 3000);
                    },
                    (err) => model.setStatus({ text: `Couldn't close this tab: ${errorText(err)}`, tone: "error" })
                );
                return true;
            case "files:rename":
                if (sel.focus && sel.names.size === 1 && sel.names.has(sel.focus)) model.setRenaming(sel.focus);
                return true;
            case "files:refresh":
                model.refresh();
                return true;
            case "files:trash":
                void model.trash(model.selectedEntries());
                return true;
            case "files:deletePermanently":
                askDeletePermanently(model.selectedEntries());
                return true;
            case "files:selectAll":
                model.setSelection(selectAll(sel, win.order()));
                return true;
            case "files:undo":
                void model.undo();
                return true;
            case "files:copy":
                // Both: the files for a paste in the Files pane, the paths for anywhere else.
                model.copyToClipboard("copy", model.selectedEntries());
                copyPaths(model.selectedEntries(), true);
                return true;
            case "files:cut":
                model.copyToClipboard("cut", model.selectedEntries());
                return true;
            case "files:paste":
                void model.paste();
                return true;
            case "files:editPath":
                fv.startEditingPath();
                return true;
            case "files:newFolder":
                void model.createNew("dir");
                return true;
            case "files:filter":
                fv.toggleFilter();
                return true;
        }
        return false;
    };
    onCleanup(registerPaneCommandRunner(model.blockId, runFilesCommand));
    return runFilesCommand;
}
