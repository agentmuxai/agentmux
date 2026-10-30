// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Editor's document tabs on the shared surfaces: `editorDocTabs` is the
 * pane's store as a `DocTabsModel` (the strip, the keys, moves), and
 * `editorDocTabHost` its move host, with what a moved Editor tab carries.
 * Kept out of `editor-model.ts`, which is at its size cap; it acts on the
 * pane's store and on the model's per-tab maps.
 * docs/specs/PLAN_EDITOR_ON_SHARED_DOC_TABS_2026_10_10.md,
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
import { docTabsHost, type DocTabHost, type MergeResult } from "@/app/doc-tabs/doc-tab-hosts";
import { emptyDocTabs, type DocTab } from "@/app/doc-tabs/doc-tabs";
import type { DocTabsModel } from "@/app/doc-tabs/doc-tabs-controller";
import { canonicalizePath, dispatch, snapshot, type EditorBuffer } from "@/app/store/editor-pane-state-store";
import { pushNotification } from "@/app/store/global";
import type { EditorMode, EditorViewModel } from "./editor-model";

/** Tab `tabId` of Editor `blockId` dragged to just before or after another.
 *  False when nothing moved (dropped where it already was). */
function moveEditorTabTo(blockId: string, tabId: string, targetId: string, position: "before" | "after"): boolean {
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

/**
 * The Editor pane's tabs as a `DocTabsModel`: each action is the store's
 * command, so the dirty-close confirmation, its events and the audit ring
 * stay the Editor's.
 */
export function editorDocTabs(model: EditorViewModel): DocTabsModel<EditorBuffer> {
    const blockId = model.blockId;
    const doc = () => {
        model.sliceVersionAtom();
        return snapshot(blockId)?.doc ?? emptyDocTabs<EditorBuffer>();
    };
    const tabOf = (id: string) => doc().tabs.find((t) => t.id === id);
    return {
        tabs: () => doc().tabs,
        activeId: () => doc().activeId,
        showStrip: () => doc().tabs.length > 0,
        activate: (id) => model.switchTab(id),
        // A tab with unsaved changes asks first (`closeTab`): nothing to refuse.
        close: (id) => {
            model.closeTab(id);
            return true;
        },
        promote: (id) => void dispatch(blockId, { type: "PinTab", tabId: id, source: "user" }),
        cycle: (delta) => model.cycleTab(delta),
        move: (id, delta) => {
            const at = doc().tabs.findIndex((t) => t.id === id);
            if (at >= 0) dispatch(blockId, { type: "ReorderTab", tabId: id, toIndex: at + delta, source: "user" });
        },
        moveTo: (id, targetId, position) => moveEditorTabTo(blockId, id, targetId, position),
        reopen: () => {
            const before = snapshot(blockId)?.doc;
            model.reopenLastClosed();
            return snapshot(blockId)?.doc !== before;
        },
        // No scratch buffers in an editor on a host (they are local files).
        canAddNew: () => !model.connection(),
        newDocument: () => {
            void model.openScratch(false);
            return true;
        },
        detach: (id) => {
            const tab = tabOf(id);
            if (!tab) return null;
            // TabClosed (from DetachTab) releases the model's and the view's
            // state for it.
            dispatch(blockId, { type: "DetachTab", tabId: id, source: "user" });
            return tab;
        },
        attach: (tab, at) => void dispatch(blockId, { type: "AttachTab", tab, at, source: "user" }),
    };
}

/** The Editor behind each host, for a merge to reach the source's tabs and state. */
const modelOfHost = new WeakMap<DocTabHost, EditorViewModel>();

/** Why an Editor with a save in flight can't be merged into another yet. */
const SAVING_REASON = "It is still being saved. Try again in a moment.";

/** Say where an unsaved copy went: it opens as an untitled tab, with nothing in it to tell which file it was. */
function noteKeptCopy(title: string): void {
    pushNotification({
        icon: "fa-clone",
        title: "Kept your unsaved copy",
        message: `${title} had different unsaved changes in both Editors: the other copy is in a new untitled tab.`,
        timestamp: new Date().toISOString(),
        type: "info",
        expiration: Date.now() + 8000,
    });
}

/**
 * Take every document tab of Editor `source` into Editor `model`, in their order after the
 * destination's own, with their state (the same carry as dragging one tab). The destination's
 * active document stays active. A document open in both (SPEC_DOCUMENT_PANES_MERGE_DOCUMENT_TABS
 * §3): both clean, one tab; one unsaved, that copy wins and takes the clean tab's place; both
 * unsaved and different, the arriving text becomes an untitled tab next to the destination's;
 * both unsaved and the same, one tab. Nothing is changed, and the reason is given, when a tab of
 * the source is being saved or refuses to leave.
 */
function mergeEditorFrom(model: EditorViewModel, host: DocTabHost, source: DocTabHost): MergeResult {
    const sourceModel = modelOfHost.get(source);
    if (!sourceModel || sourceModel === model) return { merged: false };
    const arriving = [...editorDocTabs(sourceModel).tabs()];
    for (const tab of arriving) {
        const reason = source.refuseGive(tab);
        if (reason) return { merged: false, reason };
    }
    const mine = editorDocTabs(model);
    const front = mine.activeId();
    let last: string | undefined;
    const placeAfterLast = (): { targetId: string; position: "after" } | undefined =>
        last ? { targetId: last, position: "after" } : undefined;
    for (const tab of arriving) {
        const same = mine.tabs().find((t) => t.key === tab.key);
        const transfer = source.give(tab.id);
        if (!transfer) continue;
        if (!same) {
            host.take(transfer, placeAfterLast() ?? lastTabPosition(mine));
            last = tab.id;
            continue;
        }
        const theirs = (transfer.live as EditorLiveState | undefined)?.content;
        const mineDirty = same.dirty === true;
        if (tab.dirty && !mineDirty) {
            // The unsaved copy replaces the clean one, in the clean one's place.
            const tabs = mine.tabs();
            const at = tabs.findIndex((t) => t.id === same.id);
            const next = tabs[at + 1];
            const prev = tabs[at - 1];
            mine.detach(same.id);
            host.take(
                transfer,
                next ? { targetId: next.id, position: "before" } : prev ? { targetId: prev.id, position: "after" } : undefined
            );
        } else if (tab.dirty && mineDirty && theirs !== undefined && theirs !== model._contentByTab.get(same.id)) {
            void model.openDroppedText(tab.title, theirs);
            noteKeptCopy(tab.title);
        }
        // Otherwise the destination's tab stays and the arriving copy is dropped.
    }
    if (front && mine.activeId() !== front && mine.tabs().some((t) => t.id === front)) mine.activate(front);
    return { merged: true };
}

/** After the last tab, or nothing (an empty Editor takes it as its first). */
function lastTabPosition(tabs: DocTabsModel<EditorBuffer>): { targetId: string; position: "after" } | undefined {
    const all = tabs.tabs();
    const last = all[all.length - 1];
    return last ? { targetId: last.id, position: "after" } : undefined;
}

/** An Editor pane's host (doc-tab-hosts.ts): tabs move between Editors on
 *  the same computer, with their unsaved text and undo history. */
export function editorDocTabHost(model: EditorViewModel): DocTabHost {
    const host: DocTabHost = docTabsHost(editorDocTabs(model), "editor", {
        scope: () => model.connection(),
        refuseGive: (tab) => (saving.get(model)?.has(tab.id) ? "It is still being saved. Try again in a moment." : null),
        liveOf: (tab) => liveStateOf(model, tab),
        arriving: (arriving, moved) => {
            const live = (moved ?? {}) as EditorLiveState;
            let tab = arriving;
            // Arriving without its text (a clean tab, or one not read yet):
            // read here, fresh, rather than show an empty or stale buffer.
            if (tab.payload.contentLoaded && live.content === undefined) {
                tab = { ...tab, payload: { ...tab.payload, contentLoaded: false, contentHash: "" } };
            }
            // In place before the tab arrives, so its first render has them.
            // (Its file is not open here: the host refuses that.)
            if (live.content !== undefined) model._contentByTab.set(tab.id, live.content);
            if (live.encoding) model._encodingByTab.set(tab.id, live.encoding);
            if (live.mode) {
                model._tabModes.set(tab.id, live.mode);
                model._tabModesVersion[1]((v) => v + 1);
            }
            if (live.editorState !== undefined) {
                let movedIn = movedStates.get(model);
                if (!movedIn) movedStates.set(model, (movedIn = new Map()));
                movedIn.set(tab.id, live.editorState);
            }
            return tab;
        },
        // A read tab watches its file from here now (DetachTab stopped the
        // source's watch); one not read yet is watched when it is.
        arrived: (tab, moved) => {
            const live = (moved ?? {}) as EditorLiveState;
            if (tab.payload.contentLoaded && live.content !== undefined) {
                model._syncWatch(tab.id, canonicalizePath(tab.payload.filePath));
            }
        },
    });
    const merging: DocTabHost = {
        ...host,
        mergeFrom: (source) => {
            const sourceModel = modelOfHost.get(source);
            if (sourceModel && saving.get(sourceModel)?.size) return { merged: false, reason: SAVING_REASON };
            return mergeEditorFrom(model, merging, source);
        },
        newDocument: () => void model.openScratch(false),
    };
    modelOfHost.set(merging, model);
    return merging;
}
