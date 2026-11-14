// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Renaming a file from its document tab (docs/specs/SPEC_EDITOR_TAB_RENAME_2026_11_14.md).
 *
 * Two halves. The disk half renames through `fs.rename`, the Files pane's
 * route (it never overwrites, handles a change of case only, and works on an
 * SSH host), and moves this pane's tabs to the new path; every other Editor
 * follows by the `fs:renamed` event srv sends for any rename. The tab half is
 * the state of the name box in the strip: which tab it is open on, the line
 * under it, and what committing does. A saved file is renamed in its folder;
 * an Untitled document is saved under the typed name, by the Save As path.
 *
 * Nothing touches the disk before Enter: leaving the box cancels.
 */

import { createSignal, onCleanup, type Accessor } from "solid-js";
import { dispatch, snapshot, type EditorBuffer, type EditorTab } from "@/app/store/editor-pane-state-store";
import type { DocTab } from "@/app/doc-tabs/doc-tabs";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import type { FileRenamedEvent } from "@/types/rpc/FileRenamedEvent";
import { isSaving, trackSave } from "./editor-doc-tabs";
import { detectLanguage } from "./editor-language";
import type { EditorViewModel } from "./editor-model";
import { baseName, dirName, errorSentence, pathAfterRename, samePath, typedNameProblem, untitledDestination } from "./editor-rename-names";
import { isDroppedPath } from "./viewers/viewer-tabs";

/** A save on a host waits as long as its saves do. */
const HOST_TIMEOUT_MS = 180_000;

/** A tab on a real file: not an Untitled buffer, not a dropped item with no path. */
const hasFile = (b: Pick<EditorBuffer, "filePath" | "isScratch">): boolean => !b.isScratch && !isDroppedPath(b.filePath);

/**
 * Move this pane's tabs from `oldPath` to `newPath`: the tab on that file,
 * and the tabs inside it when it is a folder. Each keeps its id, its text and
 * its unsaved mark; its language follows a changed extension, and its file
 * watch moves with it.
 */
function applyRename(model: EditorViewModel, oldPath: string, newPath: string): void {
    const doc = snapshot(model.blockId)?.doc;
    if (!doc) return;
    // Closed tabs too: reopening one must find its file.
    const paths = new Set([...doc.tabs, ...doc.closed].filter((t) => hasFile(t.payload)).map((t) => t.payload.filePath));
    for (const path of paths) {
        const after = pathAfterRename(path, oldPath, newPath);
        if (after === null || after === path) continue;
        const language = detectLanguage(after);
        dispatch(model.blockId, {
            type: "RenameFile",
            oldPath: path,
            newPath: after,
            ...(language !== detectLanguage(path) ? { language } : {}),
            source: "system",
        });
        model._resyncWatchAfterRename(path, after);
    }
}

/**
 * Rename `path` to `newName` in its folder, on this computer or on the pane's
 * host. Null when done, with this pane's tabs on the new path; else srv's
 * sentence for why not, with nothing changed.
 */
export async function renameOnDisk(model: EditorViewModel, path: string, newName: string): Promise<string | null> {
    const connection = model.connection();
    try {
        const result = await RpcApi.FsRenameCommand(
            TabRpcClient,
            { path, new_name: newName, block_id: model.blockId, ...(connection ? { connection } : {}) },
            connection ? { timeout: HOST_TIMEOUT_MS } : undefined
        );
        applyRename(model, path, result.new_path);
        // The tree spells paths its own way (the system's separators), which a tab's path may not
        // match: it lists its open folders again rather than one folder by name.
        void model.treeModel.refresh();
        return null;
    } catch (e: unknown) {
        return errorSentence(e);
    }
}

/** A rename done anywhere (another pane's tab or tree, the Files pane, another window): this pane's tabs follow. */
export function followRename(model: EditorViewModel, renamed: FileRenamedEvent): void {
    if ((renamed.connection ?? "") !== model.connection()) return;
    applyRename(model, renamed.old_path, renamed.new_path);
}

/** Where the last Untitled document was saved, for the next one in a pane that has shown no file. */
let lastSaveFolder: string | null = null;

/** Note the folder a document was just saved into (Save As, or a name typed into an Untitled tab). */
export function noteSavedTo(path: string): void {
    const folder = dirName(path);
    if (folder) lastSaveFolder = folder;
}

/** For tests. */
export function forgetSaveFolder(): void {
    lastSaveFolder = null;
}

/**
 * The folder a name typed into an Untitled tab goes to: the folder of the
 * file this pane showed last, else the folder the last save went to, else
 * the home folder.
 */
export function untitledFolder(model: EditorViewModel): string {
    const doc = snapshot(model.blockId)?.doc;
    if (doc) {
        const onFile = (t: DocTab<EditorBuffer> | undefined): t is DocTab<EditorBuffer> => !!t && hasFile(t.payload);
        const recent = doc.mru.map((id) => doc.tabs.find((t) => t.id === id)).find(onFile) ?? doc.closed.find(onFile);
        if (recent) return dirName(recent.payload.filePath) || "~";
    }
    return lastSaveFolder ?? "~";
}

/** The line under the name box. */
export interface RenameLine {
    kind: "hint" | "busy" | "error";
    text: string;
}

/** The name box of a document tab: its state, and what it does. */
export interface TabRename {
    /** The tab whose name is being edited, if any. */
    tabId: Accessor<string | null>;
    /** Whether a commit is on its way: the box takes no Enter, Escape or blur until the answer. */
    busy: Accessor<boolean>;
    /** What is under the box: where an Untitled document will go, that it is being renamed, or why it wasn't. */
    line: Accessor<RenameLine | null>;
    /** Why `tab` has no name to edit, or null. */
    refusal: (tab: Pick<EditorTab, "filePath" | "isScratch">) => string | null;
    /** Open the box on `tabId` (default: the tab in front). False when there is nothing to rename. */
    start: (tabId?: string) => boolean;
    /** A double-click on tab `tabId`: `start`, unless it is the tail of a double-click that closed the tab before it. */
    startFromDoubleClick: (tabId: string) => boolean;
    /** The name the box starts with. */
    initialName: () => string;
    /** The box's text changed. */
    setTyped: (text: string) => void;
    /** Close the box, changing nothing. `returnFocus`: the keyboard goes back to the document (Escape, not a click elsewhere). */
    cancel: (returnFocus?: boolean) => void;
    /** Enter: rename, or save an Untitled document. The box closes when done and stays open, with the reason under it, when refused. */
    commit: (typed: string) => Promise<void>;
}

/** A tab's name as its strip shows it: an Untitled buffer's own, else the file's. */
const titleOf = (tab: Pick<EditorTab, "filePath" | "displayName">): string => tab.displayName || baseName(tab.filePath);

/**
 * A double-click on a tab's close button closes it with the first click, and the second lands on
 * the tab that slid under the pointer: for this long after a tab closes, a double-click renames nothing.
 */
const AFTER_CLOSE_MS = 500;

const STILL_SAVING = "It is still being saved. Try again in a moment.";
const NO_FILE = "This document has no file on disk.";

/** Enter on a saved file's tab: null when renamed, else why not. */
async function renameTabFile(model: EditorViewModel, tab: EditorTab, name: string): Promise<string | null> {
    const problem = typedNameProblem(name);
    if (problem) return problem;
    if (isSaving(model, tab.id)) return STILL_SAVING;
    const folder = dirName(tab.filePath);
    const target = (/[\\/]$/.test(folder) ? folder : folder + "/") + name;
    const taken = snapshot(model.blockId)?.tabs.some((t) => t.id !== tab.id && samePath(t.filePath, target));
    if (taken) return `“${name}” is already open in a tab here. Close that tab first.`;
    return renameOnDisk(model, tab.filePath, name);
}

/** Enter on an Untitled tab: null when saved under the typed name, else why not. */
async function saveUntitledAs(model: EditorViewModel, tab: EditorTab, text: string): Promise<string | null> {
    const dest = untitledDestination(text, untitledFolder(model));
    if ("problem" in dest) return dest.problem;
    if (isSaving(model, tab.id)) return STILL_SAVING;
    const problem = await trackSaveResult(model, tab.id, dest.path);
    if (problem === null) noteSavedTo(snapshot(model.blockId)?.tabs.find((t) => t.id === tab.id)?.filePath ?? dest.path);
    return problem;
}

export function createTabRename(model: EditorViewModel, opts: { focusDocument: () => void; now?: () => number }): TabRename {
    const now = opts.now ?? (() => performance.now());
    let closedAt = Number.NEGATIVE_INFINITY;
    onCleanup(
        model.onSliceEvent((event) => {
            if (event.type === "TabClosed") closedAt = now();
        })
    );
    const [tabId, setTabId] = createSignal<string | null>(null);
    const [typed, setTyped] = createSignal("");
    const [error, setError] = createSignal<string | null>(null);
    const [busy, setBusy] = createSignal(false);

    const tabOf = (id: string | null): EditorTab | undefined => snapshot(model.blockId)?.tabs.find((t) => t.id === id);
    const refusal = (tab: Pick<EditorTab, "filePath" | "isScratch">): string | null =>
        !tab.isScratch && isDroppedPath(tab.filePath) ? NO_FILE : null;

    const close = (returnFocus: boolean) => {
        setTabId(null);
        setError(null);
        setBusy(false);
        if (returnFocus) opts.focusDocument();
    };

    const start = (id?: string): boolean => {
        const tab = tabOf(id ?? model.activeIdAtom());
        if (!tab || refusal(tab) || busy()) return false;
        // The box is the tab in front's: a click on a tab brings it forward anyway, the menu may not have.
        if (model.activeIdAtom() !== tab.id) model.switchTab(tab.id);
        // Naming a document is a decision to keep it open.
        if (tab.isPreview) dispatch(model.blockId, { type: "PinTab", tabId: tab.id, source: "user" });
        setError(null);
        setTyped(titleOf(tab));
        setTabId(tab.id);
        return true;
    };

    const line = (): RenameLine | null => {
        const tab = tabOf(tabId());
        if (!tab) return null;
        const problem = error();
        if (problem) return { kind: "error", text: problem };
        if (busy()) return { kind: "busy", text: tab.isScratch ? "Saving…" : "Renaming…" };
        if (!tab.isScratch) return null;
        const dest = untitledDestination(typed(), untitledFolder(model));
        return "path" in dest ? { kind: "hint", text: `Saves to ${dest.path}` } : null;
    };

    const commit = async (text: string): Promise<void> => {
        const tab = tabOf(tabId());
        if (!tab || busy()) return;
        const name = text.trim();
        if (!name || name === titleOf(tab)) return close(true);
        setError(null);
        setBusy(true);
        const problem = tab.isScratch ? await saveUntitledAs(model, tab, name) : await renameTabFile(model, tab, name);
        setBusy(false);
        // The tab may have left the pane meanwhile: its box went with it.
        if (tabId() !== tab.id) return;
        if (problem === null) close(true);
        else setError(problem);
    };

    return {
        tabId,
        busy,
        line,
        refusal,
        start,
        startFromDoubleClick: (id) => now() - closedAt >= AFTER_CLOSE_MS && start(id),
        initialName: () => {
            const tab = tabOf(tabId());
            return tab ? titleOf(tab) : "";
        },
        setTyped: (text) => {
            setTyped(text);
            setError(null);
        },
        cancel: (returnFocus = false) => {
            if (!busy()) close(returnFocus);
        },
        commit,
    };
}

/** Save the Untitled tab `tabId` to `path`, marked as a save in flight (so the tab can't move meanwhile). */
async function trackSaveResult(model: EditorViewModel, tabId: string, path: string): Promise<string | null> {
    let problem: string | null = null;
    await trackSave(
        model,
        tabId,
        model.saveFileAs(path, false).then((p) => {
            problem = p;
        })
    );
    return problem;
}
