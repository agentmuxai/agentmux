// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Editor's document tabs as drag targets: moving a tab within its strip,
 * and between Editor panes. Kept out of `editor-model.ts`, which is at its
 * size cap; it acts on the pane's store and on the model's per-tab maps.
 * docs/reports/REPORT_DOC_TAB_DRAG_AND_DROP_2026_10_09.md §3.4.
 *
 * A tab's payload (`EditorBuffer`) holds its path, language and scratch
 * identity, not its text. A move between panes therefore carries the tab's
 * live state with it: the text (unsaved edits included), the encoding a save
 * writes back, its editor mode, and its CodeMirror history and selection as
 * JSON. The `EditorState` object itself can't move: its extensions include
 * the source view's update listener, which reports edits to the source
 * pane's model. The target view rebuilds it with its own extensions.
 */

import { historyField } from "@codemirror/commands";
import type { EditorState } from "@codemirror/state";
import { ALREADY_OPEN_THERE, type DocTabHost } from "@/app/doc-tabs/doc-tab-hosts";
import type { DocTab } from "@/app/doc-tabs/doc-tabs";
import { canonicalizePath, dispatch, snapshot, type EditorBuffer } from "@/app/store/editor-pane-state-store";
import type { EditorMode, EditorViewModel } from "./editor-model";

/** Tab `tabId` of Editor `blockId` dragged to just before or after another.
 *  False when nothing moved (dropped where it already was). */
export function moveEditorTabTo(blockId: string, tabId: string, targetId: string, position: "before" | "after"): boolean {
    const before = snapshot(blockId)?.doc;
    dispatch(blockId, { type: "MoveTabTo", tabId, targetId, position, source: "user" });
    return snapshot(blockId)?.doc !== before;
}

/** Saves in flight, counted per tab, per pane. A tab can't move mid-save:
 *  the save's end (its unsaved mark cleared, a scratch buffer moved to its new
 *  path) would land on the pane it left. A second save can start before the
 *  first ends (Save As submitted twice), so the tab stays put until the last
 *  one has. */
const saving = new WeakMap<EditorViewModel, Map<string, number>>();

/** Run `save` for tab `tabId` of `model`, marking the tab as being saved. */
export function trackSave(model: EditorViewModel, tabId: string | null | undefined, save: Promise<void>): Promise<void> {
    if (!tabId) return save;
    let tabs = saving.get(model);
    if (!tabs) saving.set(model, (tabs = new Map()));
    tabs.set(tabId, (tabs.get(tabId) ?? 0) + 1);
    return save.finally(() => {
        const left = (tabs!.get(tabId) ?? 1) - 1;
        if (left > 0) tabs!.set(tabId, left);
        else tabs!.delete(tabId);
    });
}

/** What travels with an Editor tab, beside the tab itself. */
interface EditorLiveState {
    /** A tab's unsaved text. Absent for a clean tab: its text is its file,
     *  which the target reads fresh, so a change on disk the source was still
     *  reading (or never saw) isn't lost. */
    content?: string;
    encoding?: { encoding: string; bom: string; lineEnding: string; hadDecodeErrors: boolean };
    mode?: EditorMode;
    /** `EditorState.toJSON` with its history: undo, redo and the selection. */
    editorState?: unknown;
}

/** Each pane's view: the CodeMirror state it holds for a tab (the live one
 *  for the tab in front, a saved one for the rest). */
const viewStates = new WeakMap<EditorViewModel, (tabId: string) => EditorState | undefined>();
/** Moved-in tabs' CodeMirror states, until the view builds them. */
const movedStates = new WeakMap<EditorViewModel, Map<string, unknown>>();

/** The view tells where its CodeMirror states are. Returns the cleanup. */
export function provideEditorStates(model: EditorViewModel, stateOf: (tabId: string) => EditorState | undefined): () => void {
    viewStates.set(model, stateOf);
    return () => {
        if (viewStates.get(model) === stateOf) viewStates.delete(model);
    };
}

/** The CodeMirror state (as JSON) that moved in with tab `tabId`, once: the
 *  view builds it with `EditorState.fromJSON(json, config, EDITOR_STATE_FIELDS)`. */
export function takeMovedEditorState(model: EditorViewModel, tabId: string): unknown | undefined {
    const moved = movedStates.get(model);
    const json = moved?.get(tabId);
    moved?.delete(tabId);
    return json;
}

/** The fields a moved `EditorState` carries besides its text and selection. */
export const EDITOR_STATE_FIELDS = { history: historyField };

function liveStateOf(model: EditorViewModel, tab: DocTab<EditorBuffer>): EditorLiveState {
    const live: EditorLiveState = {
        encoding: model._encodingByTab.get(tab.id),
        mode: model._tabModes.get(tab.id),
    };
    if (tab.dirty && model._contentByTab.has(tab.id)) live.content = model._contentByTab.get(tab.id);
    // A tab that moved in and moves on before this pane's view has built it
    // still has its state waiting here: pass that on (and don't leave it
    // behind). A built tab's own state is the newer.
    const pending = takeMovedEditorState(model, tab.id);
    try {
        live.editorState = viewStates.get(model)?.(tab.id)?.toJSON(EDITOR_STATE_FIELDS) ?? pending;
    } catch {
        // History that can't be serialized stays behind; the text still moves.
        live.editorState = pending;
    }
    return live;
}

/** An Editor pane's host (doc-tab-hosts.ts): tabs move between Editors on
 *  the same computer, with their unsaved text and undo history. */
export function editorDocTabHost(model: EditorViewModel): DocTabHost {
    const blockId = model.blockId;
    const tabOf = (tabId: string) => snapshot(blockId)?.doc.tabs.find((t) => t.id === tabId);
    return {
        docType: "editor",
        scope: () => model.connection(),
        peek: (tabId) => tabOf(tabId) ?? null,
        refuseGive: (tab) => (saving.get(model)?.has(tab.id) ? "It is still being saved. Try again in a moment." : null),
        refuseTake: (tab) => (snapshot(blockId)?.doc.tabs.some((t) => t.key === tab.key) ? ALREADY_OPEN_THERE : null),
        give: (tabId) => {
            const tab = tabOf(tabId);
            if (!tab) return null;
            const live = liveStateOf(model, tab);
            // TabClosed (from DetachTab) releases the model's and the view's
            // state for it, once the live state above has been taken.
            dispatch(blockId, { type: "DetachTab", tabId, source: "user" });
            return { docType: "editor", tab, live };
        },
        take: (transfer, at) => {
            const live = (transfer.live ?? {}) as EditorLiveState;
            let tab = transfer.tab as DocTab<EditorBuffer>;
            // Arriving without its text (a clean tab, or one not read yet):
            // read here, fresh, rather than show an empty or stale buffer.
            if (tab.payload.contentLoaded && live.content === undefined) {
                tab = { ...tab, payload: { ...tab.payload, contentLoaded: false, contentHash: "" } };
            }
            // In place before the tab arrives, so its first render has them.
            // (Its file isn't open here: refuseTake saw to that.)
            if (live.content !== undefined) model._contentByTab.set(tab.id, live.content);
            if (live.encoding) model._encodingByTab.set(tab.id, live.encoding);
            if (live.mode) {
                model._tabModes.set(tab.id, live.mode);
                model._tabModesVersion[1]((v) => v + 1);
            }
            if (live.editorState !== undefined) {
                let moved = movedStates.get(model);
                if (!moved) movedStates.set(model, (moved = new Map()));
                moved.set(tab.id, live.editorState);
            }
            dispatch(blockId, { type: "AttachTab", tab, at, source: "user" });
            // A read tab watches its file from here now (DetachTab stopped the
            // source's watch); one not read yet is watched when it is.
            if (tab.payload.contentLoaded && live.content !== undefined) {
                model._syncWatch(tab.id, canonicalizePath(tab.payload.filePath));
            }
        },
    };
}
