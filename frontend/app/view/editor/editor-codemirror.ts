// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createEffect, on, onCleanup, onMount, untrack, type Accessor } from "solid-js";
import { EditorView, basicSetup } from "codemirror";
import { Compartment, EditorState, type Extension } from "@codemirror/state";
import { keymap } from "@codemirror/view";
import { search } from "@codemirror/search";
import { lintGutter } from "@codemirror/lint";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { focusManager } from "@/app/store/focusManager";
import { codeMirrorKeys } from "@/app/keybindings";
import { keybindingsVersion } from "@/app/keybindings/registry";
import { noteResolved } from "@/app/keybindings/app-api";
import type { CmHandle } from "./editor-cm-handle";
import { EDITOR_STATE_FIELDS, provideEditorStates, takeMovedEditorState } from "./editor-doc-tabs";
import { userIsTypingElsewhereIn } from "./editor-focus-guard";
import { LspSession } from "./editor-lsp-session";
import type { EditorViewModel } from "./editor-model";
import { editorTheme, isLightPolarity } from "./editor-theme";
import { cursorAtLine } from "./open-at-line";

// ── Language loader ─────────────────────────────────────────────────────────
// Lazy-load language extensions to keep initial bundle small.

async function loadLanguage(lang: string): Promise<Extension | null> {
    try {
        switch (lang) {
            case "typescript":
            case "javascript": {
                const { javascript } = await import("@codemirror/lang-javascript");
                return javascript({ typescript: lang === "typescript", jsx: true });
            }
            case "python": {
                const { python } = await import("@codemirror/lang-python");
                return python();
            }
            case "rust": {
                const { rust } = await import("@codemirror/lang-rust");
                return rust();
            }
            case "html": {
                const { html } = await import("@codemirror/lang-html");
                return html();
            }
            case "css": {
                const { css } = await import("@codemirror/lang-css");
                return css();
            }
            case "json": {
                const { json } = await import("@codemirror/lang-json");
                return json();
            }
            case "markdown": {
                const { markdown } = await import("@codemirror/lang-markdown");
                return markdown();
            }
            default:
                return null;
        }
    } catch {
        return null;
    }
}

export interface CodeMirrorDeps {
    model: EditorViewModel;
    cm: CmHandle;
    root: () => HTMLDivElement | undefined;
    container: Accessor<HTMLDivElement | undefined>;
    /** Keeps the markdown preview in step with the CodeMirror document. */
    setLiveDoc: (text: string) => void;
    runEditorCommand: (command: string) => boolean;
}

/**
 * CodeMirror for one Editor pane: building and rebuilding the view, keeping each tab's
 * state across switches, and following live changes to word wrap, the theme and the
 * save keys. The view itself is `cm.view`, shared with the pane's commands.
 */
export class CodeMirrorSession {
    readonly lintCompartment = new Compartment();
    /** Word wrap is a pane-wide setting, not per tab: a stale per-tab snapshot must not bring back an old value. */
    private readonly wordWrapCompartment = new Compartment();
    /** The Save keys come from the shortcut table and follow live changes to the user's keybindings. */
    private readonly saveKeysCompartment = new Compartment();
    /** CodeMirror's light/dark base styles follow the app theme's polarity, which can change while tabs are open. */
    private readonly themeCompartment = new Compartment();
    readonly lsp: LspSession;
    /** Per-tab CodeMirror state: cursor, selection, scroll and undo history across tab switches. */
    private readonly cmStates = new Map<string, EditorState>();
    private activeTabIdForCm: string | null = null;
    /** Guards `setup` against concurrent calls (see there). */
    private setupGeneration = 0;

    constructor(private readonly d: CodeMirrorDeps) {
        this.lsp = new LspSession(d.model, d.cm, this.lintCompartment);
    }

    private themeExtension(): Extension {
        return editorTheme(isLightPolarity());
    }

    /** Resyncs the theme when the app theme's polarity flips. */
    watchPolarity(): void {
        const polarityObserver = new MutationObserver(() => {
            this.d.cm.view?.dispatch({ effects: this.themeCompartment.reconfigure(this.themeExtension()) });
        });
        polarityObserver.observe(document.documentElement, { attributes: true, attributeFilter: ["data-theme-polarity"] });
        onCleanup(() => polarityObserver.disconnect());
    }

    /** A tab moving to another Editor takes its state from here: the live one for the tab in front, the saved one for the rest. */
    provideStates(): void {
        const { model, cm } = this.d;
        onCleanup(provideEditorStates(model, (tabId) => (tabId === this.activeTabIdForCm ? cm.view?.state : this.cmStates.get(tabId))));
    }

    /** Keys from the shortcut table (`editor:save`, `editor:saveAs`). */
    private saveKeymap(): Extension {
        const run = (command: string) => () => {
            // False when the command does not apply (Save As on a non-scratch tab), so the
            // key is not swallowed. Noted only when it applied, or PressKeys counts a key
            // that did nothing as resolved.
            const ran = this.d.runEditorCommand(command);
            if (ran) noteResolved(command, "editor");
            return ran;
        };
        return keymap.of([
            ...codeMirrorKeys("editor:save").map((key) => ({ key, run: run("editor:save") })),
            ...codeMirrorKeys("editor:saveAs").map((key) => ({ key, run: run("editor:saveAs") })),
        ]);
    }

    private wrapExtension(): Extension {
        return this.d.model.wordWrapAtom() ? [EditorView.lineWrapping] : [];
    }

    private buildExtensions(readOnly: boolean): Extension[] {
        const { model, setLiveDoc } = this.d;
        const extensions: Extension[] = [
            basicSetup,
            this.themeCompartment.of(this.themeExtension()),
            search(),
            // lintGutter installs the lint state field behind the inline diagnostic
            // underlines (LSP pushes via setDiagnostics); its gutter column is hidden in CSS.
            lintGutter(),
            this.lintCompartment.of([]),
            this.wordWrapCompartment.of(this.wrapExtension()),
            EditorView.updateListener.of((update) => {
                if (update.docChanged) {
                    const content = update.state.doc.toString();
                    model.onContentChange(content);
                    setLiveDoc(content); // keep the markdown preview in sync
                    this.lsp.scheduleChange(content);
                }
            }),
            // Ctrl+S → save; on scratch tabs triggers Save As instead.
            // Ctrl+Shift+S → Save As for scratch tabs only (Phase 1; non-scratch Save As is Phase 2).
            this.saveKeysCompartment.of(this.saveKeymap()),
        ];
        if (readOnly) {
            extensions.push(EditorState.readOnly.of(true));
        }
        return extensions;
    }

    private destroyView(): void {
        const { cm, model } = this.d;
        if (cm.view) {
            cm.view.destroy();
            cm.view = null;
            model.cmViewRef.current = null;
        }
    }

    /** The state a tab moved in from another Editor brings (its undo history and selection), if it is still the text shown. */
    private movedState(tabId: string | undefined, content: string, extensions: Extension[]): EditorState | undefined {
        // Taken only past the superseded check, so a superseded build leaves it for the next one.
        const moved = tabId ? takeMovedEditorState(this.d.model, tabId) : undefined;
        // Only when it is the text shown: a clean tab is read fresh on arrival, and its file may have changed meanwhile.
        if (moved === undefined || (moved as { doc?: unknown }).doc !== content) return undefined;
        try {
            return EditorState.fromJSON(moved, { extensions }, EDITOR_STATE_FIELDS);
        } catch {
            return undefined; // its text is still `content`
        }
    }

    /**
     * Builds or rebuilds CodeMirror. It is async and awaits `loadLanguage()` BETWEEN destroying the
     * previous view and constructing the new one, so two calls that both start before either constructs
     * would both find no view, both skip the destroy, and both append an EditorView to the same
     * container, orphaning one. The effects call it on container mount and again when loading flips,
     * ~2 ms apart, so `setupGeneration` lets only the newest call build.
     */
    async setup(content: string, language: string, readOnly: boolean): Promise<void> {
        const { model, cm, container, root, setLiveDoc } = this.d;
        const parent = container();
        if (!parent) return;
        const gen = ++this.setupGeneration;
        const tabId = untrack(model.activeIdAtom);

        this.destroyView();

        const extensions = this.buildExtensions(readOnly);

        // Load language extension
        const langExt = await loadLanguage(language);
        if (langExt) extensions.push(langExt);

        // A later setup() started while we awaited loadLanguage(): it owns the view.
        if (gen !== this.setupGeneration) return;
        // Defensive: any view present now was constructed during our await, so drop it rather than stack another.
        this.destroyView();

        const state = this.movedState(tabId, content, extensions);
        cm.view = new EditorView({
            state: state ?? EditorState.create({ doc: content, extensions }),
            parent,
        });
        // The extensions were built before the language loaded; the theme may have changed polarity since.
        cm.view.dispatch({ effects: this.themeCompartment.reconfigure(this.themeExtension()) });
        setLiveDoc(content); // seed preview from the freshly-built doc
        // Runs on every (re)build, not just the pane's first mount, so switching the in-pane
        // file-tab strip re-lands the caret too (SPEC_PANE_SELECT_AUTOFOCUS_2026_09_22.md §3a).
        model.cmViewRef.current = cm.view;
        // Opened at a line (pane.open's `line`): the cursor goes there once.
        const openAt = model.takeOpenAtLine(model.filePathAtom());
        if (openAt) cm.view.dispatch(cursorAtLine(cm.view.state, openAt));
        // Not while the user types in this pane's own file tree: a reload landing mid-keystroke
        // there must not yank the caret into CodeMirror (the spec's §5 constraint 1).
        if (!userIsTypingElsewhereIn(root(), document.activeElement)) {
            focusManager.claimFocusOnMount(model.blockId, () => model.giveFocus());
        }
    }

    /** Loads the tree's roots and builds the first view when the pane mounts with a file already open. */
    mountInitial(): void {
        const { model, setLiveDoc } = this.d;
        onMount(() => {
            // Load file-tree roots from backend: $HOME (auto-expanded) +
            // every reachable drive/mount (sibling roots, collapsed).
            void RpcApi.GetEditorRootsCommand(TabRpcClient, {}).then((res) => {
                if (res?.home) {
                    void model.treeModel.setRootsAndLoad(res.home, res.drives ?? []);
                }
            });

            const content = model.contentAtom();
            const lang = model.languageAtom();
            const readOnly = model.readOnlyAtom();
            if (content || model.filePathAtom()) {
                setLiveDoc(content); // seed before async setup to avoid a blank preview
                void this.setup(content, lang, readOnly);
            }
        });
    }

    /** The effects that keep the view in step with the model: tab changes, word wrap, the user's keybindings. */
    useLiveEffects(): void {
        const { model, cm } = this.d;
        createEffect(() => this.onTabChange());

        // Live-toggle word wrap (right-click menu → model.toggleWordWrap()) on
        // whichever tab is currently showing, without rebuilding CodeMirror.
        createEffect(() => {
            const wrap = model.wordWrapAtom();
            if (!cm.view) return;
            cm.view.dispatch({ effects: this.wordWrapCompartment.reconfigure(wrap ? [EditorView.lineWrapping] : []) });
        });

        // Follow live changes to the user's keybindings without rebuilding CodeMirror.
        createEffect(
            on(
                keybindingsVersion,
                () => cm.view?.dispatch({ effects: this.saveKeysCompartment.reconfigure(this.saveKeymap()) }),
                { defer: true }
            )
        );
    }

    /**
     * Rebuilds CodeMirror on tab change. Tracks activeIdAtom (not just filePathAtom) so each tab
     * gets a unique trigger; the outgoing tab's state is snapshotted into cmStates BEFORE the
     * rebuild so it can be restored on the next switch back.
     */
    private onTabChange(): void {
        const { model, cm, container } = this.d;
        const activeId = model.activeIdAtom(); // reactive: tab change
        const loading = model.loadingAtom(); // reactive: wait for content
        // Reactive on purpose (#3260): an external change to a CLEAN active tab
        // replaces the buffer and bumps _contentVersion without moving `loading`, so the content
        // itself is tracked or the pane would show stale text and a later save could overwrite the
        // change. Typing cannot re-trigger this: onContentChange() does NOT bump _contentVersion
        // (CodeMirror is the source of truth for the live buffer), and a dirty tab is never reloaded.
        model.contentAtom();
        // A tab in another viewer (an image, a video) has no CodeMirror buffer: keep the outgoing
        // text tab's state for when it comes back, and leave CodeMirror as it is, hidden.
        if (model.activeViewer() !== "text") {
            untrack(() => {
                if (this.activeTabIdForCm && cm.view) this.cmStates.set(this.activeTabIdForCm, cm.view.state);
                this.activeTabIdForCm = null;
            });
            return;
        }
        // container() is reactive: when the body <Show> mounts the container after content loads,
        // this effect re-runs and proceeds (fixes the first-open blank-preview race).
        if (!activeId || loading || !container()) return;

        untrack(() => this.restoreOrBuild(activeId));
    }

    private restoreOrBuild(activeId: string): void {
        const { model, cm, setLiveDoc } = this.d;
        const path = model.filePathAtom();
        const content = model.contentAtom();
        const lang = model.languageAtom();
        const readOnly = model.readOnlyAtom();

        // Snapshot outgoing tab's CodeMirror state for cursor/scroll/undo preservation.
        // Skipped on first mount (no prevId).
        if (this.activeTabIdForCm && this.activeTabIdForCm !== activeId && cm.view) {
            this.cmStates.set(this.activeTabIdForCm, cm.view.state);
        }
        this.activeTabIdForCm = activeId;

        // A saved state for this tab is restored via setState on the existing view (no destroy);
        // otherwise a fresh one is built. The snapshot is dropped when the tab's content was
        // reloaded underneath us (#3260): cmStates is only cleared on
        // TabClosed, so restoring a stale one would never reach setup() with the freshly-read
        // content and a later save would clobber the external change. Compare the SNAPSHOT against
        // the content, not the live view (it still holds the OUTGOING tab). Never when dirty: a
        // dirty buffer legitimately differs from disk and is never reloaded.
        let saved = this.cmStates.get(activeId);
        if (saved && !model.dirtyAtom() && saved.doc.toString() !== content) {
            this.cmStates.delete(activeId);
            saved = undefined;
        }
        if (saved && cm.view) {
            cm.view.setState(saved);
            // Resync the pane-wide settings: the restored state was snapshotted with whatever
            // wrap value was active at that time, which may be stale.
            cm.view.dispatch({
                effects: [
                    this.wordWrapCompartment.reconfigure(this.wrapExtension()),
                    this.saveKeysCompartment.reconfigure(this.saveKeymap()),
                    this.themeCompartment.reconfigure(this.themeExtension()),
                ],
            });
            setLiveDoc(saved.doc.toString()); // seed preview from restored doc
            // Re-wire LSP for the now-active file's content.
            void this.lsp.startIfSupported(path, lang, content);
            return;
        }
        setLiveDoc(content); // seed before async setup to avoid a blank preview
        void this.setup(content, lang, readOnly).then(() => {
            void this.lsp.startIfSupported(path, lang, content);
        });
    }

    /** Clears a closed tab's cached state (so reopening the file starts fresh), and everything when the pane goes. */
    watchTabClose(): void {
        const { model, cm } = this.d;
        const unsubSliceEvents = model.onSliceEvent((event) => {
            if (event.type === "TabClosed") {
                this.cmStates.delete(event.tabId);
                // The tab in front left. It isn't the outgoing tab to snapshot when the next one
                // shows: a tab moved to another pane keeps its id, and a snapshot taken now would
                // come back over its newer text if it is moved back.
                if (this.activeTabIdForCm === event.tabId) this.activeTabIdForCm = null;
            }
        });

        onCleanup(() => {
            void this.lsp.teardown();
            unsubSliceEvents();
            this.cmStates.clear();
            cm.view?.destroy();
            cm.view = null;
            model.cmViewRef.current = null;
        });
    }
}
