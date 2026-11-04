// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Editor view — file-tree column on the left + CodeMirror on the right.
// Tree visibility (model.treeExpandedAtom): hidden by the first button on the
// tree's toolbar, shown again by the button leading the document bar.
// Spec: docs/specs/SPEC_EDITOR_FILE_TREE_2026-05-26.md

import { createSignal, onCleanup, onMount, Show, type Accessor, type JSX } from "solid-js";
import { ContextMenu } from "@/app/components/context-menu";
import { ConfirmDialog } from "@/app/components/confirm-dialog";
import { DocTabStrip } from "@/app/doc-tabs/DocTabStrip";
import type { DocTabsModel } from "@/app/doc-tabs/doc-tabs-controller";
import type { EditorBuffer } from "@/app/store/editor-pane-state-store";
import { IconButton } from "@/app/element/ui";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { TextViewerSwitch, ViewerHost } from "./viewers/viewer-host";
import { SaveAsInput } from "./editor-save-as-input";
import { editorDocTabs } from "./editor-doc-tabs";
import { registerDocTabDropZone } from "@/app/doc-tabs/doc-tab-hosts";
import { FileTree } from "./file-tree";
import "./editor-view.scss";
import { keyLabel } from "@/app/keybindings";
import { OpenFromRemoteModal } from "./open-from-remote-modal";
import { EditorEmptyState } from "./EditorEmptyState";
import { EditorErrorPanel, EditorLspBanner, EditorLspStatusChip, EditorMarkdownPreview, EditorModeToolbar } from "./EditorPanels";
import { lspStatusChipText } from "./editor-lsp-status";
import type { EditorViewModel } from "./editor-model";
import { tip } from "@/app/element/tip";
import type { LspState } from "./lsp/lsp-client";
import type { CmHandle } from "./editor-cm-handle";
import { CodeMirrorSession } from "./editor-codemirror";
import {
    createSaveAs,
    focusEditorBody,
    useEditorCommands,
    useEditorKeys,
    useFocusRegion,
    type EditorShows,
} from "./editor-commands";
import { copyToClipboard, useLspBanner, useTreeMenu, type TreeMenu } from "./editor-tree-state";
import { previewResizeHandler, treeResizeHandler, useEditorZoom } from "./editor-zoom-resize";

// ── Editor View Component ───────────────────────────────────────────────────

/**
 * The handful of shortcuts worth surfacing on the empty editor, in this
 * platform's notation. Not Save: there is nothing to save until a file is open.
 */
function editorEmptyShortcuts(): { keys: string[]; label: string }[] {
    return [
        { keys: [keyLabel("mod+f")], label: "Find" },
        { keys: [keyLabel("mod+shift+v")], label: "Toggle .md preview" },
        { keys: [keyLabel("mod+="), keyLabel("mod+-")], label: "Zoom" },
    ];
}

/**
 * Load file-tree roots from the backend: $HOME (auto-expanded) plus every
 * reachable drive/mount (sibling roots, collapsed).
 */
function loadTreeRoots(model: EditorViewModel): void {
    void RpcApi.GetEditorRootsCommand(TabRpcClient, {}).then((res) => {
        if (res?.home) {
            void model.treeModel.setRootsAndLoad(res.home, res.drives ?? []);
        }
    });
}

export function EditorViewComponent(props: { model: EditorViewModel }): JSX.Element {
    const model = props.model;
    // Reactive ref: the markdown preview seeds `liveDoc` from the tab-change
    // effect, which guards on the container being mounted. On first open the
    // container mounts only AFTER content finishes loading (the body <Show>
    // gates on !loadingAtom), so a plain `let` ref left the effect early-
    // returning before the container existed → blank preview until a manual
    // Source/Preview toggle. A signal makes the effect re-run when the
    // container mounts, seeding the preview reliably.
    const [containerRef, setContainerRef] = createSignal<HTMLDivElement>();
    let rootRef: HTMLDivElement | undefined;
    const root = () => rootRef;
    const cm: CmHandle = { view: null };

    const emptyShortcuts = editorEmptyShortcuts();

    // Live CodeMirror doc text for the active tab — the markdown preview binds to this, NOT
    // model.contentAtom(): that memo only recomputes on file load / tab change (onContentChange
    // deliberately doesn't bump it on keystrokes), so it would render stale pre-edit text. It is
    // seeded whenever CM is (re)built or restored, and updated on every doc change.
    const [liveDoc, setLiveDoc] = createSignal("");
    // Markdown's preview, and CodeMirror itself, belong to the text viewer:
    // an image or a PDF tab shows its own viewer (viewers/viewer-host.tsx).
    const showsText = (): boolean => model.activeViewer() === "text";
    const isMarkdown = (): boolean => showsText() && model.languageAtom() === "markdown";
    const shows: EditorShows = { showsText, isMarkdown };

    // The pane's tabs on the shared strip and keys (PLAN_EDITOR_ON_SHARED_DOC_TABS).
    const docTabs = editorDocTabs(model);
    const saveAs = createSaveAs(model);
    const focus = useFocusRegion(root);
    const runEditorCommand = useEditorCommands({ model, cm, root, docTabs, ...shows, focusRegion: focus.focusRegion, triggerSaveAs: saveAs.triggerSaveAs });
    const { contentZoomed, rootZoom } = useEditorZoom(model, cm, root);
    useEditorKeys(root, docTabs, runEditorCommand);

    const session = new CodeMirrorSession({ model, cm, root, container: containerRef, setLiveDoc, runEditorCommand });
    session.watchPolarity();
    session.provideStates();
    onMount(() => loadTreeRoots(model));
    session.mountInitial();
    session.useLiveEffects();

    // An Editor tab from another pane, dropped anywhere on this one, joins it.
    onMount(() => {
        if (rootRef) onCleanup(registerDocTabDropZone(rootRef, "editor", model.blockId));
    });
    session.watchTabClose();

    const tree = useTreeMenu(model);
    const banner = useLspBanner(session.lsp.lspState);
    const showTreeButton = () => !model.treeExpandedAtom() && !model.connection();

    return (
        <div
            ref={(el) => { rootRef = el; }}
            class="editor-view"
            classList={{ "editor-view--tree-collapsed": !model.treeExpandedAtom() }}
            data-zoom-scope={contentZoomed() ? "chrome" : undefined}
            style={{ zoom: rootZoom() }}
            onFocusIn={focus.trackFocusRegion}
            onFocusOut={focus.leaveFocusRegion}
        >
            <Show when={model.openFromRemoteAtom() && rootRef}>
                <OpenFromRemoteModal
                    blockRef={{ current: rootRef! }}
                    anchorRef={{ current: rootRef! }}
                    current={model.connection()}
                    onOpen={(host, path) => model.openFromRemote(host, path)}
                    onClose={() => model.setOpenFromRemote(false)}
                />
            </Show>
            <Show when={model.treeExpandedAtom()}>
                <EditorTreeColumn
                    model={model}
                    tree={tree}
                    focusRegion={focus.focusRegion}
                    onResize={treeResizeHandler(model, rootZoom)}
                    onFocusEditor={() => focusEditorBody(model, cm, root, shows)}
                />
            </Show>
            <EditorMainColumn
                model={model}
                docTabs={docTabs}
                saveAs={saveAs}
                shows={shows}
                focusRegion={focus.focusRegion}
                lspState={session.lsp.lspState}
                banner={banner}
                liveDoc={liveDoc}
                setContainerRef={setContainerRef}
                emptyShortcuts={emptyShortcuts}
                showTreeButton={showTreeButton}
                onPreviewResize={previewResizeHandler(model, rootZoom)}
            />
            <EditorDialogs tree={tree} />
        </div>
    );
}

/** The file tree beside the document, and the handle that resizes it. */
function EditorTreeColumn(props: {
    model: EditorViewModel;
    tree: TreeMenu;
    focusRegion: Accessor<"tree" | "body" | null>;
    onResize: (e: MouseEvent) => void;
    onFocusEditor: () => void;
}): JSX.Element {
    const { model, tree } = props;
    return (
        <>
            <div
                class="editor-tree-column editor-region"
                classList={{ "editor-region--focused": props.focusRegion() === "tree" }}
                style={{
                    width: `${model.treeWidthAtom()}px`,
                    flex: `0 0 ${model.treeWidthAtom()}px`,
                }}
            >
                <FileTree
                    model={model.treeModel}
                    activeFilePath={model.filePathAtom()}
                    showHidden={model.showHiddenAtom()}
                    onFileClick={(path) => void model.openFilePreview(path)}
                    onFileDblClick={(path) => void model.openFile(path)}
                    onToggleHidden={() => void model.toggleShowHidden()}
                    onHideTree={() => void model.toggleTreeExpanded()}
                    onContextMenu={tree.handleTreeContextMenu}
                    renamingPath={tree.renamingPath()}
                    onRenameConfirm={(path, name) => void tree.handleRenameConfirm(path, name)}
                    onRenameCancel={() => tree.setRenamingPath(null)}
                    newEntry={tree.newEntry()}
                    onNewEntryConfirm={(parent, name, kind) => void tree.handleNewEntryConfirm(parent, name, kind)}
                    onNewEntryCancel={() => tree.setNewEntry(null)}
                    onStartRename={(path) => tree.setRenamingPath(path)}
                    onFocusEditor={props.onFocusEditor}
                />
            </div>
            <div
                class="editor-tree-resize-handle"
                onMouseDown={props.onResize}
                {...tip("Drag to resize file tree", { label: true })}
            />
        </>
    );
}

/** The document column: the tab bar, the notices, and the body (CodeMirror, a viewer or the empty state). */
function EditorMainColumn(props: {
    model: EditorViewModel;
    docTabs: DocTabsModel<EditorBuffer>;
    saveAs: ReturnType<typeof createSaveAs>;
    shows: EditorShows;
    focusRegion: Accessor<"tree" | "body" | null>;
    lspState: Accessor<LspState | null>;
    banner: ReturnType<typeof useLspBanner>;
    liveDoc: Accessor<string>;
    setContainerRef: (el: HTMLDivElement) => void;
    emptyShortcuts: { keys: string[]; label: string }[];
    showTreeButton: () => boolean;
    onPreviewResize: (e: MouseEvent) => void;
}): JSX.Element {
    const { model, docTabs, saveAs, banner } = props;
    const { showsText, isMarkdown } = props.shows;
    const lspState = props.lspState;
    const statusChipText = (): string => lspStatusChipText(lspState(), model.languageAtom());
    const statusChipKind = (): string => {
        const s = lspState();
        return s?.kind ?? "none";
    };
    return (
        <>
            {/* The focus line only says something beside the tree. */}
            <div
                class="editor-main-column"
                classList={{
                    "editor-region": model.treeExpandedAtom(),
                    "editor-region--focused": model.treeExpandedAtom() && props.focusRegion() === "body",
                }}
            >
                <EditorTabBar model={model} docTabs={docTabs} saveAs={saveAs} showTreeButton={props.showTreeButton} />

                <Show when={isMarkdown() && model.tabsAtom().length > 0}>
                    <EditorModeToolbar model={model} />
                </Show>

                <Show when={model.loadingAtom()}>
                    <div class="editor-loading">Loading...</div>
                </Show>

                {/* Full-pane error panel when a file failed to load (invalid path, permission
                    denied, binary): replaces the CodeMirror body for the active tab while the
                    error sticks. Errors with content already loaded (save failures) still show
                    in the top banner below. */}
                <Show when={model.errorAtom() && model.activeTabAtom() && !model.activeTabAtom()?.contentLoaded}>
                    <EditorErrorPanel model={model} />
                </Show>

                <Show when={model.errorAtom() && model.activeTabAtom()?.contentLoaded}>
                    <div class="editor-error">{model.errorAtom()}</div>
                </Show>

                {/* LSP install banner — appears when the language server's binary
                    isn't on PATH. Dismissable per session via the X button. */}
                <Show when={banner.lspBannerVisible()}>
                    <EditorLspBanner state={lspState} onDismiss={banner.dismissBanner} onCopy={copyToClipboard} />
                </Show>

                <EditorBody
                    model={model}
                    shows={props.shows}
                    liveDoc={props.liveDoc}
                    setContainerRef={props.setContainerRef}
                    emptyShortcuts={props.emptyShortcuts}
                    showTreeButton={props.showTreeButton}
                    onPreviewResize={props.onPreviewResize}
                />

                {/* LSP status chip — bottom-of-pane indicator. Only shown when
                    there's an active client (i.e. an LSP-supported file is open). */}
                <Show when={showsText() && lspState() && lspState()?.kind !== "disposed"}>
                    <EditorLspStatusChip kind={statusChipKind()} text={statusChipText()} />
                </Show>
            </div>
        </>
    );
}

/** The document tab bar; while the tree is hidden, the button that shows it leads the bar. */
function EditorTabBar(props: {
    model: EditorViewModel;
    docTabs: DocTabsModel<EditorBuffer>;
    saveAs: ReturnType<typeof createSaveAs>;
    showTreeButton: () => boolean;
}): JSX.Element {
    const { model, docTabs, saveAs } = props;
    return (
        <DocTabStrip
            ctl={docTabs}
            docType="editor"
            blockId={model.blockId}
            show={() => props.showTreeButton() || docTabs.tabs().length > 0}
            leading={
                // While the tree is hidden, the button that shows it
                // leads the document bar (the tree's own toolbar hides
                // it). An editor on a remote host has no tree.
                <Show when={props.showTreeButton()}>
                    <IconButton
                        icon="folder-tree"
                        label="Show file tree"
                        density="compact"
                        class="editor-tree-toggle"
                        onClick={() => void model.toggleTreeExpanded()}
                    />
                </Show>
            }
            tooltipOf={(t) => t.payload.filePath}
            addTitle={`New scratch buffer (${keyLabel("ctrl+t")})`}
            // Not while its Save As box is open: the drag would take
            // the box's text selection.
            canDrag={(t) => saveAs.saveAsTabId() !== t.id}
            tabClass={(t) => ({ "editor-tab--saveas": saveAs.saveAsTabId() === t.id })}
            renderLabel={(t) =>
                saveAs.saveAsTabId() === t.id ? (
                    <SaveAsInput onConfirm={(path) => void saveAs.handleSaveAsConfirm(path)} onCancel={() => saveAs.setSaveAsTabId(null)} />
                ) : undefined
            }
        />
    );
}

/** The document body: CodeMirror or a viewer with the Markdown preview beside it, or the empty state. */
function EditorBody(props: {
    model: EditorViewModel;
    shows: EditorShows;
    liveDoc: Accessor<string>;
    setContainerRef: (el: HTMLDivElement) => void;
    emptyShortcuts: { keys: string[]; label: string }[];
    showTreeButton: () => boolean;
    onPreviewResize: (e: MouseEvent) => void;
}): JSX.Element {
    const { model } = props;
    const { showsText, isMarkdown } = props.shows;
    return (
        <Show
            when={
                model.filePathAtom() &&
                !model.loadingAtom() &&
                // Don't render CodeMirror when the active tab failed
                // to load — the centered error panel takes the body.
                !(model.errorAtom() && model.activeTabAtom() && !model.activeTabAtom()?.contentLoaded)
            }
            fallback={<EditorEmptyState shortcuts={props.emptyShortcuts} onOpenFromRemote={() => model.setOpenFromRemote(true)} showTreeHint={props.showTreeButton} />}
        >
            <div class="editor-body-wrap">
                <Show when={!showsText()} fallback={<TextViewerSwitch model={model} />}>
                    <ViewerHost model={model} />
                </Show>
                <div
                    class="editor-codemirror"
                    ref={props.setContainerRef}
                    style={{
                        display: !showsText() || (isMarkdown() && model.editorMode() === "preview") ? "none" : undefined,
                    }}
                />
                <Show when={isMarkdown() && model.editorMode() === "split"}>
                    <div
                        class="editor-preview-divider"
                        onMouseDown={props.onPreviewResize}
                        {...tip("Drag to resize preview", { label: true })}
                    />
                </Show>
                <Show when={isMarkdown() && model.editorMode() !== "source"}>
                    <EditorMarkdownPreview model={model} text={props.liveDoc} />
                </Show>
            </div>
        </Show>
    );
}

/** The file tree's context menu and the confirmation dialog, above everything. */
function EditorDialogs(props: { tree: TreeMenu }): JSX.Element {
    const { tree } = props;
    return (
        <>
            {/* File-tree context menu — rendered via Portal above everything */}
            <Show when={tree.ctxMenu()}>
                {(menu) => (
                    <ContextMenu
                        items={menu().items}
                        x={menu().x}
                        y={menu().y}
                        onClose={() => tree.setCtxMenu(null)}
                    />
                )}
            </Show>

            {/* Delete confirmation modal — replaces window.confirm() */}
            <Show when={tree.deleteConfirm()}>
                {(dc) => (
                    <ConfirmDialog
                        title={dc().title}
                        message={dc().message}
                        confirmLabel={dc().confirmLabel ?? "Delete"}
                        onConfirm={() => { dc().onConfirm(); tree.setDeleteConfirm(null); }}
                        onCancel={() => tree.setDeleteConfirm(null)}
                    />
                )}
            </Show>
        </>
    );
}
