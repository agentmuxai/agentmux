// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// NOTE: Editor is a pane-level view for editing files with syntax highlighting.
// It is NOT a standalone IDE — complex editing happens in the agent's terminal
// or via agent tool calls. This covers quick edits, file viewing, and diffing.
//
// State management: slice #10 editor-pane-state (frontend/app/store/
// editor-pane-state-store.ts) owns the tab list, active id, dirty flags, and
// recently-closed stack. This file is a thin projection layer between that
// slice and the editor view. Tab-content blobs are held in a view-local Map
// (this._contentByTab) — content is deliberately NOT in the slice (large,
// not auditable, not persistable cheaply). The slice tracks contentHash +
// contentLoaded so it can reason about dirty-vs-disk without holding the
// buffer.
//
// Spec: specs/SPEC_EDITOR_TABS_2026-05-26.md (Phase 1B).
// Earlier specs: SPEC_EDITOR_FILE_TREE_2026-05-26.md, SPEC_EDITOR_LSP_AND_THEMES_2026-05-26.md.

import { BlockNodeModel } from "@/app/block/blocktypes";
import { pushNotification, setActiveTab, useBlockAtom, workspace } from "@/app/store/global";
import {
    EditorPaneEvent,
    EditorTab,
    canonicalizePath,
    dispatch,
    registerEditorPane,
    setEventSink,
    snapshot,
    unregisterEditorPane,
    getAllActiveScratchIds,
} from "@/app/store/editor-pane-state-store";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { WorkspaceService } from "@/app/store/services";
import { createBlockOnModel, waitForLayoutModel } from "@/app/tab/tab-presets";
import { getWaveObjectAtom, makeORef } from "@/app/store/wos";
import { waveEventSubscribe } from "@/app/store/wps";
import { WpsEvent } from "@/app/store/wps-events";
import { createMemo, createSignal, type Accessor } from "solid-js";
import { FileTreeModel } from "./file-tree-model";

const META_TREE_EXPANDED = "editor:tree_expanded";
const META_SHOW_HIDDEN = "editor:show_hidden";
const META_TREE_WIDTH = "editor:tree_width";
const META_PREVIEW_HEIGHT = "editor:preview_height";
const META_LEGACY_FILE = "file";

export type EditorMode = "preview" | "source" | "split";
const META_SCRATCH = "editor:scratch";
const TREE_WIDTH_DEFAULT = 240;
const TREE_WIDTH_MIN = 150;
const TREE_WIDTH_MAX = 600;
const PREVIEW_HEIGHT_DEFAULT = 300;
const PREVIEW_HEIGHT_MIN = 80;
const PREVIEW_HEIGHT_MAX = 1200;

/** Hash a string with SHA-256 → hex. Used for the slice's contentHash field
 *  so the saga (Phase 1C) can decide whether a tab is dirty vs. disk. Cheap;
 *  files are capped at 10 MB by the read RPC. */
async function sha256Hex(s: string): Promise<string> {
    const buf = new TextEncoder().encode(s);
    const digest = await crypto.subtle.digest("SHA-256", buf);
    return Array.from(new Uint8Array(digest))
        .map((b) => b.toString(16).padStart(2, "0"))
        .join("");
}

// ── Module-level event-sink fan-out ───────────────────────────────────────
// The slice's setEventSink is a singleton — a per-instance call would
// last-writer-wins and silently disable earlier panes' subscriptions. We
// install the sink ONCE at module load and dispatch every slice event to
// each registered EditorViewModel instance. Add/remove on construct/dispose.
const _instanceHandlers = new Set<(events: EditorPaneEvent[]) => void>();
let _sinkInstalled = false;
function installGlobalSinkOnce(): void {
    if (_sinkInstalled) return;
    setEventSink((events: EditorPaneEvent[]) => {
        for (const handler of _instanceHandlers) handler(events);
    });
    _sinkInstalled = true;
}

export class EditorViewModel implements ViewModel {
    viewType = "editor";
    blockId: string;
    nodeModel: BlockNodeModel;

    viewIcon: Accessor<string | IconButtonDecl>;
    viewName: Accessor<string>;
    viewText: Accessor<string | HeaderElem[]>;
    noPadding: Accessor<boolean> = () => true;

    get viewComponent(): ViewComponent {
        return null; // overridden by barrel via Object.defineProperty
    }

    // ── Slice projection ───────────────────────────────────────────────
    // A version counter signal triggers Solid re-evaluation whenever the
    // slice's slot cell mutates. We use a setEventSink callback (set in the
    // constructor) to bump the counter on every dispatch.
    private _sliceVersion = createSignal<number>(0);
    sliceVersionAtom: Accessor<number> = this._sliceVersion[0];

    tabsAtom: Accessor<EditorTab[]>;
    activeIdAtom: Accessor<string | null>;
    activeTabAtom: Accessor<EditorTab | null>;

    // ── Derived accessors (preserve the old surface so editor-view.tsx
    //    keeps working unchanged) ────────────────────────────────────
    filePathAtom: Accessor<string>;
    languageAtom: Accessor<string>;
    dirtyAtom: Accessor<boolean>;
    readOnlyAtom: Accessor<boolean>;
    errorAtom: Accessor<string | null>;
    loadingAtom: Accessor<boolean>;

    /** Content of the active tab. Sourced from the view-local Map; signals
     *  re-evaluation when the active tab changes OR when its content
     *  loads/updates. */
    contentAtom: Accessor<string>;

    // ── Per-tab content store (view-local) ────────────────────────────
    // Content blobs by tabId. Keys removed on TabClosed.
    private _contentByTab = new Map<string, string>();
    // Bump-counter signal so contentAtom re-evaluates when we mutate the Map.
    private _contentVersion = createSignal<number>(0);
    // Detected text encoding per tab (SPEC_EDITOR_FILE_ENCODINGS), captured on
    // read so save round-trips the original encoding/bom/line-ending instead of
    // silently rewriting the file as UTF-8.
    private _encodingByTab = new Map<
        string,
        { encoding: string; bom: string; lineEnding: string; hadDecodeErrors: boolean }
    >();

    // ── Tree state (per-pane, persisted in block meta) ──────────────────
    private _treeExpanded = createSignal<boolean>(true);
    treeExpandedAtom: Accessor<boolean> = this._treeExpanded[0];

    private _showHidden = createSignal<boolean>(false);
    showHiddenAtom: Accessor<boolean> = this._showHidden[0];

    private _treeWidth = createSignal<number>(TREE_WIDTH_DEFAULT);
    treeWidthAtom: Accessor<number> = this._treeWidth[0];

    private _previewHeight = createSignal<number>(PREVIEW_HEIGHT_DEFAULT);
    previewHeightAtom: Accessor<number> = this._previewHeight[0];

    // Per-tab editor mode: "preview" | "source" | "split".
    // Not persisted — tabs return to their language-appropriate default on reopen.
    private _tabModes = new Map<string, EditorMode>();
    private _tabModesVersion = createSignal<number>(0);

    treeModel = new FileTreeModel();

    blockAtom: Accessor<Block | undefined>;
    zoomAtom!: Accessor<number>;

    /** Tabs that started loading via openFile — used so concurrent dispatches
     *  for the same path don't double-fetch. */
    private _loadingPaths = new Set<string>();

    /** Slice event subscribers wired from the view (for snapshot/restore of
     *  CodeMirror state, dirty-confirm modal, LSP didOpen/didClose). */
    private _eventSubscribers = new Set<(event: EditorPaneEvent) => void>();

    /** Per-instance handler registered into the module-level fan-out.
     *  Removed on dispose. */
    private _globalHandler: (events: EditorPaneEvent[]) => void;

    // ── Live-reload (SPEC_EDITOR_LIVE_FILE_RELOAD_2026_07_18) ───────────
    // Path currently registered with the backend watcher, per tabId. A tab
    // is watched from the moment its content first loads until it closes
    // (or the pane disposes); a preview-tab file swap re-syncs it (unwatch
    // old path, watch new). Backend refcounts by block_id, so re-watching
    // the same path from this same pane is idempotent.
    private _watchedPathByTab = new Map<string, string>();
    private _unsubFileChanged: () => void = () => {};

    constructor(blockId: string, nodeModel: BlockNodeModel) {
        this.blockId = blockId;
        this.nodeModel = nodeModel;

        this.blockAtom = getWaveObjectAtom<Block>(makeORef("block", blockId));

        // Register this pane's slot in the slice.
        registerEditorPane(blockId);

        // Install the module-level event-sink fan-out the first time any
        // editor pane is constructed; subsequent panes share it.
        installGlobalSinkOnce();

        // Register this instance's handler in the fan-out. Bumps our local
        // sliceVersion signal so all projections re-evaluate, then mirrors
        // events to view-side subscribers (CodeMirror snapshot/restore,
        // future dirty-confirm modal, LSP coupling). Removed on dispose so
        // no closure over a disposed model survives.
        this._globalHandler = (events: EditorPaneEvent[]) => {
            this._sliceVersion[1]((v) => v + 1);
            for (const ev of events) {
                if (ev.type === "TabClosed") {
                    this._contentByTab.delete(ev.tabId);
                    this._encodingByTab.delete(ev.tabId);
                    this._tabModes.delete(ev.tabId);
                    this._unwatchTab(ev.tabId);
                }
                for (const sub of this._eventSubscribers) sub(ev);
            }
        };
        _instanceHandlers.add(this._globalHandler);

        // Live-reload: one subscription per pane, scoped to this block.
        // Fires when a path open in one of this pane's tabs changes on disk.
        this._unsubFileChanged = waveEventSubscribe({
            eventType: WpsEvent.EditorFileChanged,
            scope: makeORef("block", blockId),
            handler: (event) => {
                const path = (event as any)?.data?.path as string | undefined;
                if (path) void this._handleExternalFileChanged(path);
            },
        });

        // Projections from the slice's slot cell. Reading sliceVersionAtom
        // creates the Solid dependency that makes these re-evaluate.
        //
        // Wrapped in `useBlockAtom` (which calls createRoot under the hood) —
        // a bare createMemo here would be created in the constructor's
        // non-reactive scope. It'd compute its initial value but downstream
        // subscribers (createEffect in editor-view) wouldn't reliably re-run
        // when the source signals change. Same fix as the zoomAtom in
        // PR #1084 — see that comment for context.
        this.tabsAtom = useBlockAtom(blockId, "editor-tabs-list", () =>
            createMemo<EditorTab[]>(() => {
                this.sliceVersionAtom();
                return snapshot(this.blockId)?.tabs ?? [];
            }),
        );
        this.activeIdAtom = useBlockAtom(blockId, "editor-tabs-active-id", () =>
            createMemo<string | null>(() => {
                this.sliceVersionAtom();
                return snapshot(this.blockId)?.activeTabId ?? null;
            }),
        );
        this.activeTabAtom = useBlockAtom(blockId, "editor-tabs-active-tab", () =>
            createMemo<EditorTab | null>(() => {
                const id = this.activeIdAtom();
                const tabs = this.tabsAtom();
                return tabs.find((t) => t.id === id) ?? null;
            }),
        );

        // Derived: per-active-tab signals. Existing consumers see the same
        // shape as before — they just track whichever tab is active.
        this.filePathAtom = () => this.activeTabAtom()?.filePath ?? "";
        this.languageAtom = () => this.activeTabAtom()?.language ?? "";
        this.dirtyAtom = () => this.activeTabAtom()?.dirty ?? false;
        this.readOnlyAtom = () => this.activeTabAtom()?.readOnly ?? false;
        this.errorAtom = () => this.activeTabAtom()?.loadError ?? null;
        // "Loading" is true between OpenFile and TabContentLoaded — i.e. the
        // active tab exists but its content hasn't arrived yet.
        this.loadingAtom = () => {
            const tab = this.activeTabAtom();
            return tab != null && !tab.contentLoaded && tab.loadError == null;
        };

        this.contentAtom = useBlockAtom(blockId, "editor-tabs-content", () =>
            createMemo<string>(() => {
                this._contentVersion[0](); // dep on content bumps
                const id = this.activeIdAtom();
                return id ? this._contentByTab.get(id) ?? "" : "";
            }),
        );

        // Per-pane zoom (PR #1084). Wrapped in useBlockAtom (which calls
        // createRoot under the hood) — a bare createMemo here wouldn't have
        // a tracking owner and would snapshot once.
        this.zoomAtom = useBlockAtom(blockId, "editor-zoom", () =>
            createMemo<number>(() => {
                const z = this.blockAtom()?.meta?.["term:zoom"];
                if (typeof z !== "number" || isNaN(z)) return 1.0;
                return Math.max(0.5, Math.min(2.0, z));
            }),
        );

        // Pane title — full file path of active tab, with `*` for dirty.
        // Wrap in useBlockAtom for the same reason as tabsAtom/activeIdAtom —
        // a bare createMemo in the constructor doesn't sit inside a tracking
        // owner, so block-frame subscribers reading `viewName()` wouldn't
        // always see updates when the active tab changes.
        this.viewName = useBlockAtom(blockId, "editor-view-name", () =>
            createMemo<string>(() => {
                const fp = this.filePathAtom();
                if (!fp) return "Editor";
                return this.dirtyAtom() ? `${fp} *` : fp;
            }),
        );

        // Pane icon doubles as the file-tree expand/collapse toggle.
        this.viewIcon = useBlockAtom(blockId, "editor-view-icon", () =>
            createMemo<IconButtonDecl>(() => {
                const expanded = this.treeExpandedAtom();
                return {
                    elemtype: "iconbutton",
                    icon: expanded ? "folder-tree" : "folder",
                    title: expanded ? "Hide file tree" : "Show file tree",
                    click: () => void this.toggleTreeExpanded(),
                };
            }),
        );

        this.viewText = () => [];

        // Restore persisted tree state from block meta.
        const meta = this.blockAtom()?.meta;
        if (meta?.[META_TREE_EXPANDED] === false) {
            this._treeExpanded[1](false);
        }
        if (meta?.[META_SHOW_HIDDEN] === true) {
            this._showHidden[1](true);
        }
        const persistedWidth = meta?.[META_TREE_WIDTH];
        if (typeof persistedWidth === "number") {
            this._treeWidth[1](clampTreeWidth(persistedWidth));
        }
        const persistedPreviewH = meta?.[META_PREVIEW_HEIGHT];
        if (typeof persistedPreviewH === "number") {
            this._previewHeight[1](clampPreviewHeight(persistedPreviewH));
        }

        // Backwards-compat hydration: existing block meta uses `file` (the
        // pre-tabs key). If present, restore as a single tab. The saga in
        // Phase 1C will own the new `editor:tabs` key; this branch stays
        // until 1C lands + one minor version of grace.
        const legacyFile = meta?.[META_LEGACY_FILE];
        if (typeof legacyFile === "string" && legacyFile) {
            void this.openFile(legacyFile);
        } else if (meta?.[META_SCRATCH] === true && snapshot(blockId)?.tabs.length === 0) {
            // Widget default: open a scratch buffer when no file was persisted.
            void this.openScratch();
        }
    }

    // ── Event subscription (used by editor-view.tsx for CodeMirror state
    //    snapshot/restore + future modals) ──────────────────────────────
    onSliceEvent(handler: (event: EditorPaneEvent) => void): () => void {
        this._eventSubscribers.add(handler);
        return () => this._eventSubscribers.delete(handler);
    }

    // ── Tab actions (dispatched into the slice) ──────────────────────
    /** Open a file as a *pinned* tab (the default). Use for explicit-open
     *  paths like double-click in the tree, path-input submit, drag-drop,
     *  programmatic open. The pinned tab survives subsequent tree clicks. */
    async openFile(filePath: string): Promise<void> {
        return this._openFileWithMode(filePath, "pinned");
    }

    /** Open a file as a *preview* tab (VS Code-style). Use for tree single-
     *  click: at most ONE preview tab per pane; subsequent single-clicks
     *  replace its file. Editing the preview promotes it to pinned. */
    async openFilePreview(filePath: string): Promise<void> {
        return this._openFileWithMode(filePath, "preview");
    }

    /** Convert the active tab from preview → pinned. The view calls this
     *  when the user double-clicks the preview tab in the strip. No-op if
     *  the tab is already pinned. */
    pinActiveTab(): void {
        const tab = this.activeTabAtom();
        if (!tab || !tab.isPreview) return;
        dispatch(this.blockId, { type: "PinTab", tabId: tab.id, source: "user" });
    }

    private async _openFileWithMode(filePath: string, mode: "preview" | "pinned"): Promise<void> {
        const canonical = canonicalizePath(filePath);
        // If a load is already in flight for this path, we still need to
        // honor a preview→pinned transition (rapid dbl-click on a tree
        // row: the single-click added canonical to _loadingPaths, then
        // the dblclick's pinned-open would early-return and never pin).
        // Dispatch OpenFile so the slice runs the pin-if-existing logic,
        // then bail (the RPC's continuation will populate content as
        // usual).
        if (this._loadingPaths.has(canonical)) {
            dispatch(this.blockId, {
                type: "OpenFile",
                path: filePath,
                language: detectLanguage(filePath),
                mode,
                source: "user",
            });
            return;
        }
        this._loadingPaths.add(canonical);

        // Dispatch OpenFile. The slice either activates an existing tab (and
        // emits only TabActivated) or appends a new one (TabOpened +
        // TabActivated). Pass the *mapped* language (detectLanguage) — the
        // slice's bare `deriveLanguage` returns raw extensions, which
        // downstream lookups (loadLanguage in editor-view, LSP support
        // table) don't recognize.
        const events = dispatch(this.blockId, {
            type: "OpenFile",
            path: filePath,
            language: detectLanguage(filePath),
            mode,
            source: "user",
        });

        // Find the tab id (just-opened or pre-existing).
        const opened = events.find(
            (e) => e.type === "TabOpened" || e.type === "TabActivated",
        );
        if (!opened || (opened.type !== "TabOpened" && opened.type !== "TabActivated")) {
            this._loadingPaths.delete(canonical);
            return;
        }
        const tabId = opened.tabId;

        // If the slice considers content loaded for this tab AND we have it
        // in the view-local cache, we're done. The slice's contentLoaded
        // flag is the source of truth: it's reset to false when a preview
        // tab's file is swapped (same tabId, different path) so we
        // correctly force a reload in that case.
        const sliceTab = snapshot(this.blockId)?.tabs.find((t) => t.id === tabId);
        if (sliceTab?.contentLoaded && this._contentByTab.has(tabId)) {
            this._loadingPaths.delete(canonical);
            return;
        }
        // Stale view-local content (preview-swap left it behind) — evict
        // so the about-to-fire RPC populates the new file's content.
        this._contentByTab.delete(tabId);
        try {
            const hash = await this._loadFileIntoTab(tabId, filePath);
            if (hash !== null) {
                // Backwards-compat: persist the active file path under the
                // legacy meta key so a downgrade (or a Phase 1C-less build)
                // restores it. The 1C saga replaces this with the full
                // editor:tabs persistence.
                await this.persistMeta({ [META_LEGACY_FILE]: filePath });
                this._syncWatch(tabId, canonical);
            }
        } finally {
            this._loadingPaths.delete(canonical);
        }
    }

    /** Fetch `filePath`'s current disk content via RPC and apply it to
     *  `tabId`'s view-local buffer + slice state. Shared by the initial-open
     *  path (`_openFileWithMode`) and the live-reload path
     *  (`_handleExternalFileChanged`) — same fetch/sniff/hash/dispatch
     *  sequence either way, just triggered differently.
     *
     *  When `skipIfHashMatches` is given and the freshly-read content hashes
     *  the same, the buffer/dispatch are skipped entirely (no-op reload) —
     *  this is what suppresses a self-triggered echo from our own
     *  `writeeditorfile` save also tripping the fs watcher, and avoids
     *  redundant re-renders on a metadata-only touch.
     *
     *  Returns the new contentHash on success, or null if the load failed,
     *  was refused (binary/oversized), or the tab's path moved out from
     *  under us mid-fetch. */
    private async _loadFileIntoTab(
        tabId: string,
        filePath: string,
        opts?: { skipIfHashMatches?: string },
    ): Promise<string | null> {
        const canonical = canonicalizePath(filePath);
        try {
            const result = await RpcApi.ReadEditorFileCommand(TabRpcClient, {
                path: filePath,
            });
            const content = result?.content ?? "";

            // Re-check the slice: the tab's path may have been swapped out
            // from under us (preview-swap, or the tab closed) while this RPC
            // was in flight. Without this check we'd write THIS file's
            // content into a tab now showing a DIFFERENT file. Validate
            // by canonicalized path because the slice canonicalizes too.
            const tabNow = snapshot(this.blockId)?.tabs.find((t) => t.id === tabId);
            if (!tabNow || tabNow.filePath !== canonical) {
                return null;
            }

            // Refuse content that looks binary or that would freeze the
            // renderer in CodeMirror. Specifically:
            //   1. Any NUL byte → binary (text files don't contain U+0000).
            //   2. Single line over 100K chars → would block the layout
            //      thread laying out a half-megabyte-wide line (real-world
            //      hit: Windows NTUSER.DAT regtrans-ms files come through
            //      as 512KB of one line).
            // Both fast checks; bail before stashing content or hashing.
            const refusal = sniffUnopenable(content);
            if (refusal) {
                dispatch(this.blockId, {
                    type: "TabContentLoadFailed",
                    tabId,
                    error: refusal,
                    source: "system",
                });
                return null;
            }

            const hash = await sha256Hex(content);
            if (opts?.skipIfHashMatches !== undefined && opts.skipIfHashMatches === hash) {
                return hash;
            }

            this._contentByTab.set(tabId, content);
            this._rememberEncoding(tabId, result);
            this._contentVersion[1]((v) => v + 1);

            dispatch(this.blockId, {
                type: "TabContentLoaded",
                tabId,
                contentHash: hash,
                readOnly: result?.read_only ?? false,
                source: "system",
            });
            return hash;
        } catch (e: unknown) {
            const message = e instanceof Error ? e.message : String(e);
            dispatch(this.blockId, {
                type: "TabContentLoadFailed",
                tabId,
                error: message,
                source: "system",
            });
            return null;
        }
    }

    /** Handler for `editor:file_changed` WPS events. Reloads every open tab
     *  in this pane pointing at the changed path — but only if it's clean.
     *  A dirty tab is never auto-clobbered (no dirty-conflict banner exists
     *  yet; see SPEC_EDITOR_LIVE_FILE_RELOAD_2026_07_18.md Phase 3 — until
     *  then a dirty tab simply doesn't reload, same as today's behavior). */
    private async _handleExternalFileChanged(rawPath: string): Promise<void> {
        const canonical = canonicalizePath(rawPath);
        const tabs = snapshot(this.blockId)?.tabs ?? [];
        for (const tab of tabs) {
            if (canonicalizePath(tab.filePath) !== canonical) continue;
            if (tab.dirty) continue;
            if (this._loadingPaths.has(canonical)) continue; // a load is already in flight for this path
            await this._loadFileIntoTab(tab.id, tab.filePath, { skipIfHashMatches: tab.contentHash });
        }
    }

    /** Sync the backend watch registration for `tabId` to `canonicalPath`.
     *  No-op if already watching that exact path (covers repeat loads of an
     *  unchanged tab). Unwatches the previous path first when it differs
     *  (preview-tab file swap: same tabId, new file). */
    private _syncWatch(tabId: string, canonicalPath: string): void {
        const prev = this._watchedPathByTab.get(tabId);
        if (prev === canonicalPath) return;
        if (prev) {
            void RpcApi.UnwatchEditorFileCommand(TabRpcClient, { path: prev, block_id: this.blockId }).catch(() => {});
        }
        this._watchedPathByTab.set(tabId, canonicalPath);
        void RpcApi.WatchEditorFileCommand(TabRpcClient, { path: canonicalPath, block_id: this.blockId }).catch(() => {});
    }

    /** Stop watching whatever path `tabId` was registered under, if any.
     *  Called on tab close and pane dispose. */
    private _unwatchTab(tabId: string): void {
        const prev = this._watchedPathByTab.get(tabId);
        if (!prev) return;
        this._watchedPathByTab.delete(tabId);
        void RpcApi.UnwatchEditorFileCommand(TabRpcClient, { path: prev, block_id: this.blockId }).catch(() => {});
    }

    /** Re-sync the watch registration after an in-app rename (`renameFile`).
     *  The slice's `RenameFile` command updates a tab's `filePath` in place
     *  (same tabId), but that leaves the backend still watching the OLD path
     *  — which no longer exists after the rename — and never registers the
     *  new one, silently breaking live-reload for that tab. Looks up the
     *  tab by its (already-updated) new path and re-points the watch if it
     *  was being watched under the old one. No-op for a tab that never
     *  finished loading (never got a watch registered in the first place). */
    private _resyncWatchAfterRename(oldPath: string, newPath: string): void {
        const oldCanon = canonicalizePath(oldPath);
        const newCanon = canonicalizePath(newPath);
        const tab = snapshot(this.blockId)?.tabs.find((t) => t.filePath === newCanon);
        if (!tab || this._watchedPathByTab.get(tab.id) !== oldCanon) return;
        this._syncWatch(tab.id, newCanon);
    }

    closeTab(tabId: string): void {
        // Phase 1B: force-close even for dirty tabs (matches today's
        // behavior — pane closes silently lose unsaved changes). The
        // dirty-confirm modal lands in a follow-up commit; until then,
        // `force: true` short-circuits the RequestDirtyConfirm path.
        dispatch(this.blockId, { type: "CloseTab", tabId, force: true, source: "user" });
    }

    switchTab(tabId: string): void {
        dispatch(this.blockId, { type: "SwitchTab", tabId, source: "user" });
    }

    reopenLastClosed(): void {
        dispatch(this.blockId, { type: "ReopenLastClosed", source: "user" });
    }

    /** Called by CodeMirror's updateListener on every change. Updates the
     *  view-local content Map and dispatches MarkDirty on the first transition
     *  from clean → dirty. (The slice no-ops MarkDirty on already-dirty.) */
    onContentChange(content: string): void {
        const tabId = this.activeIdAtom();
        if (!tabId) return;
        this._contentByTab.set(tabId, content);
        // No content-version bump here — CodeMirror is the source of truth
        // for the live buffer; contentAtom doesn't need to re-fire on every
        // keystroke (that would defeat CodeMirror's incremental rendering).
        if (!this.dirtyAtom()) {
            dispatch(this.blockId, {
                type: "MarkDirty",
                tabId,
                source: "cm-update",
            });
        }
    }

    /** Capture detected encoding metadata from a read result so save can
     *  round-trip it. Missing fields default to UTF-8 (back-compat). */
    private _rememberEncoding(
        tabId: string,
        result: CommandReadEditorFileResult | undefined | null,
    ): void {
        if (!result) return;
        this._encodingByTab.set(tabId, {
            encoding: result.encoding ?? "UTF-8",
            bom: result.bom ?? "none",
            lineEnding: result.line_ending ?? "lf",
            hadDecodeErrors: result.had_decode_errors ?? false,
        });
    }

    /** True when the file was decoded with U+FFFD replacements (likely the
     *  wrong encoding). Saving would re-encode the lossy buffer over the
     *  original bytes, so callers must refuse the write. */
    private _decodedWithErrors(tabId: string): boolean {
        return this._encodingByTab.get(tabId)?.hadDecodeErrors ?? false;
    }

    /** Encoding fields to spread onto a WriteEditorFileCommand so the file
     *  saves back in its original encoding. Empty (⇒ backend UTF-8 default)
     *  when the tab's encoding is unknown. */
    private _encodingFor(
        tabId: string,
    ): { encoding?: string; bom?: string; line_ending?: string } {
        const e = this._encodingByTab.get(tabId);
        if (!e) return {};
        return { encoding: e.encoding, bom: e.bom, line_ending: e.lineEnding };
    }

    async saveFile(): Promise<void> {
        if (this.readOnlyAtom()) return;
        const tab = this.activeTabAtom();
        if (!tab) return;

        // Refuse to save a file that decoded with replacement characters
        // (likely the wrong encoding) — re-encoding the lossy buffer would
        // overwrite the original bytes (silent data loss), and is the symmetric
        // counterpart to the encode-side lossy-write refusal. Recovery is
        // Reopen-with-Encoding (Phase 3 of SPEC_EDITOR_FILE_ENCODINGS).
        if (this._decodedWithErrors(tab.id)) {
            dispatch(this.blockId, {
                type: "TabContentLoadFailed",
                tabId: tab.id,
                error: "Save blocked: this file didn't decode cleanly (replacement characters present) — likely the wrong encoding. Reopen with the correct encoding before saving to avoid data loss.",
                source: "system",
            });
            return;
        }

        const content = this._contentByTab.get(tab.id) ?? "";

        try {
            await RpcApi.WriteEditorFileCommand(TabRpcClient, {
                path: tab.filePath,
                content,
                ...this._encodingFor(tab.id),
            });
            dispatch(this.blockId, {
                type: "ClearDirty",
                tabId: tab.id,
                source: "system",
            });
            // Recompute content hash since disk now matches buffer.
            const hash = await sha256Hex(content);
            dispatch(this.blockId, {
                type: "TabContentLoaded",
                tabId: tab.id,
                contentHash: hash,
                readOnly: tab.readOnly,
                source: "system",
            });
        } catch (e: unknown) {
            const message = e instanceof Error ? e.message : String(e);
            dispatch(this.blockId, {
                type: "TabContentLoadFailed",
                tabId: tab.id,
                error: `Save failed: ${message}`,
                source: "system",
            });
        }
    }

    // ── Scratch buffer ──────────────────────────────────────────────────

    /** Open (or focus) a scratch buffer. Called from the constructor when
     *  editor:scratch is true and no other file is open. */
    async openScratch(): Promise<void> {
        // Reuse an existing scratch tab if one is already open in this pane.
        const existing = snapshot(this.blockId)?.tabs.find((t) => t.isScratch);
        if (existing) {
            dispatch(this.blockId, { type: "SwitchTab", tabId: existing.id, source: "system" });
            return;
        }
        try {
            // Exclude scratch files already open in any editor pane so that
            // two panes never share the same backing scratch file.
            const excludeIds = getAllActiveScratchIds();
            const result = await RpcApi.CreateScratchFileCommand(TabRpcClient, {
                exclude_scratch_ids: excludeIds.length > 0 ? excludeIds : undefined,
            });
            const events = dispatch(this.blockId, {
                type: "OpenScratch",
                filePath: result.file_path,
                scratchId: result.scratch_id,
                displayName: result.display_name,
                // Plain-text scratch: an untitled buffer is for typing, not a
                // markdown doc. The backing file is `.md` on disk (persistence
                // detail), but treating the tab as markdown made the new
                // styled-preview feature open a fresh scratch as a blank
                // rendered pane. "text" → editable, no syntax-render. The user
                // can Save As `.md` to get markdown behavior.
                language: "text",
                source: "system",
            });
            const opened = events.find((e) => e.type === "TabOpened" || e.type === "TabActivated");
            if (!opened || (opened.type !== "TabOpened" && opened.type !== "TabActivated")) return;
            const tabId = opened.tabId;
            // Read the actual on-disk content — the backend may have returned a
            // reused scratch file that already has content from a prior session.
            // Seeding "" here would clobber that content on the next Ctrl+S.
            const fileResult = await RpcApi.ReadEditorFileCommand(TabRpcClient, { path: result.file_path });
            const content = fileResult?.content ?? "";
            this._contentByTab.set(tabId, content);
            this._rememberEncoding(tabId, fileResult);
            this._contentVersion[1]((v) => v + 1);
            const hash = content === "" ? "" : await sha256Hex(content);
            dispatch(this.blockId, {
                type: "TabContentLoaded",
                tabId,
                contentHash: hash,
                readOnly: false,
                source: "system",
            });
        } catch {
            // Scratch creation failed — editor opens in empty state, user can
            // open a file manually.
        }
    }

    /** Promote the active scratch tab to a real path (Save As). */
    async saveFileAs(destPath: string): Promise<void> {
        const tab = this.activeTabAtom();
        if (!tab?.isScratch || !tab.scratchId) return;
        try {
            // Flush the live in-memory buffer to the scratch file on disk first.
            // MoveScratchFileCommand copies from disk, so without this step any
            // edits typed since the last auto-save would be silently discarded.
            const liveContent = this._contentByTab.get(tab.id);
            if (liveContent !== undefined) {
                await RpcApi.WriteEditorFileCommand(TabRpcClient, {
                    path: tab.filePath,
                    content: liveContent,
                    ...this._encodingFor(tab.id),
                });
            }
            const result = await RpcApi.MoveScratchFileCommand(TabRpcClient, {
                scratch_id: tab.scratchId,
                destination_path: destPath,
            });
            dispatch(this.blockId, {
                type: "PromoteScratch",
                tabId: tab.id,
                newPath: result.file_path,
                source: "user",
            });
            // Clear the cached buffer so openFile reloads from disk and
            // computes a fresh contentHash. Without this, the early-return at
            // openFile:372 (contentLoaded && _contentByTab.has) would skip the
            // hash update, leaving the slice with a stale hash after PromoteScratch.
            this._contentByTab.delete(tab.id);
            await this.openFile(result.file_path);
        } catch (e: unknown) {
            const message = e instanceof Error ? e.message : String(e);
            dispatch(this.blockId, {
                type: "TabContentLoadFailed",
                tabId: tab.id,
                error: `Save As failed: ${message}`,
                source: "system",
            });
        }
    }

    // ── File tree mutations ─────────────────────────────────────────────

    /** Log + surface a file-tree mutation failure as a toast. Every RPC in this
     *  section previously only console.error'd on failure — from the user's
     *  perspective a rejected rename/create/delete looked exactly like a
     *  no-op (the inline input just closes, nothing else happens, no
     *  explanation). This is the one place all four converge so the fix
     *  can't be forgotten on the next handler added here. */
    private reportFileOpError(action: string, e: unknown): void {
        const message = e instanceof Error ? e.message : String(e);
        console.error(`[editor] ${action} failed: ${message}`);
        pushNotification({
            icon: "fa-triangle-exclamation",
            title: `${action} failed`,
            message,
            timestamp: new Date().toISOString(),
            type: "error",
            expiration: Date.now() + 8000,
        });
    }

    /** Rename a file/folder. Updates open tabs (including children for dirs) and refreshes the tree. */
    async renameFile(path: string, newName: string): Promise<void> {
        try {
            const result = await RpcApi.RenameEditorFileCommand(TabRpcClient, {
                old_path: path,
                new_name: newName,
            });
            // Update the exact tab for this path.
            dispatch(this.blockId, {
                type: "RenameFile",
                oldPath: path,
                newPath: result.new_path,
                source: "system",
            });
            this._resyncWatchAfterRename(path, result.new_path);
            // Also update any tabs that are children of a renamed directory.
            // Canonicalize both paths with a trailing sep so we don't match
            // "~/proj2" when renaming "~/proj".
            const oldCanon = canonicalizePath(path);
            const newCanon = canonicalizePath(result.new_path);
            const oldPrefix = oldCanon + "/";
            const newPrefix = newCanon + "/";
            for (const tab of snapshot(this.blockId)?.tabs ?? []) {
                const tabCanon = canonicalizePath(tab.filePath);
                if (tabCanon.startsWith(oldPrefix)) {
                    const newTabPath = newPrefix + tabCanon.slice(oldPrefix.length);
                    dispatch(this.blockId, { type: "RenameFile", oldPath: tab.filePath, newPath: newTabPath, source: "system" });
                    this._resyncWatchAfterRename(tab.filePath, newTabPath);
                }
            }
            // Refresh the parent directory.
            const parentPath = path.replace(/[/\\][^/\\]*$/, "") || path;
            void this.treeModel.refreshPath(parentPath);
        } catch (e: unknown) {
            this.reportFileOpError("Rename", e);
        }
    }

    /** Delete a file or folder, closing any open tabs that were pointing to it.
     *
     *  When `confirmNeeded` is provided, the caller is responsible for showing
     *  a confirmation UI. It receives a `proceed` callback — call it to execute
     *  the delete; not calling it aborts. When omitted, falls back to
     *  `window.confirm` (for programmatic or non-UI callers). */
    async deleteFile(
        path: string,
        recursive: boolean,
        confirmNeeded?: (proceed: () => void) => void,
    ): Promise<void> {
        const label = recursive ? "folder and all its contents" : "file";
        const name = path.split(/[/\\]/).pop() ?? path;

        const doDelete = async () => {
            try {
                await RpcApi.DeleteEditorFileCommand(TabRpcClient, { path, recursive });
                const canon = canonicalizePath(path);
                for (const tab of snapshot(this.blockId)?.tabs ?? []) {
                    const tabCanon = canonicalizePath(tab.filePath);
                    if (tabCanon === canon || tabCanon.startsWith(canon + "/") || tabCanon.startsWith(canon + "\\")) {
                        dispatch(this.blockId, { type: "CloseTab", tabId: tab.id, force: true, source: "system" });
                    }
                }
                const parentPath = path.replace(/[/\\][^/\\]*$/, "") || path;
                const entryName = path.split(/[/\\]/).pop() ?? "";
                this.treeModel.removeEntryOptimistic(parentPath, entryName);
            } catch (e: unknown) {
                this.reportFileOpError("Delete", e);
            }
        };

        if (confirmNeeded) {
            confirmNeeded(() => void doDelete());
        } else {
            if (!window.confirm(`Delete ${name} (${label})? This cannot be undone.`)) return;
            await doDelete();
        }
    }

    /** Create an empty file in the tree. Opens it as a preview tab on success. */
    async createFile(parentPath: string, name: string): Promise<void> {
        try {
            const result = await RpcApi.CreateEditorFileCommand(TabRpcClient, { parent_path: parentPath, name });
            this.treeModel.addEntryOptimistic(parentPath, { name, is_dir: false, is_symlink: false });
            await this.openFilePreview(result.file_path);
        } catch (e: unknown) {
            this.reportFileOpError("Create file", e);
            throw e;
        }
    }

    /** Create a directory in the tree. Ensures it's visible. */
    async createDir(parentPath: string, name: string): Promise<void> {
        try {
            await RpcApi.CreateEditorDirCommand(TabRpcClient, { parent_path: parentPath, name });
            this.treeModel.addEntryOptimistic(parentPath, { name, is_dir: true, is_symlink: false });
        } catch (e: unknown) {
            this.reportFileOpError("Create folder", e);
            throw e;
        }
    }

    /** Reveal a path in the OS file manager. */
    async revealInExplorer(path: string): Promise<void> {
        try {
            await RpcApi.OpenInShellCommand(TabRpcClient, { path });
        } catch {
            // Non-fatal — some platforms may not support this.
        }
    }

    /** Open the folder in a terminal pane (split right of the current pane). */
    async openInTerminal(folderPath: string): Promise<void> {
        try {
            // pane.open is a raw RPC — no typed wrapper exists yet in RpcApi.
            await TabRpcClient.rpcCall("pane.open", {
                view: "term",
                cwd: folderPath,
                split_direction: "right",
                split_reference_block_id: this.blockId,
            }, {});
        } catch {
            // pane.open might not be registered yet — fail silently.
        }
    }

    /** Open a file in a new editor pane, split right of the current one.
     *  Same pane.open mechanism as openInTerminal above, just view:"editor" +
     *  file instead of view:"term" + cwd. See
     *  docs/specs/SPEC_EDITOR_FILE_TREE_OPEN_ACTIONS_2026_07_12.md. */
    async openToTheSide(filePath: string): Promise<void> {
        try {
            await TabRpcClient.rpcCall("pane.open", {
                view: "editor",
                file: filePath,
                split_direction: "right",
                split_reference_block_id: this.blockId,
            }, {});
        } catch {
            // pane.open might not be registered yet — fail silently, same as openInTerminal.
        }
    }

    /** Open a file in a new editor pane inside a brand-new, otherwise-empty
     *  app tab. Deliberately bypasses createTab() in store/global.ts, which
     *  always layers on the agent/sysinfo/swarm default preset — wrong here,
     *  the user asked for THIS file, not a fresh default workspace.
     *
     *  Three things every other CreateTab caller in this codebase gets for
     *  free via applyTabPreset() (tab-presets.ts), which this method can't
     *  call directly since it always adds the default widget set — so each
     *  is replicated individually here:
     *
     *  1. waitForLayoutModel(tabId) before touching the new tab at all. The
     *     tab's WaveObj + LayoutState propagate via subscription some time
     *     after CreateTab returns, not synchronously with it.
     *  2. createBlockOnModel(...) — NOT the pane.open RPC used for
     *     openToTheSide above. Confirmed live: pane.open against a freshly
     *     created tab_id succeeds server-side with zero errors ("block
     *     created + layout updated" in the srv log) and the block STILL
     *     never renders — the tab's client-side layoutModel, even once
     *     waitForLayoutModel() confirms the object exists, isn't yet
     *     subscribed to receive the backend's layout:update broadcast for
     *     that specific brand-new tab. createBlockOnModel goes through
     *     ObjectService.CreateBlock + layoutModel.treeReducer(...) directly
     *     — the same client-side path applyTabPreset uses for every preset
     *     widget — which sidesteps the gap entirely. The constructed
     *     BlockDef mirrors exactly what the backend's build_pane_meta
     *     (agentmux-srv/src/server/app_api/pane.rs) would have produced for
     *     `pane.open { view: "editor", file }` on the openToTheSide path.
     *  3. Explicit setActiveTab() AFTER the block exists. Belt-and-suspenders
     *     alongside CreateTab's own `activate=true` (3rd arg below): the
     *     reducer itself only auto-activates a workspace's very FIRST tab
     *     ever (see create_tab_second_tab_does_not_steal_active in
     *     reducer.rs), but the service layer already compensates —
     *     `agentmux-srv/src/server/service/workspace.rs`'s CreateTab handler
     *     dispatches a follow-up SetActiveTab whenever `activate=true` and
     *     the reducer didn't auto-activate, so this call is redundant in
     *     practice, not a workaround for a live bug (#2155's "activate arg
     *     silently dropped" half was already fixed server-side by the time
     *     it was filed). Kept anyway: harmless, and it's still what makes
     *     the destination tab already have its content when the switch's
     *     reveal-gate opens — avoids a flash of an empty tab. */
    async openInNewTab(filePath: string): Promise<void> {
        const ws = workspace();
        if (!ws) return;
        try {
            const tabId = await WorkspaceService.CreateTab(ws.oid, "", true, false);
            const layoutModel = await waitForLayoutModel(tabId);
            if (!layoutModel) return; // Tab never propagated — nothing safe to do.
            const isMarkdown = filePath.toLowerCase().endsWith(".md");
            await createBlockOnModel(
                tabId,
                layoutModel,
                {
                    meta: {
                        view: "editor",
                        file: filePath,
                        ...(isMarkdown ? { "editor:tree_expanded": false } : {}),
                    },
                },
                null,
                null,
            );
            await setActiveTab(tabId);
        } catch {
            // Fail silently, consistent with the file tree's other pane.open callers.
        }
    }

    // ── Tree state (unchanged from Phase 1A of file-tree spec) ─────────
    async toggleTreeExpanded(): Promise<void> {
        const next = !this._treeExpanded[0]();
        this._treeExpanded[1](next);
        await this.persistMeta({ [META_TREE_EXPANDED]: next });
    }

    async toggleShowHidden(): Promise<void> {
        const next = !this._showHidden[0]();
        this._showHidden[1](next);
        await this.persistMeta({ [META_SHOW_HIDDEN]: next });
    }

    setTreeWidth(width: number): void {
        this._treeWidth[1](clampTreeWidth(width));
    }

    async commitTreeWidth(): Promise<void> {
        await this.persistMeta({ [META_TREE_WIDTH]: this._treeWidth[0]() });
    }

    editorMode(): EditorMode {
        void this._tabModesVersion[0](); // reactive dependency
        const tabId = this.activeIdAtom();
        if (!tabId) return "source";
        const stored = this._tabModes.get(tabId);
        if (stored !== undefined) return stored;
        return this.activeTabAtom()?.language === "markdown" ? "preview" : "source";
    }

    setEditorMode(mode: EditorMode): void {
        const tabId = this.activeIdAtom();
        if (!tabId) return;
        this._tabModes.set(tabId, mode);
        this._tabModesVersion[1]((v) => v + 1);
    }

    toggleEditorMode(): void {
        this.setEditorMode(this.editorMode() === "preview" ? "source" : "preview");
    }

    setPreviewHeight(height: number): void {
        this._previewHeight[1](clampPreviewHeight(height));
    }

    async commitPreviewHeight(): Promise<void> {
        await this.persistMeta({ [META_PREVIEW_HEIGHT]: this._previewHeight[0]() });
    }

    private async persistMeta(meta: Record<string, unknown>): Promise<void> {
        try {
            await RpcApi.SetMetaCommand(TabRpcClient, {
                oref: makeORef("block", this.blockId),
                meta,
            });
        } catch {
            // Persistence failure isn't fatal — in-memory signal still drives
            // the current pane's behavior. On reopen, the previous persisted
            // value (or default) wins.
        }
    }

    giveFocus(): boolean {
        return false;
    }

    dispose(): void {
        this._unsubFileChanged();
        for (const tabId of [...this._watchedPathByTab.keys()]) this._unwatchTab(tabId);
        _instanceHandlers.delete(this._globalHandler);
        this._eventSubscribers.clear();
        this._contentByTab.clear();
        this._encodingByTab.clear();
        unregisterEditorPane(this.blockId);
    }
}

function clampTreeWidth(w: number): number {
    if (!Number.isFinite(w)) return TREE_WIDTH_DEFAULT;
    return Math.max(TREE_WIDTH_MIN, Math.min(TREE_WIDTH_MAX, Math.round(w)));
}

function clampPreviewHeight(h: number): number {
    if (!Number.isFinite(h)) return PREVIEW_HEIGHT_DEFAULT;
    return Math.max(PREVIEW_HEIGHT_MIN, Math.min(PREVIEW_HEIGHT_MAX, Math.round(h)));
}

/** Sniff content that's unsafe to hand to CodeMirror. Returns a human-readable
 *  refusal message (used as the tab's loadError) or null when the content is
 *  fine to render. Cheap — scans up to the first 4096 bytes for NUL.
 *
 *  Why this lives in the model: CodeMirror will faithfully try to lay out
 *  half-a-megabyte of binary on one line (the trigger case was a Windows
 *  NTUSER.DAT regtrans-ms file at 512KB) and block the renderer indefinitely.
 *  The 10MB read cap doesn't catch it because the file is small. */
const MAX_BYTES_FOR_NUL_SNIFF = 4096;
const MAX_SINGLE_LINE_CHARS = 100_000;

function sniffUnopenable(content: string): string | null {
    // NUL-byte sniff — any U+0000 in the first few KB means binary.
    const sniffLen = Math.min(content.length, MAX_BYTES_FOR_NUL_SNIFF);
    for (let i = 0; i < sniffLen; i++) {
        if (content.charCodeAt(i) === 0) {
            return "This file looks binary (contains NUL bytes) and can't be displayed in the editor.";
        }
    }
    // Single-line-too-long sniff — even a "text" file is unsafe to render if
    // it has no newline within the first 100K chars. CodeMirror's layout
    // engine handles long files line-by-line; a single 100K+ line stalls it.
    const firstNewline = content.indexOf("\n");
    const lineLen = firstNewline === -1 ? content.length : firstNewline;
    if (lineLen > MAX_SINGLE_LINE_CHARS) {
        return `This file's first line is ${lineLen.toLocaleString()} characters long — too wide to render. Open it in an external editor.`;
    }
    return null;
}

// ── Language detection ──────────────────────────────────────────────────────
// Maps file extensions to the canonical names the rest of the editor expects:
// `loadLanguage` in editor-view.tsx, LSP_SUPPORTED_LANGUAGES in install-hints.ts.
// The slice's `deriveLanguage` returns raw extensions ("ts", "py") — fine as a
// fallback label, but downstream lookups need the mapped form ("typescript",
// "python"). Keep this map in sync with `loadLanguage`'s switch.

const EXTENSION_MAP: Record<string, string> = {
    ".ts": "typescript",
    ".tsx": "typescript",
    ".js": "javascript",
    ".jsx": "javascript",
    ".mjs": "javascript",
    ".cjs": "javascript",
    ".py": "python",
    ".rs": "rust",
    ".html": "html",
    ".htm": "html",
    ".css": "css",
    ".scss": "css",
    ".json": "json",
    ".md": "markdown",
    ".markdown": "markdown",
    ".yaml": "yaml",
    ".yml": "yaml",
    ".toml": "toml",
    ".sh": "shell",
    ".bash": "shell",
    ".zsh": "shell",
    ".ps1": "powershell",
    ".sql": "sql",
    ".go": "go",
    ".java": "java",
    ".c": "c",
    ".cpp": "cpp",
    ".h": "c",
    ".hpp": "cpp",
    ".xml": "html",
    ".svg": "html",
};

function detectLanguage(filePath: string): string {
    const lower = filePath.toLowerCase();
    // Extension lookup. Use endsWith so multi-segment extensions like
    // `.test.tsx` still resolve via the trailing component.
    for (const [ext, lang] of Object.entries(EXTENSION_MAP)) {
        if (lower.endsWith(ext)) return lang;
    }
    // Special bareword filenames (no extension or unusual conventions).
    const name = lower.split(/[\\/]/).pop() ?? "";
    if (name === "dockerfile") return "shell";
    if (name === "makefile") return "shell";
    if (name === "cargo.toml" || name === "cargo.lock") return "toml";
    return "text";
}
