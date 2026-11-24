// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Opening things from the Files pane: an entry, a folder in a new tab, a permanent delete behind its
 * confirmation, and paths to the clipboard. Split out of FilesView.
 */

import type { FsEntry } from "@/types/rpc/FsEntry";
import { errorText } from "./files-model";
import { askedFirstAgent } from "@/app/util/open-asks-first";
import { openFolderInNewTab, openInEditor, openInPane, openTargetOf, openWithOs, runsWhenOpenedByOs } from "./files-open";
import type { FilesLocal } from "./files-view-state";

export type FilesOpening = ReturnType<typeof useFilesOpening>;

export function useFilesOpening(fv: FilesLocal) {
    const { model, setConfirm } = fv;

    /** A file with the system's default app. A program, a shortcut or a script asks first (`mustAsk`); and a file a
     *  container agent can write asks first too, naming the agent (srv says so, and the answer is sent again as
     *  confirmed). Both are panels in this pane. */
    const openWithDefaultApp = (entry: FsEntry, mustAsk: boolean = runsWhenOpenedByOs(entry.name)): void => {
        if (model.refuseInRoot("Opening with another program")) return;
        const path = model.pathOf(entry.name);
        const fail = (err: unknown) => model.setStatus({ text: `Couldn't open ${entry.name}: ${errorText(err)}`, tone: "error" });
        const go = (confirmed: boolean): void => {
            void openWithOs(path, model.blockId, confirmed).catch((err) => {
                const agent = askedFirstAgent(err);
                if (agent === null) return fail(err);
                setConfirm({
                    title: `Open ${entry.name}?`,
                    message: agent
                        ? `${entry.name} is in the folder of ${agent}, a container agent that can change it. Opening it runs your computer's app on what ${agent} wrote.`
                        : `${entry.name} is in a folder a container agent can change. Opening it runs your computer's app on what the agent wrote.`,
                    confirmLabel: "Open",
                    onConfirm: () => go(true),
                });
            });
        };
        if (!mustAsk) return go(false);
        setConfirm({
            title: `Run ${entry.name}?`,
            message: `${entry.name} is a program or shortcut. Opening it runs it.`,
            confirmLabel: "Run",
            onConfirm: () => go(false),
        });
    };

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
        else openWithDefaultApp(entry, target === "program");
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

    return { openEntry, openWithDefaultApp, openInNewTab, askDeletePermanently, copyPaths };
}
