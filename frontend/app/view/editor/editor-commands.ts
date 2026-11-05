// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createSignal, onCleanup, onMount, type Accessor } from "solid-js";
import { openSearchPanel } from "@codemirror/search";
import { docTabAction, docTabKeyAction, runDocTabAction } from "@/app/doc-tabs/doc-tabs-controller";
import type { DocTabsModel } from "@/app/doc-tabs/doc-tabs-controller";
import { giveBlockFocus } from "@/app/store/focusManager";
import { paneCommandFor } from "@/app/keybindings";
import { registerPaneCommandRunner } from "@/app/keybindings/app-api";
import { bodyFocusTarget } from "./editor-body-focus";
import type { CmHandle } from "./editor-cm-handle";
import type { EditorBuffer } from "@/app/store/editor-pane-state-store";
import { trackSave } from "./editor-doc-tabs";
import type { EditorViewModel } from "./editor-model";

type Root = () => HTMLDivElement | undefined;

/** What the commands and key handlers need to know about what the pane shows. */
export interface EditorShows {
    showsText: () => boolean;
    isMarkdown: () => boolean;
}

/** The Save As flow of a scratch tab: its name box in the tab bar, and what confirming does. */
export function createSaveAs(model: EditorViewModel) {
    const [saveAsTabId, setSaveAsTabId] = createSignal<string | null>(null);

    const triggerSaveAs = () => {
        const tab = model.activeTabAtom();
        if (tab?.isScratch) setSaveAsTabId(tab.id);
    };

    const handleSaveAsConfirm = async (path: string) => {
        const tabId = saveAsTabId();
        setSaveAsTabId(null);
        if (path) await trackSave(model, tabId, model.saveFileAs(path));
    };
    return { saveAsTabId, setSaveAsTabId, triggerSaveAs, handleSaveAsConfirm };
}

/**
 * Which of the tree and the editor body has the keyboard: its column
 * gets an accent line along the top (spec §6).
 */
export function useFocusRegion(root: Root) {
    const [focusRegion, setFocusRegion] = createSignal<"tree" | "body" | null>(null);
    const trackFocusRegion = (e: FocusEvent) => {
        const target = e.target as Element;
        setFocusRegion(target.closest(".editor-tree-column") ? "tree" : target.closest(".editor-main-column") ? "body" : null);
    };
    const leaveFocusRegion = (e: FocusEvent) => {
        if (!root()?.contains(e.relatedTarget as Node | null)) setFocusRegion(null);
    };
    return { focusRegion, trackFocusRegion, leaveFocusRegion };
}

/** The keyboard to the editor body: the text, else the viewer showing the file. */
export function focusEditorBody(model: EditorViewModel, cm: CmHandle, root: Root, shows: EditorShows): void {
    const textShown = shows.showsText() && !(shows.isMarkdown() && model.editorMode() === "preview");
    if (cm.view && textShown) {
        cm.view.focus();
        return;
    }
    const el = root();
    const body = el ? bodyFocusTarget(el) : null;
    if (body) body.focus({ preventScroll: true });
    else giveBlockFocus(model.blockId);
}

interface CommandDeps extends EditorShows {
    model: EditorViewModel;
    cm: CmHandle;
    root: Root;
    docTabs: DocTabsModel<EditorBuffer>;
    focusRegion: Accessor<"tree" | "body" | null>;
    triggerSaveAs: () => void;
}

/**
 * The editor's own commands, run by its keys and by MuxAPI's RunCommand alike. Each returns
 * false when the command doesn't apply right now. Registered as the pane's command runner.
 */
export function useEditorCommands(d: CommandDeps): (command: string) => boolean {
    const { model, cm, root, docTabs } = d;
    const runEditorCommand = (command: string): boolean => {
        const tabAction = docTabAction(command);
        if (tabAction) {
            runDocTabAction(tabAction, docTabs);
            return true;
        }
        switch (command) {
            // Find → CodeMirror's native find panel (find/replace, regex, case, whole-word).
            // Only in source view: in rendered markdown there's nothing editable to search.
            // See docs/specs/SPEC_EDITOR_AND_APP_FIND_2026_06_17.md.
            case "editor:find":
                if (!cm.view || model.editorMode() === "preview") return false;
                openSearchPanel(cm.view);
                return true;
            // Toggle between preview and source modes.
            case "editor:togglePreview":
                if (!d.isMarkdown()) return false;
                model.toggleEditorMode();
                return true;
            case "editor:save":
                if (model.activeTabAtom()?.isScratch) {
                    d.triggerSaveAs();
                } else {
                    void trackSave(model, model.activeIdAtom(), model.saveFile());
                }
                return true;
            // Phase 1: Save As is only implemented for scratch tabs.
            case "editor:saveAs":
                if (!model.activeTabAtom()?.isScratch) return false;
                d.triggerSaveAs();
                return true;
            // From the editor body to the tree, showing it if it's hidden, and back. A remote editor has no tree.
            case "editor:focusTree":
                if (model.connection()) return false;
                if (d.focusRegion() === "tree") {
                    focusEditorBody(model, cm, root, d);
                } else {
                    if (!model.treeExpandedAtom()) void model.toggleTreeExpanded();
                    root()?.querySelector<HTMLElement>('.file-tree [role="treeitem"][tabindex="0"]')?.focus();
                }
                return true;
        }
        return false;
    };
    onCleanup(registerPaneCommandRunner(model.blockId, runEditorCommand));
    return runEditorCommand;
}

/**
 * Editor-pane key handling, capture phase. Intercepted here so the app-wide Cmd/Ctrl+F binding
 * (keymodel.ts → universal block-search, which the editor doesn't implement) never swallows
 * these, and a single deterministic path handles them whatever CodeMirror's focus state.
 */
export function useEditorKeys(root: Root, docTabs: DocTabsModel<EditorBuffer>, runEditorCommand: (command: string) => boolean): void {
    onMount(() => {
        const el = root();
        if (!el) return;

        const handleEditorKeys = (ev: KeyboardEvent) => {
            const mod = ev.ctrlKey || ev.metaKey;
            if (!mod) return;
            // Document-tab keys (SPEC_DOCUMENT_TABS §4.3), before CodeMirror
            // sees them. A text box (Save As, a tree rename) keeps its own.
            const tabKey = docTabKeyAction(ev);
            if (tabKey && !(ev.target instanceof HTMLInputElement || ev.target instanceof HTMLTextAreaElement)) {
                ev.preventDefault();
                ev.stopPropagation();
                runDocTabAction(tabKey, docTabs);
                return;
            }
            // The editor's keys are the `editor:*` rows of the shortcut table.
            // Save and Save As are CodeMirror keys (the save keymap).
            const command = paneCommandFor(ev, "editor");
            if ((command === "editor:find" || command === "editor:togglePreview" || command === "editor:focusTree") && runEditorCommand(command)) {
                ev.preventDefault();
                ev.stopPropagation();
            }
        };
        el.addEventListener("keydown", handleEditorKeys, { capture: true });
        onCleanup(() => root()?.removeEventListener("keydown", handleEditorKeys, { capture: true }));
    });
}
