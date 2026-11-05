// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Opening things from the Files pane: an entry, a folder in a new tab, a permanent delete behind its
 * confirmation, and paths to the clipboard. Split out of FilesView.
 */

import type { FsEntry } from "@/types/rpc/FsEntry";
import { errorText } from "./files-model";
import { openFolderInNewTab, openInEditor, openInPane, openTargetOf, openWithOs } from "./files-open";
import type { FilesLocal } from "./files-view-state";

export type FilesOpening = ReturnType<typeof useFilesOpening>;

export function useFilesOpening(fv: FilesLocal) {
    const { model, setConfirm } = fv;

    const openEntry = (entry: FsEntry): void => {
        const path = model.pathOf(entry.name);
        if (entry.is_dir) {
            void model.navigate(path);
            return;
        }
        const target = openTargetOf(entry.name);
        if (target !== "editor" && model.refuseInRoot("Opening this file")) return;
        const fail = (err: unknown) => model.setStatus({ text: `Couldn't open ${entry.name}: ${errorText(err)}`, tone: "error" });
        // On a host: in an editor on that host; nothing else opens there yet.
        if (model.connection()) {
            if (target === "editor") void openInPane("editor", path, model.blockId, model.connection()).catch(fail);
            else model.notOnHost("Opening programs and other files");
            return;
        }
        if (target === "editor") void openInEditor(model, path).catch(fail);
        else if (target === "os") void openWithOs(path, model.blockId).catch(fail);
        else
            setConfirm({
                title: `Run ${entry.name}?`,
                message: `${entry.name} is a program or shortcut. Opening it runs it.`,
                confirmLabel: "Run",
                onConfirm: () => void openWithOs(path, model.blockId).catch(fail),
            });
    };

    /** A folder in a new Files tab of this pane (Ctrl+Enter, middle-click,
     *  the menu); Ctrl+T opens the folder shown. */
    const openInNewTab = (dir: string): void => {
        if (model.refuseInRoot("A new tab")) return;
        void openFolderInNewTab(model.blockId, dir, model.connection() || undefined).catch((err) =>
            model.setStatus({ text: `Couldn't open a new tab: ${errorText(err)}`, tone: "error" })
        );
    };

    const askDeletePermanently = (list: FsEntry[]): void => {
        if (list.length === 0) return;
        const what = list.length === 1 ? `"${list[0].name}"` : `${list.length} items`;
        const folders = list.some((e) => e.is_dir) ? " Folders are deleted with everything in them." : "";
        setConfirm({
            title: `Delete ${what} permanently?`,
            message: `This can't be undone: it skips the Trash.${folders}`,
            confirmLabel: "Delete permanently",
            onConfirm: () => void model.deletePermanently(list),
        });
    };

    const copyPaths = (list: FsEntry[], quiet = false): void => {
        const text = list.map((e) => model.pathOf(e.name)).join("\n");
        void navigator.clipboard?.writeText(text).then(
            () => {
                if (!quiet) model.setStatus({ text: list.length === 1 ? "Copied the path" : `Copied ${list.length} paths`, tone: "info" }, 2500);
            },
            () => model.setStatus({ text: "Couldn't copy to the clipboard", tone: "error" })
        );
    };

    return { openEntry, openInNewTab, askDeletePermanently, copyPaths };
}
