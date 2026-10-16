// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// NOTE: Editor is a pane-level view for editing files with syntax highlighting.
// It is NOT a standalone IDE — complex editing happens in the agent's terminal
// or via agent tool calls. This covers quick edits, file viewing, and diffing.
//
// State management: slice #10 editor-pane-state (frontend/app/store/
// editor-pane-state-store.ts) owns the tab list, active id, dirty flags, and
// recently-closed stack, on the shared document-tab model
// (docs/specs/SPEC_DOCUMENT_TABS_2026_10_02.md §6.1). This file is a thin
// projection layer between that slice and the editor view, and keeps the
// pane's tabs in its block's `doctabs` record. Tab-content blobs are held in a view-local Map
// (this._contentByTab) — content is deliberately NOT in the slice (large,
// not auditable, not persistable cheaply). The slice tracks contentHash +
// contentLoaded so it can reason about dirty-vs-disk without holding the
// buffer.
//
// Tab behaviour: SPEC_DOCUMENT_TABS_2026_10_02.md, the reducer in
// frontend/app/store/editor-pane-state-store.ts and its tests. Earlier specs: SPEC_EDITOR_FILE_TREE_2026-05-26.md, SPEC_EDITOR_LSP_AND_THEMES_2026-05-26.md.

import type { PaneTabHostContext } from "@/app/block/pane-tab-registry";
import type { EditorView } from "codemirror";
import { pushNotification, useBlockAtom } from "@/app/store/global";
import {
    EditorPaneEvent,
    EditorTab,
    canonicalizePath,
    deserializeEditorBuffer,
    dispatch,
    registerEditorPane,
    serializeEditorBuffer,
    setEventSink,
    snapshot,
    unregisterEditorPane,
    getAllActiveScratchIds,
} from "@/app/store/editor-pane-state-store";
import { DOC_TABS_META, hydrateDocTabs } from "@/app/doc-tabs/doc-tabs";
import { DocTabsSaver } from "@/app/doc-tabs/doc-tabs-controller";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { makeORef } from "@/app/store/mos";
import { onMuxEvent } from "@/app/store/mps";
import { WpsEvent } from "@/app/store/mps-events";
import { fireAndForget } from "@/util/util";
import { createEffect, createMemo, createRoot, createSignal, type Accessor } from "solid-js";
import { FileTreeModel } from "./file-tree-model";
import type { CommandReadEditorFileResult } from "@/app/store/rpc-api";
import { registerFileDropTarget } from "@/app/drag/file-drop";
import { notifyDrop } from "@/app/drag/file-drop-actions";
import { createEditorDropHook, openEmptyScratch } from "./editor-drop";
import { openPendingFiles, pendingEntryKey } from "./pending-open-files";
import { detectLanguage } from "./editor-language";
import { sniffUnopenable } from "./editor-sniff";
import { openFileInNewTab } from "./editor-open";
import { applyMetaPatch, migrateMediaMeta } from "./media-migration";
import { chooseViewer, isBinaryProbe, isViewerId, viewerForDropped, type FileProbe, type ViewerId } from "./viewers/viewer-choice";
import { droppedFileOf, isDroppedPath, rememberDropped, ViewerTabs } from "./viewers/viewer-tabs";
import { showBlockWithoutFocus } from "@/app/util/reveal-block";
import { readZoom } from "@/app/store/zoom-factor";
import { isSshConnection } from "@/app/view/term/ssh-connection";
import { META_OPEN_AT_LINE, readOpenAtLine } from "./open-at-line";
import { openRemoteFile } from "./open-from-remote";

const META_TREE_EXPANDED = "editor:tree_expanded";
const META_SHOW_HIDDEN = "editor:show_hidden";
const META_TREE_WIDTH = "editor:tree_width";
const META_PREVIEW_HEIGHT = "editor:preview_height";
const META_WORD_WRAP = "editor:word_wrap";
const META_LEGACY_FILE = "file";
// Reuse (SPEC_EDITOR_MCP_OPEN_BLANK_PREVIEW_AND_PANE_REUSE_2026_08_03.md
// Part 2): files pushed by reuse-and-not-yet-mounted OpenEditor calls.
// Drained (all entries, in order) once at construction, then cleared —
// deliberately NOT delivered via MPS event replay, which has no ack/consume
// concept and would otherwise re-fire on every future reconnect (codex P1
// on PR #2404). An array, not a single scalar, so 2+ calls stacking up
// before the pane mounts don't overwrite/lose each other (codex P1, second
// finding on the same PR).
const META_PENDING_OPEN_FILES = "editor:pending_open_files";
// The initial `file` opens following newer files in its folder (a media
// open, SPEC_EDITOR_REVAMP_UNIFIED_VIEWERS_2026_10_15.md §4.9). Cleared once used.
const META_FOLLOW_FOLDER = "editor:follow_folder";

export type EditorMode = "preview" | "source" | "split";

/** How a file opens: following newer files in its folder (a media open),
 *  in a given viewer (OpenEditor's `viewer`). */
export interface OpenFileOpts {
    followFolder?: boolean;
    viewer?: string;
}
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

/** The editor's state behind its native pane tab (`editorPaneTab`, editor.tsx). */
export class EditorViewModel {
    viewType = "editor";
    blockId: string;

    // Populated by editor-view.tsx whenever it (re)builds the CodeMirror
    // instance — mirrors AgentViewModel.focusTargetRef / TermViewModel.termRef.
    // giveFocus() below is a no-op until the first build completes.
    cmViewRef: { current: EditorView | null } = { current: null };

    viewName: Accessor<string>;
    viewText: Accessor<string | HeaderElem[]>;


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
    readonly _contentByTab = new Map<string, string>();
    // Bump-counter signal so contentAtom re-evaluates when we mutate the Map.
    private _contentVersion = createSignal<number>(0);
    // Detected text encoding per tab (SPEC_EDITOR_FILE_ENCODINGS), captured on
    // read so save round-trips the original encoding/bom/line-ending instead of
    // silently rewriting the file as UTF-8.
    readonly _encodingByTab = new Map<
        string,
        { encoding: string; bom: string; lineEnding: string; hadDecodeErrors: boolean }
    >();

    // ── Tree state (per-pane, persisted in block meta) ──────────────────
    private _treeExpanded = createSignal<boolean>(true);
    /** The file tree lists this computer's folders: hidden in an editor on
     *  an SSH host (its files open from Hangar there). */
    treeExpandedAtom: Accessor<boolean> = () => this._treeExpanded[0]() && !this.connection();

    private _showHidden = createSignal<boolean>(false);
    showHiddenAtom: Accessor<boolean> = this._showHidden[0];

    private _treeWidth = createSignal<number>(TREE_WIDTH_DEFAULT);
    treeWidthAtom: Accessor<number> = this._treeWidth[0];

    private _previewHeight = createSignal<number>(PREVIEW_HEIGHT_DEFAULT);
    previewHeightAtom: Accessor<number> = this._previewHeight[0];

    // Word wrap defaults ON (matches the editor's long-standing hardcoded
    // behavior before this setting existed — see editor-view.tsx's
    // wordWrapCompartment) so existing panes see no behavior change until
    // the user explicitly turns it off via the right-click menu.
    private _wordWrap = createSignal<boolean>(true);
    wordWrapAtom: Accessor<boolean> = this._wordWrap[0];
    /** "Open from remote…" is showing its picker (open-from-remote-modal.tsx). */
    private _openFromRemote = createSignal<boolean>(false);
    openFromRemoteAtom: Accessor<boolean> = this._openFromRemote[0];

    // Probes and revisions of the tabs' files (SPEC_EDITOR_REVAMP_UNIFIED_VIEWERS_2026_10_15.md §4.1).
    readonly viewerTabs = new ViewerTabs();

    // Per-tab editor mode: "preview" | "source" | "split".
    // Not persisted — tabs return to their language-appropriate default on reopen.
    readonly _tabModes = new Map<string, EditorMode>();
    readonly _tabModesVersion = createSignal<number>(0);

    treeModel = new FileTreeModel();

    /** The block's meta — the host context's, reactive. */
    meta: Accessor<MetaType | undefined>;
    private ctx: PaneTabHostContext;
    zoomAtom!: Accessor<number>;

    /** The line pane.open asked the initial file to open at, until the view
     *  has moved the cursor there (takeOpenAtLine). */
    private _openAtLine: { path: string; line: number } | null = null;
    /** Tabs that started loading via openFile — used so concurrent dispatches
     *  for the same path don't double-fetch. */
    private _loadingPaths = new Set<string>();
    private _disposeFileDrop: () => void = () => {};

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
    private _disposePendingOpenFilesEffect: () => void = () => {};

    // Document-tab persistence (SPEC_DOCUMENT_TABS §5.3). With no tabs left,
    // the pre-tabs `file` key goes too, or the pane would reopen that file.
    private readonly _docTabsSaver = new DocTabsSaver(
        () => snapshot(this.blockId)?.doc,
        serializeEditorBuffer,
        (record) => void this.persistMeta(record ? { [DOC_TABS_META]: record } : { [DOC_TABS_META]: null, [META_LEGACY_FILE]: null })
    );

    /** Set by the view: ask before closing a tab with unsaved changes;
     *  `discard` closes it anyway. Without it, such a close is refused. */
    confirmDirtyClose: ((tab: EditorTab, discard: () => void) => void) | null = null;

    // A native pane tab (Pane Tab contract Phase 2c): built by `create(ctx)`
    // (editor.tsx); its own block's meta comes from, and goes to, the host
    // context.
    constructor(ctx: PaneTabHostContext) {
        const blockId = ctx.blockId;
        this.ctx = ctx;
        this.blockId = blockId;

        this.meta = ctx.meta;

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
            this._docTabsSaver.schedule();
            // A tab shown for the first time since a restart, or reopened,
            // reads its file now (one being opened is already reading).
            if (events.some((e) => e.type === "TabActivated" || e.type === "TabOpened" || e.type === "TabsRestored")) {
                queueMicrotask(() => this._ensureActiveLoaded());
            }
            for (const ev of events) {
                if (ev.type === "TabClosed") {
                    this._contentByTab.delete(ev.tabId);
                    this._encodingByTab.delete(ev.tabId);
                    this._tabModes.delete(ev.tabId);
                    this.viewerTabs.forget(ev.tabId);
                    this._unwatchTab(ev.tabId);
                }
                for (const sub of this._eventSubscribers) sub(ev);
            }
        };
        _instanceHandlers.add(this._globalHandler);

        // Live-reload: one subscription per pane, scoped to this block.
        // Fires when a path open in one of this pane's tabs changes on disk.
        this._unsubFileChanged = onMuxEvent(WpsEvent.EditorFileChanged, (data) => {
                if (data.path) void this._handleExternalFileChanged(data.path);
            }, makeORef("block", blockId));

        // Pane-reuse (SPEC_EDITOR_MCP_OPEN_BLANK_PREVIEW_AND_PANE_REUSE_2026_08_03.md
        // Part 2): reactively drains META_PENDING_OPEN_FILES whenever the
        // backend writes to it — a file an OpenEditor reuse call pushed into
        // this pane instead of creating a second Editor pane. Reactive
        // (createEffect over the block's meta), not a one-shot construction-time
        // check, so it uniformly covers BOTH "this pane wasn't mounted yet
        // when the backend wrote the request" and "already mounted, backend
        // writes it later" through the same MuxObj sync path this pane
        // already reactively depends on for everything else.
        //
        // An earlier version used a SEPARATE live MPS event (fired directly
        // alongside the meta write) for the already-mounted case, racing
        // this same meta update. Reagent (PR #2404) found the race: the MPS
        // event is a direct WS push and arrives essentially synchronously,
        // while the meta write reaches `meta()` only after an async
        // MuxObj DB-refetch — so the live event's handler could run and try
        // to dequeue its own path from `meta()` BEFORE that same
        // write had actually landed there, reading stale data and no-op'ing,
        // stranding the entry to be wrongly reprocessed on a later remount.
        // Removing the separate live path entirely (rather than patching the
        // race) leaves one delivery mechanism and one reactive consumer —
        // no ordering between two paths to get wrong. Costs a small amount
        // of latency for the already-mounted case (a real MuxObj round-trip
        // instead of a direct push) in exchange for not being racy.
        createRoot((dispose) => {
            this._disposePendingOpenFilesEffect = dispose;
            createEffect(() => {
                const pending = this.meta()?.[META_PENDING_OPEN_FILES];
                if (!Array.isArray(pending) || pending.length === 0) return;
                // Show the pane too (restore / stack tab / un-magnify), but
                // don't switch window tabs or take the user's caret.
                const processed = openPendingFiles(
                    pending,
                    (path, opts) => void this.openFile(path, opts),
                    () => void showBlockWithoutFocus(this.blockId)
                );
                const done = new Set(processed.map(pendingEntryKey));
                fireAndForget(() => {
                    // Re-read fresh (not the `pending` snapshot above) right
                    // before writing, and remove only the entries THIS run
                    // processed, rather than blind-clearing to `[]` — reagent
                    // P1: an unconditional clear could overwrite a file the
                    // backend appended between this effect's read and this
                    // write landing, silently dropping it (the same class of
                    // lost-update bug already fixed twice in this PR at
                    // different points, reappearing here). This narrows the
                    // race to the network round-trip of this one write,
                    // rather than eliminating it outright — a fully atomic
                    // fix needs a backend remove-these-entries command
                    // running under the reducer's single-writer lock, not a
                    // client-side read-filter-write; deferred given how
                    // narrow the remaining window is (needs a third
                    // concurrent reuse call specifically inside it).
                    const current = this.meta()?.[META_PENDING_OPEN_FILES];
                    const remaining = Array.isArray(current)
                        ? current.filter((p) => !done.has(pendingEntryKey(p)))
                        : [];
                    return this.persistMeta({ [META_PENDING_OPEN_FILES]: remaining });
                });
            });
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
        // MEMOIZED, deliberately — a bare arrow function here caused the
        // editor to silently DISCARD the first character typed into any file.
        //
        // As a plain accessor, every caller that read `loadingAtom()`
        // subscribed to `activeTabAtom()` — the whole tab OBJECT — rather than
        // to this boolean. `editor-view.tsx`'s "rebuild CodeMirror on tab
        // change" effect reads it, so ANY mutation of the active tab re-ran
        // that effect, even when `loading` was false both before and after.
        //
        // The first edit does exactly that: `onContentChange()` dispatches
        // MarkDirty — guarded by `if (!this.dirtyAtom())`, so it fires only on
        // the FIRST edit — the tab object changes, the effect re-runs, and
        // CodeMirror is rebuilt from the on-disk content, throwing away the
        // character just typed along with the destroyed view. That is why the
        // symptom was "the first keystroke never appears, but the dirty marker
        // does": the model was told about the edit, the view was replaced.
        //
        // Confirmed in a live trace on CEF 152, to the millisecond:
        //   15:13:01.642  docChanged  {docLen: 1}   <- keystroke lands
        //   15:13:01.643  setupEditor {docLen: 0}   <- rebuilt from disk
        //
        // `createMemo` only propagates when the computed value actually
        // changes, so tab mutations that leave `loading` alone no longer reach
        // the effect. See
        // docs/specs/SPEC_EDITOR_FIRST_KEYSTROKE_NOT_RENDERED_2026_09_15.md.
        this.loadingAtom = useBlockAtom(blockId, "editor-tabs-loading", () =>
            createMemo<boolean>(() => {
                const tab = this.activeTabAtom();
                return tab != null && !tab.contentLoaded && tab.loadError == null;
            }),
        );

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
            createMemo<number>(() => readZoom(this.meta())),
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

        this.viewText = () => [];

        // A saved Media pane opens as an Editor (the `media` alias): its meta,
        // rewritten into an Editor's, is what it restores from, and is written
        // back once (SPEC_EDITOR_REVAMP_UNIFIED_VIEWERS_2026_10_15.md §4.12.2).
        const rawMeta = this.meta() as Record<string, unknown> | undefined;
        const mediaPatch = migrateMediaMeta(rawMeta);
        if (mediaPatch) queueMicrotask(() => void this.persistMeta(mediaPatch));

        // Restore persisted tree state from block meta.
        const meta = (mediaPatch ? applyMetaPatch(rawMeta, mediaPatch) : rawMeta) as MetaType | undefined;
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
        if (meta?.[META_WORD_WRAP] === false) {
            this._wordWrap[1](false);
        }

        // The pane's tabs, from its block's `doctabs` record. A pane from
        // before document tabs has only `file` (the pre-tabs key): it opens
        // with that one file.
        const restored = hydrateDocTabs(meta?.[DOC_TABS_META], deserializeEditorBuffer);
        const legacyFile = meta?.[META_LEGACY_FILE];
        // A line to open `file` at, once (pane.open's `line`). Kept in the
        // meta until the loaded file uses it (takeOpenAtLine), so an editor
        // unmounted mid-load still lands there when it comes back.
        const openAt = readOpenAtLine(meta, legacyFile);
        if (openAt != null && typeof legacyFile === "string") {
            this._openAtLine = { path: canonicalizePath(legacyFile), line: openAt };
        }
        if (restored) {
            this._docTabsSaver.holds(meta?.[DOC_TABS_META]);
            dispatch(blockId, { type: "RestoreDocTabs", doc: restored, source: "hydrate" });
        } else if (typeof legacyFile === "string" && legacyFile) {
            const followFolder = meta?.[META_FOLLOW_FOLDER] === true;
            void this.openFile(legacyFile, { followFolder });
            if (followFolder) void this.persistMeta({ [META_FOLLOW_FOLDER]: null });
        } else if (meta?.[META_SCRATCH] === true && snapshot(blockId)?.tabs.length === 0) {
            // Widget default: open a scratch buffer when no file was persisted.
            void this.openScratch();
        }

        // Files dropped onto the pane (SPEC_DRAG_AND_DROP_CONSOLIDATION §5.3).
        this._disposeFileDrop = registerFileDropTarget(
            blockId,
            createEditorDropHook({
                // A file from this computer isn't on the host an editor there
                // reads from, and dropped text would become a local scratch.
                openFile: async (path) => {
                    if (this.connection()) return notifyDrop.cantOpen(path, "editor");
                    await this.openFile(path);
                },
                openText: async (name, content) => {
                    if (this.connection()) return notifyDrop.cantOpen(name, "editor");
                    await this.openDroppedText(name, content);
                },
                openBytes: async (file) => {
                    if (this.connection()) return notifyDrop.cantOpen(file.name || "file", "editor");
                    await this.openDroppedFile(file);
                },
                cantOpen: (name) => notifyDrop.cantOpen(name, "editor"),
            }),
        );

        // Reuse: pending files are handled by the reactive createEffect
        // above (covers this initial-mount case too — no separate one-shot
        // check needed here, and having both would double-open).
    }

    /** The line to put the cursor on now that `path`'s content is in the
     *  editor, once; undefined for any other file or while it is loading. */
    takeOpenAtLine(path: string | undefined): number | undefined {
        const pending = this._openAtLine;
        if (!pending || !path || this.loadingAtom() || canonicalizePath(path) !== pending.path) return undefined;
        this._openAtLine = null;
        // Used: a reload of the pane opens where the user left it instead.
        void this.persistMeta({ [META_OPEN_AT_LINE]: null });
        return pending.line;
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
    async openFile(filePath: string, opts?: OpenFileOpts): Promise<void> {
        return this._openFileWithMode(filePath, "pinned", opts);
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

    private async _openFileWithMode(filePath: string, mode: "preview" | "pinned", opts?: OpenFileOpts): Promise<void> {
        const canonical = canonicalizePath(filePath);
        // If a load is already in flight for this path, we still need to
        // honor a preview→pinned transition (rapid dbl-click on a tree
        // row: the single-click added canonical to _loadingPaths, then
        // the dblclick's pinned-open would early-return and never pin).
        // Dispatch OpenFile so the slice runs the pin-if-existing logic,
        // then bail (the RPC's continuation will populate content as
        // usual).
        if (this._loadingPaths.has(canonical)) {
            const events = dispatch(this.blockId, {
                type: "OpenFile",
                path: filePath,
                language: detectLanguage(filePath),
                mode,
                source: "user",
            });
            // A media open while the file is still loading still follows.
            const hit = events.find((e) => e.type === "TabOpened" || e.type === "TabActivated");
            if (opts?.followFolder && hit && (hit.type === "TabOpened" || hit.type === "TabActivated")) {
                dispatch(this.blockId, { type: "SetFollowFolder", tabId: hit.tabId, follow: true, source: "system" });
            }
            return;
        }
        this._loadingPaths.add(canonical);
        const followFolder = opts?.followFolder === true;

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
        if (followFolder) dispatch(this.blockId, { type: "SetFollowFolder", tabId, follow: true, source: "system" });
        if (opts?.viewer && isViewerId(opts.viewer)) {
            const tab = snapshot(this.blockId)?.tabs.find((t) => t.id === tabId);
            if (tab?.viewerOverride !== opts.viewer) {
                dispatch(this.blockId, { type: "SetTabViewer", tabId, viewer: opts.viewer, override: true, source: "system" });
                this._contentByTab.delete(tabId);
            }
        }

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
                // editor:tabs persistence. Not a dropped file's: it is gone
                // after a restart.
                if (!isDroppedPath(canonical)) await this.persistMeta({ [META_LEGACY_FILE]: filePath });
                this._syncWatch(tabId, canonical);
            }
        } finally {
            this._loadingPaths.delete(canonical);
        }
    }

    /** The SSH connection this editor's files are on (block meta
     *  `connection`, set when Hangar opens a file on a host); "" for this
     *  computer. Remote terminals spec §6.3. */
    connection(): string {
        const c = this.meta?.()?.["connection"];
        return isSshConnection(c) ? String(c).trim() : "";
    }

    /** Where a request for `tabId`'s file runs: the host, and this pane for
     *  ssh's prompts; a scratch tab's file is always on this computer. */
    private on(tabId?: string): { connection?: string; block_id?: string } {
        const c = this.connection();
        const scratch = tabId != null && snapshot(this.blockId)?.tabs.find((t) => t.id === tabId)?.isScratch;
        return c && !scratch ? { connection: c, block_id: this.blockId } : {};
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
            // Probe before reading (§4.2): one stat and 8 KB on srv decide the
            // viewer, so a binary file is never downloaded into CodeMirror.
            // This computer's files only; a scratch buffer is always text.
            const tabAtStart = snapshot(this.blockId)?.tabs.find((t) => t.id === tabId);
            if (isDroppedPath(canonical)) return this._loadDroppedIntoTab(tabId, canonical, opts);
            let allowBinaryText = false;
            if (!this.on(tabId).connection && !tabAtStart?.isScratch) {
                const probe = await RpcApi.FileProbeCommand(TabRpcClient, { path: filePath });
                const tabNow = snapshot(this.blockId)?.tabs.find((t) => t.id === tabId);
                if (!tabNow || tabNow.filePath !== canonical) return null;
                this.viewerTabs.setProbe(tabId, probe as FileProbe);
                const viewer = chooseViewer(probe, tabNow.viewerOverride);
                if (tabNow.viewer !== viewer) dispatch(this.blockId, { type: "SetTabViewer", tabId, viewer, source: "system" });
                if (viewer !== "text") return this._markViewerLoaded(tabId, `probe:${probe.size}:${probe.mtime}`, opts);
                // "Open as text anyway" on a binary file: show its bytes.
                allowBinaryText = isBinaryProbe(probe);
            }

            const result = await RpcApi.ReadEditorFileCommand(
                TabRpcClient,
                { path: filePath, ...this.on(tabId) },
                // Over ssh, which may first ask the user something.
                this.on(tabId).connection ? { timeout: 180_000 } : undefined
            );
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
            const refusal = sniffUnopenable(content, allowBinaryText);
            // "Open as text anyway" that CodeMirror can't take after all: back
            // to the file's own viewer, the choice dropped, and say why.
            const probe = refusal && allowBinaryText ? this.viewerTabs.probe(tabId) : null;
            if (refusal && probe) {
                dispatch(this.blockId, { type: "SetTabViewer", tabId, viewer: chooseViewer(probe), clearOverride: true, source: "system" });
                this.reportFileOpError("Opening as text", new Error(refusal));
                this._markViewerLoaded(tabId, `probe:${probe.size}:${probe.mtime}`);
                return null;
            }
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

    /** Handler for `editor:file_changed` MPS events. Reloads every open tab
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
    _syncWatch(tabId: string, canonicalPath: string): void {
        // A host's files aren't watched (this computer's watcher can't see
        // them); reopening the file shows changes made there.
        if (this.connection() || isDroppedPath(canonicalPath)) return;
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

    /** Close a tab. One with unsaved changes asks first (the view's
     *  `confirmDirtyClose`); closing it then discards them. */
    closeTab(tabId: string): void {
        const events = dispatch(this.blockId, { type: "CloseTab", tabId, source: "user" });
        if (!events.some((e) => e.type === "RequestDirtyConfirm")) return;
        const tab = snapshot(this.blockId)?.tabs.find((t) => t.id === tabId);
        if (tab && this.confirmDirtyClose) {
            this.confirmDirtyClose(tab, () => void dispatch(this.blockId, { type: "CloseTab", tabId, force: true, source: "user" }));
        }
    }

    switchTab(tabId: string): void {
        dispatch(this.blockId, { type: "SwitchTab", tabId, source: "user" });
    }

    /** The next or previous tab, wrapping (Ctrl+Tab, Ctrl+PageDown). */
    cycleTab(delta: number): void {
        dispatch(this.blockId, { type: "CycleTab", delta, source: "user" });
    }

    /** Read the active tab's file if it hasn't been (a tab restored from
     *  the block, or reopened). */
    private _ensureActiveLoaded(): void {
        const tab = snapshot(this.blockId)?.tabs.find((t) => t.id === snapshot(this.blockId)?.activeTabId);
        if (!tab || tab.contentLoaded || tab.loadError != null) return;
        const canonical = canonicalizePath(tab.filePath);
        if (this._loadingPaths.has(canonical)) return;
        this._loadingPaths.add(canonical);
        void this._loadFileIntoTab(tab.id, tab.filePath)
            .then((hash) => {
                if (hash !== null) this._syncWatch(tab.id, canonical);
            })
            .finally(() => this._loadingPaths.delete(canonical));
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
        // Only the text viewer has a buffer: never write an image's tab.
        if (tab.viewer && tab.viewer !== "text") return;

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
            await RpcApi.WriteEditorFileCommand(
                TabRpcClient,
                {
                    path: tab.filePath,
                    content,
                    ...this._encodingFor(tab.id),
                    ...this.on(tab.id),
                },
                this.on(tab.id).connection ? { timeout: 180_000 } : undefined
            );
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
     *  editor:scratch is true and no other file is open (`reuseExisting`
     *  true — idempotency guard, in case that path ever fires twice), and
     *  from the "+" button (`reuseExisting` false — a user clicking "+"
     *  always wants a genuinely new tab, same as any other tabbed UI;
     *  silently re-activating the pane's existing scratch tab there reads
     *  as "+ does nothing" when that tab is already the active one). */
    async openScratch(reuseExisting: boolean = true): Promise<string | undefined> {
        // A scratch buffer is a file on this computer: not in an editor on a
        // host, whose reads and saves go there.
        if (this.connection()) return undefined;
        if (reuseExisting) {
            // Reuse an existing scratch tab if one is already open in this pane.
            const existing = snapshot(this.blockId)?.tabs.find((t) => t.isScratch);
            if (existing) {
                dispatch(this.blockId, { type: "SwitchTab", tabId: existing.id, source: "system" });
                return existing.id;
            }
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
            if (!opened || (opened.type !== "TabOpened" && opened.type !== "TabActivated")) return undefined;
            const tabId = opened.tabId;
            // Read the actual on-disk content — the backend may have returned a
            // reused scratch file that already has content from a prior session.
            // Seeding "" here would clobber that content on the next Ctrl+S.
            // Marked as loading, so the restore path doesn't read it too.
            const scratchCanon = canonicalizePath(result.file_path);
            this._loadingPaths.add(scratchCanon);
            const fileResult = await RpcApi.ReadEditorFileCommand(TabRpcClient, { path: result.file_path }).finally(() =>
                this._loadingPaths.delete(scratchCanon)
            );
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
            return tabId;
        } catch {
            // Scratch creation failed — editor opens in empty state, user can
            // open a file manually.
            return undefined;
        }
    }

    /** A dropped file with no host path: its text in a new untitled tab,
     *  unsaved, so Save As chooses where it goes. */
    async openDroppedText(name: string, content: string): Promise<void> {
        if (sniffUnopenable(content)) {
            notifyDrop.cantOpen(name, "editor");
            return;
        }
        const tabId = await openEmptyScratch(
            () => this.openScratch(false),
            (id) => this._contentByTab.get(id),
        );
        if (!tabId) {
            notifyDrop.cantOpen(name, "editor");
            return;
        }
        this._contentByTab.set(tabId, content);
        this._contentVersion[1]((v) => v + 1);
        dispatch(this.blockId, { type: "MarkDirty", tabId, source: "system" });
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
            await RpcApi.PaneOpenCommand(TabRpcClient, {
                view: "term",
                cwd: folderPath,
                split_direction: "right",
                split_reference_block_id: this.blockId,
            }, {});
        } catch {
            // Opening beside is best-effort: fail silently.
        }
    }

    /** Open a file in a new editor pane, split right of the current one.
     *  Same pane.open mechanism as openInTerminal above, just view:"editor" +
     *  file instead of view:"term" + cwd. See
     *  docs/specs/SPEC_EDITOR_FILE_TREE_OPEN_ACTIONS_2026_07_12.md. */
    async openToTheSide(filePath: string): Promise<void> {
        try {
            await RpcApi.PaneOpenCommand(TabRpcClient, {
                view: "editor",
                file: filePath,
                split_direction: "right",
                split_reference_block_id: this.blockId,
            }, {});
        } catch {
            // Best-effort, same as openInTerminal.
        }
    }

    /** Open a file in a new editor pane inside a brand-new app tab
     *  (editor-open.ts's openFileInNewTab says why it is built by hand). */
    async openInNewTab(filePath: string): Promise<void> {
        await openFileInNewTab(filePath);
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

    async toggleWordWrap(): Promise<void> {
        const next = !this._wordWrap[0]();
        this._wordWrap[1](next);
        await this.persistMeta({ [META_WORD_WRAP]: next });
    }

    /** Right-click menu items for the editor's own body (the CodeMirror
     *  area) — surfaced by blockframe.tsx's onBodyContextMenu ahead of the
     *  generic pane actions (magnify/close). Context-specific to this view,
     *  same extension point sysinfo/browser/agent use for their own toggles. */
    getBodyContextMenuItems(): ContextMenuItem[] {
        return [
            {
                label: "Word Wrap",
                type: "checkbox",
                checked: this.wordWrapAtom(),
                click: () => void this.toggleWordWrap(),
            },
            { type: "separator" },
            { label: "Open from remote…", click: () => this.setOpenFromRemote(true) },
        ];
    }

    /** Shows or hides the "Open from remote…" picker. */
    setOpenFromRemote(open: boolean): void {
        this._openFromRemote[1](open);
    }

    /** Opens `path` on `host` (open-from-remote.ts), from the picker. */
    openFromRemote(host: string, path: string): Promise<void> {
        return openRemoteFile(this, host, path);
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
            await this.ctx.setMeta(meta);
        } catch {
            // Persistence failure isn't fatal — in-memory signal still drives
            // the current pane's behavior. On reopen, the previous persisted
            // value (or default) wins.
        }
    }

    // ── Viewers (SPEC_EDITOR_REVAMP_UNIFIED_VIEWERS_2026_10_15.md §4) ──

    /** The viewer showing the tab in front; "text" until its file is probed. */
    activeViewer(): ViewerId {
        const v = this.activeTabAtom()?.viewer;
        return isViewerId(v) ? v : "text";
    }

    /** "Reopen with…": show `tabId`'s file in `viewer`, and keep that choice.
     *  A text buffer with unsaved changes stays text. */
    reopenWith(tabId: string, viewer: ViewerId): void {
        const tab = snapshot(this.blockId)?.tabs.find((t) => t.id === tabId);
        if (!tab || (tab.viewer ?? "text") === viewer || tab.dirty) return;
        this._contentByTab.delete(tabId);
        this._contentVersion[1]((v) => v + 1);
        dispatch(this.blockId, { type: "SetTabViewer", tabId, viewer, override: true, source: "user" });
    }

    setFollowFolder(tabId: string, follow: boolean): void {
        dispatch(this.blockId, { type: "SetFollowFolder", tabId, follow, source: "user" });
    }

    /** The tab shows another file (follow-folder, `[` / `]`); it reads it. */
    retargetTab(tabId: string, newPath: string): void {
        const events = dispatch(this.blockId, { type: "RetargetTab", tabId, newPath, source: "system" });
        if (events.some((e) => e.type === "TabActivated" && e.tabId === tabId)) this._contentByTab.delete(tabId);
    }

    /** A dropped file with no host path that isn't text: shown from its
     *  bytes (an image, a video), else the info card. */
    async openDroppedFile(file: File): Promise<void> {
        await this.openFile(rememberDropped(file));
    }

    private _loadDroppedIntoTab(tabId: string, path: string, opts?: { skipIfHashMatches?: string }): string | null {
        const file = droppedFileOf(path);
        if (!file) {
            const error = "This dropped file is no longer here: dropped files aren't kept after a restart. Drop it again.";
            dispatch(this.blockId, { type: "TabContentLoadFailed", tabId, error, source: "system" });
            return null;
        }
        const tab = snapshot(this.blockId)?.tabs.find((t) => t.id === tabId);
        const viewer = tab?.viewerOverride === "info" ? "info" : viewerForDropped(file.name, file.type);
        if (tab?.viewer !== viewer) dispatch(this.blockId, { type: "SetTabViewer", tabId, viewer, source: "system" });
        return this._markViewerLoaded(tabId, `dropped:${file.size}:${file.lastModified}`, opts);
    }

    /** A non-text viewer's tab is loaded once its viewer is chosen: the
     *  viewer fetches the bytes. Read-only, so save never writes it. */
    private _markViewerLoaded(tabId: string, hash: string, opts?: { skipIfHashMatches?: string }): string {
        if (opts?.skipIfHashMatches === hash) return hash;
        this._contentByTab.set(tabId, "");
        this.viewerTabs.bumpRevision(tabId);
        dispatch(this.blockId, { type: "TabContentLoaded", tabId, contentHash: hash, readOnly: true, source: "system" });
        return hash;
    }

    giveFocus(): boolean {
        const cm = this.cmViewRef.current;
        if (cm == null) return false;
        cm.focus();
        return true;
    }

    dispose(): void {
        // Tabs changed in the last moments are still written.
        this._docTabsSaver.flush();
        this._disposeFileDrop();
        this._unsubFileChanged();
        this._disposePendingOpenFilesEffect();
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
