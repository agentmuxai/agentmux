// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Editor pane state store — slice #10 of the frontend reducer roadmap.
 *
 * The Editor's open files are document tabs
 * (docs/specs/SPEC_DOCUMENT_TABS_2026_10_02.md §6.1): the tab list, the
 * active tab, preview tabs, the order and the reopen list are the shared
 * model in `frontend/app/doc-tabs/doc-tabs.ts`, changed only through its
 * commands. What this slice adds is the Editor's own state per tab (the
 * buffer's path, language, load state and scratch identity: `EditorBuffer`,
 * the tab's payload), its command and event vocabulary, the dirty-close
 * confirmation, and the audit ring.
 *
 * The state carries the shared model (`doc`) plus a projection of it in the
 * shape the editor has always read: `tabs` (flat `EditorTab`s),
 * `activeTabId` and `recentlyClosed`. The projection is rebuilt only when
 * `doc` changes and reuses unchanged tabs' objects, so Solid sees the same
 * references for tabs that didn't change.
 *
 * Per-tab CodeMirror state lives OUTSIDE this cell (the view holds it in a
 * Map keyed by tabId — it's not serializable and shouldn't be in the audit
 * ring).
 *
 * Pattern matches slice #4 (`agent-pane-state-store.ts`) and slice #9
 * (`browser-pane-state-store.ts`) — same `update(state, command) →
 * { state, events }` shape, same slot lifecycle, same `recordDispatch`
 * audit integration, same "throw on unregistered dispatch" rule.
 */

import {
    activateDoc,
    closeDoc,
    type DocTab,
    type DocTabsState,
    emptyDocTabs,
    MAX_CLOSED,
    moveDoc,
    moveDocTo,
    newDocTabId,
    openDoc,
    promoteDoc,
    updateDoc,
} from "@/app/doc-tabs/doc-tabs";
import { type CommandSource, recordDispatch } from "./command-source";

// ─────────────────────────────────────────────────────────────────────
// Types
// ─────────────────────────────────────────────────────────────────────

/** The Editor's own state for one tab: the document tab's payload. */
export interface EditorBuffer {
    /** Canonicalized absolute path (see `canonicalizePath`). */
    filePath: string;
    /** Derived from extension at open time. */
    language: string;
    /** Set when content-load returns read-only. */
    readOnly: boolean;
    /** sha256 of last-loaded content; `""` before load resolves. */
    contentHash: string;
    /** Error message from the most recent load attempt; null when fine. */
    loadError: string | null;
    /** Transient — true once the lazy fetch resolved. Not persisted. */
    contentLoaded: boolean;
    /** Scratch/untitled buffer — backed by a cache file in
     *  ~/.agentmux/cache/scratch/. True while the file hasn't been
     *  promoted to a real user-chosen path via Save As. */
    isScratch?: boolean;
    /** UUID of the backing scratch cache file. Set iff isScratch is true. */
    scratchId?: string;
    /** Label to show in the tab instead of the bare filename. Used for
     *  scratch buffers ("Untitled-1") and may be set by pane.open callers. */
    displayName?: string;
}

/** One tab as the editor reads it: the document tab, flattened. */
export interface EditorTab extends EditorBuffer {
    /** Stable for the tab's life. Survives reorders. */
    id: string;
    /** True between first change and save. */
    dirty: boolean;
    /** Preview tab — at most one per pane. A single-click in the tree
     *  opens into the preview slot (replacing the current preview's file).
     *  Double-click opens as pinned. Editing or explicit pin promotes a
     *  preview to pinned. Matches VS Code semantics. */
    isPreview: boolean;
}

interface ClosedTab {
    filePath: string;
}

export interface EditorPaneState {
    /** The shared document-tab model: the source of truth. */
    readonly doc: DocTabsState<EditorBuffer>;
    /** Projection of `doc.tabs`, in display order. */
    readonly tabs: EditorTab[];
    readonly activeTabId: string | null;
    /** Oldest first, at most MAX_RECENTLY_CLOSED (Ctrl+Shift+T). */
    readonly recentlyClosed: ClosedTab[];
}

export const MAX_RECENTLY_CLOSED = MAX_CLOSED;

// Projection cache: an unchanged document tab keeps its EditorTab object.
const projected = new WeakMap<DocTab<EditorBuffer>, EditorTab>();

function projectTab(t: DocTab<EditorBuffer>): EditorTab {
    let e = projected.get(t);
    if (!e) {
        e = { ...t.payload, id: t.id, dirty: !!t.dirty, isPreview: t.preview };
        projected.set(t, e);
    }
    return e;
}

/** The state for a document-tab model. */
export function fromDocTabs(doc: DocTabsState<EditorBuffer>): EditorPaneState {
    return {
        doc,
        tabs: doc.tabs.map(projectTab),
        activeTabId: doc.activeId,
        recentlyClosed: [...doc.closed].reverse().map((t) => ({ filePath: t.payload.filePath })),
    };
}

export const initialState = (): EditorPaneState => fromDocTabs(emptyDocTabs<EditorBuffer>());

// Hydration shapes — input to bulk-restore commands. Note these don't
// carry transient fields; the reducer reconstructs full tabs with
// `contentLoaded: false`.
interface HydratedTab {
    id: string;
    filePath: string;
    language?: string;
    readOnly?: boolean;
}

// ─────────────────────────────────────────────────────────────────────
// Commands
// ─────────────────────────────────────────────────────────────────────

/**
 * Optional `source` tag on every command — the echo-loop guard hook. The
 * view dispatches CodeMirror updates with `source: "cm-update"`; `"hydrate"`
 * marks the initial doc the view writes back to CodeMirror after a restore.
 * See slice #2 convention.
 */
type EditorCommandSource = "user" | "system" | "cm-update" | "hydrate";

export type EditorPaneCommand =
    | {
          type: "OpenFile";
          path: string;
          language?: string;
          /** "preview" (default for tree single-click) → replaces the
           *  current preview tab if any, else creates a new preview.
           *  "pinned" (tree double-click, programmatic open) → always
           *  adds a non-preview tab. Activating an already-open tab
           *  is unchanged regardless of mode. */
          mode?: "preview" | "pinned";
          source?: EditorCommandSource;
      }
    | {
          type: "OpenScratch";
          /** Real path of the backing cache file (in ~/.agentmux/cache/scratch/). */
          filePath: string;
          scratchId: string;
          displayName: string;
          language?: string;
          source?: EditorCommandSource;
      }
    | {
          type: "PromoteScratch";
          /** Scratch tab to promote. */
          tabId: string;
          /** The user-chosen real path the scratch was moved to. */
          newPath: string;
          source?: EditorCommandSource;
      }
    | { type: "CloseTab"; tabId: string; force?: boolean; source?: EditorCommandSource }
    | { type: "PinTab"; tabId: string; source?: EditorCommandSource }
    | { type: "SwitchTab"; tabId: string; source?: EditorCommandSource }
    | { type: "CycleTab"; delta: number; source?: EditorCommandSource }
    | { type: "ReorderTab"; tabId: string; toIndex: number; source?: EditorCommandSource }
    /** A tab dragged to just before or after another (`moveDocTo`). */
    | { type: "MoveTabTo"; tabId: string; targetId: string; position: "before" | "after"; source?: EditorCommandSource }
    | { type: "MarkDirty"; tabId: string; source?: EditorCommandSource }
    | { type: "ClearDirty"; tabId: string; source?: EditorCommandSource }
    | {
          type: "TabContentLoaded";
          tabId: string;
          contentHash: string;
          readOnly?: boolean;
          source?: EditorCommandSource;
      }
    | { type: "TabContentLoadFailed"; tabId: string; error: string; source?: EditorCommandSource }
    | { type: "ReopenLastClosed"; source?: EditorCommandSource }
    | {
          type: "HydrateFromMeta";
          tabs: HydratedTab[];
          activeTabId: string | null;
          source?: EditorCommandSource;
      }
    | {
          type: "HydrateFromDefaults";
          tabs: HydratedTab[];
          activeTabId: string | null;
          source?: EditorCommandSource;
      }
    | {
          /** Restore a pane's tabs from its block's `doctabs` record
           *  (SPEC_DOCUMENT_TABS §5.3), already rebuilt by `hydrateDocTabs`. */
          type: "RestoreDocTabs";
          doc: DocTabsState<EditorBuffer>;
          source?: EditorCommandSource;
      }
    | { type: "RenameFile"; oldPath: string; newPath: string; source?: EditorCommandSource };

// ─────────────────────────────────────────────────────────────────────
// Events
// ─────────────────────────────────────────────────────────────────────

export type EditorPaneEvent =
    | { type: "TabOpened"; tabId: string; filePath: string; atIndex: number }
    | { type: "TabClosed"; tabId: string; filePath: string }
    | { type: "TabActivated"; tabId: string; filePath: string }
    | {
          type: "TabsRestored";
          tabIds: string[];
          activeTabId: string | null;
          fromDefaults: boolean;
      }
    | { type: "TabDirtied"; tabId: string }
    | { type: "TabSaved"; tabId: string }
    | { type: "TabContentChanged"; tabId: string }
    | {
          type: "RequestDirtyConfirm";
          tabId: string;
          originalCommand: EditorPaneCommand;
      }
    | {
          type: "GlobalDefaultTabsChanged";
          tabs: { filePath: string }[];
          activeTabId: string | null;
      };

export interface ReducerResult {
    state: EditorPaneState;
    events: EditorPaneEvent[];
}

// ─────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────

/**
 * Canonicalize a filesystem path so `C:/x` and `C:\x` resolve to the
 * same tab (cheap, deterministic, no I/O):
 *   - strip Windows' `\\?\` extended-length ("verbatim") prefix
 *   - normalize backslashes to forward slashes
 *   - collapse repeated slashes
 *   - lowercase the Windows drive letter
 *   - strip a trailing slash (except for a bare "/" or drive root)
 *
 * **The `\\?\` strip closes a real live-reload bug**, confirmed by live
 * repro (2026-08-22): `EditorFileWatcher`'s published `editor:file_changed`
 * MPS event carries a path produced by Rust's `Path::canonicalize()`,
 * which on Windows unconditionally prepends `\\?\` (`\\?\UNC\` for a
 * network share) — well-documented std behavior, and something this
 * backend's own comments already flag in two other places
 * (`editor_file_watcher.rs`, `media_file_watcher.rs`) for the backend's
 * OWN internal path matching. Nothing on the frontend ever produces that
 * prefix for the same file (a tab's `filePath` is derived from whatever
 * path was originally requested to open it), so without stripping it
 * here, `_handleExternalFileChanged`'s `canonicalizePath(rawPath) !==
 * canonicalizePath(tab.filePath)` comparison NEVER matches — live-reload
 * silently never fires for any tab, on Windows, unconditionally.
 *
 * Symlink resolution is intentionally out of scope — that requires
 * filesystem I/O which can't sit inside a pure reducer.
 */
export function canonicalizePath(path: string): string {
    if (!path) return path;
    let p = path;
    if (p.startsWith("\\\\?\\UNC\\")) {
        p = "\\\\" + p.slice(8); // \\?\UNC\server\share\... -> \\server\share\...
    } else if (p.startsWith("\\\\?\\")) {
        p = p.slice(4); // \\?\C:\... -> C:\...
    }
    p = p.replace(/\\/g, "/");
    // codex P2 on PR #2739: a UNC path's leading "//" (the authority
    // marker distinguishing \\server\share from a current-drive-rooted
    // path) must survive the doubled-slash collapse below, or the
    // \\?\UNC\ strip above is pointless — the result would compare equal
    // between panes, but be unusable as an actual path to watch/read.
    const isUnc = p.startsWith("//");
    p = p.replace(/\/{2,}/g, "/");
    if (isUnc) p = "/" + p;
    // Windows drive letter — lowercase for stable equality.
    if (/^[A-Za-z]:\//.test(p)) {
        p = p[0].toLowerCase() + p.slice(1);
    }
    // Strip trailing slash unless this is the root.
    if (p.length > 1 && p.endsWith("/") && !/^[a-z]:\/$/.test(p)) {
        p = p.slice(0, -1);
    }
    return p;
}

/** Derive a language id from a file extension. A fallback label: the
 *  model passes its own mapped language (`detectLanguage`) on open. */
function deriveLanguage(path: string): string {
    const m = /\.([A-Za-z0-9]+)$/.exec(path);
    if (!m) return "text";
    return m[1].toLowerCase();
}

function baseName(path: string): string {
    const i = Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));
    return i >= 0 ? path.slice(i + 1) : path;
}

/** A tab's title in the strip: a scratch buffer's name, else the file's. */
export function editorTabTitle(b: Pick<EditorBuffer, "filePath" | "displayName">): string {
    return b.displayName || baseName(b.filePath);
}

/** A buffer that hasn't been read yet. */
function freshBuffer(path: string, language?: string): EditorBuffer {
    const canon = canonicalizePath(path);
    return {
        filePath: canon,
        language: language ?? deriveLanguage(canon),
        readOnly: false,
        contentHash: "",
        loadError: null,
        contentLoaded: false,
    };
}

/** A buffer as it is after a restart or a reopen: not read yet. */
function unloaded(b: EditorBuffer): EditorBuffer {
    return { ...b, contentHash: "", loadError: null, contentLoaded: false };
}

function tabById(doc: DocTabsState<EditorBuffer>, id: string): DocTab<EditorBuffer> | undefined {
    return doc.tabs.find((t) => t.id === id);
}

function same(state: EditorPaneState): ReducerResult {
    return { state, events: [] };
}

/** The state after `doc` changed (the same state when it didn't). */
function next(state: EditorPaneState, doc: DocTabsState<EditorBuffer>, events: EditorPaneEvent[] = []): ReducerResult {
    return { state: doc === state.doc ? state : fromDocTabs(doc), events };
}

function patchBuffer(doc: DocTabsState<EditorBuffer>, id: string, patch: Partial<EditorBuffer>): DocTabsState<EditorBuffer> {
    const tab = tabById(doc, id);
    if (!tab) return doc;
    const payload = { ...tab.payload, ...patch };
    return updateDoc(doc, id, {
        payload,
        key: payload.filePath,
        title: editorTabTitle(payload),
    });
}

function activated(doc: DocTabsState<EditorBuffer>): EditorPaneEvent[] {
    const tab = doc.activeId ? tabById(doc, doc.activeId) : undefined;
    return tab ? [{ type: "TabActivated", tabId: tab.id, filePath: tab.payload.filePath }] : [];
}

function opened(doc: DocTabsState<EditorBuffer>, id: string): EditorPaneEvent[] {
    const atIndex = doc.tabs.findIndex((t) => t.id === id);
    const tab = doc.tabs[atIndex];
    return [{ type: "TabOpened", tabId: id, filePath: tab.payload.filePath, atIndex }];
}

function hydrate(tabs: HydratedTab[], activeTabId: string | null): DocTabsState<EditorBuffer> {
    const docTabs: DocTab<EditorBuffer>[] = tabs.map((t) => {
        const payload = { ...freshBuffer(t.filePath, t.language), readOnly: t.readOnly ?? false };
        // Restored tabs are never previews: the user committed to them
        // by leaving them open across the restart.
        return { id: t.id, key: payload.filePath, title: editorTabTitle(payload), preview: false, pinned: false, payload };
    });
    const active = docTabs.some((t) => t.id === activeTabId) ? activeTabId : (docTabs[0]?.id ?? null);
    return { tabs: docTabs, activeId: active, mru: active ? [active] : [], closed: [] };
}

// ─────────────────────────────────────────────────────────────────────
// Reducer
// ─────────────────────────────────────────────────────────────────────

/**
 * Pure reducer. Returns the next state plus any events to emit. Never
 * throws; defensive no-ops (e.g. SwitchTab on a missing id) return the
 * input state with an empty events array.
 *
 * Invariants enforced:
 *   1. `activeTabId` points to a tab in `tabs[]` or is null when
 *      `tabs[]` is empty.
 *   2. Tab `id`s are unique within a pane, and one file has one tab
 *      (opening an open path activates its tab).
 *   3. `recentlyClosed.length <= MAX_RECENTLY_CLOSED`.
 *   4. `MarkDirty` / `ClearDirty` emit their events only on actual
 *      transitions (idempotent).
 */
export function update(state: EditorPaneState, command: EditorPaneCommand): ReducerResult {
    const doc = state.doc;
    switch (command.type) {
        case "OpenFile": {
            const buffer = freshBuffer(command.path, command.language);
            const mode = command.mode ?? "pinned";
            const existing = doc.tabs.find((t) => t.key === buffer.filePath);
            if (existing) {
                // A pinned open of the file in preview pins it (tree
                // double-click on the previewed file).
                const pinned = existing.preview && mode === "pinned" ? promoteDoc(doc, existing.id) : doc;
                const d = pinned === doc && doc.activeId === existing.id ? doc : activateDoc(pinned, existing.id);
                return next(state, d, [
                    { type: "TabActivated", tabId: existing.id, filePath: existing.payload.filePath },
                ]);
            }
            const preview = mode === "preview" ? doc.tabs.find((t) => t.preview) : undefined;
            const args = { key: buffer.filePath, title: editorTabTitle(buffer), payload: buffer, preview: mode === "preview" };
            if (preview) {
                // The preview slot shows the new file, keeping its tab id:
                // the view's state for other tabs is untouched, and the
                // reset buffer makes the model read the new file.
                const d = openDoc(doc, args, preview.id);
                return next(state, d, [{ type: "TabActivated", tabId: preview.id, filePath: buffer.filePath }]);
            }
            const id = newDocTabId();
            const d = openDoc(doc, args, id);
            return next(state, d, opened(d, id));
        }

        case "PinTab": {
            const tab = tabById(doc, command.tabId);
            if (!tab?.preview) return same(state);
            return next(state, promoteDoc(doc, tab.id));
        }

        case "CloseTab": {
            const tab = tabById(doc, command.tabId);
            // Closing an already-closed tab is benign (double-click on the
            // ×, stale IPC).
            if (!tab) return same(state);
            if (tab.dirty && !command.force) {
                // The view asks; on confirm it re-dispatches with `force`.
                return { state, events: [{ type: "RequestDirtyConfirm", tabId: tab.id, originalCommand: command }] };
            }
            const d = closeDoc(doc, tab.id);
            const events: EditorPaneEvent[] = [{ type: "TabClosed", tabId: tab.id, filePath: tab.payload.filePath }];
            if (doc.activeId === tab.id) events.push(...activated(d));
            return next(state, d, events);
        }

        case "SwitchTab": {
            if (!tabById(doc, command.tabId) || doc.activeId === command.tabId) return same(state);
            const d = activateDoc(doc, command.tabId);
            return next(state, d, activated(d));
        }

        case "CycleTab": {
            if (doc.tabs.length < 2) return same(state);
            const at = Math.max(0, doc.tabs.findIndex((t) => t.id === doc.activeId));
            const target = doc.tabs[(((at + command.delta) % doc.tabs.length) + doc.tabs.length) % doc.tabs.length];
            return update(state, { type: "SwitchTab", tabId: target.id, source: command.source });
        }

        case "ReorderTab": {
            const idx = doc.tabs.findIndex((t) => t.id === command.tabId);
            if (idx < 0) return same(state);
            const to = Math.max(0, Math.min(doc.tabs.length - 1, command.toIndex));
            return next(state, moveDoc(doc, command.tabId, to - idx));
        }

        case "MoveTabTo": {
            const moved = moveDocTo(doc, command.tabId, command.targetId, command.position);
            return moved === doc ? same(state) : next(state, moved);
        }

        case "MarkDirty": {
            const tab = tabById(doc, command.tabId);
            // Already dirty → no event re-emission.
            if (!tab || tab.dirty) return same(state);
            // Editing a preview tab promotes it (VS Code: once you've
            // started editing, the tab survives the next preview open).
            const d = promoteDoc(updateDoc(doc, tab.id, { dirty: true }), tab.id);
            return next(state, d, [{ type: "TabDirtied", tabId: tab.id }]);
        }

        case "ClearDirty": {
            const tab = tabById(doc, command.tabId);
            if (!tab?.dirty) return same(state);
            return next(state, updateDoc(doc, tab.id, { dirty: false }), [{ type: "TabSaved", tabId: tab.id }]);
        }

        case "TabContentLoaded": {
            const tab = tabById(doc, command.tabId);
            if (!tab) return same(state);
            return next(
                state,
                patchBuffer(doc, tab.id, {
                    contentLoaded: true,
                    contentHash: command.contentHash,
                    loadError: null,
                    readOnly: command.readOnly ?? tab.payload.readOnly,
                })
            );
        }

        case "TabContentLoadFailed": {
            // contentLoaded stays as it was: for operational failures (a
            // failed save) the buffer the view holds is still valid, and the
            // centered error panel must NOT replace CodeMirror; the small
            // top banner picks the error up via the loadError accessor.
            if (!tabById(doc, command.tabId)) return same(state);
            return next(state, patchBuffer(doc, command.tabId, { loadError: command.error }));
        }

        case "ReopenLastClosed": {
            const [last, ...rest] = doc.closed;
            if (!last) return same(state);
            const buffer = unloaded(last.payload);
            const trimmed = { ...doc, closed: rest };
            // Open again (a tab still open on that file just activates).
            const existing = doc.tabs.find((t) => t.key === buffer.filePath);
            if (existing) {
                const d = activateDoc(trimmed, existing.id);
                return next(state, d, activated(d));
            }
            const id = newDocTabId();
            const d = openDoc(trimmed, { key: buffer.filePath, title: editorTabTitle(buffer), payload: buffer }, id);
            return next(state, d, opened(d, id));
        }

        case "HydrateFromMeta":
        case "HydrateFromDefaults": {
            const d = { ...hydrate(command.tabs, command.activeTabId), closed: doc.closed };
            return next(state, d, [
                {
                    type: "TabsRestored",
                    tabIds: d.tabs.map((t) => t.id),
                    activeTabId: d.activeId,
                    fromDefaults: command.type === "HydrateFromDefaults",
                },
            ]);
        }

        case "RestoreDocTabs": {
            const d: DocTabsState<EditorBuffer> = {
                ...command.doc,
                tabs: command.doc.tabs.map((t) => ({ ...t, preview: false, dirty: false, payload: unloaded(t.payload) })),
                closed: doc.closed,
            };
            return next(state, d, [{ type: "TabsRestored", tabIds: d.tabs.map((t) => t.id), activeTabId: d.activeId, fromDefaults: false }]);
        }

        case "OpenScratch": {
            const canon = canonicalizePath(command.filePath);
            // A scratch tab on this file already: just activate it.
            const existing = doc.tabs.find((t) => t.key === canon && t.payload.isScratch);
            if (existing) {
                const d = activateDoc(doc, existing.id);
                return next(state, d, [{ type: "TabActivated", tabId: existing.id, filePath: existing.payload.filePath }]);
            }
            const buffer: EditorBuffer = {
                ...freshBuffer(command.filePath, command.language ?? "markdown"),
                isScratch: true,
                scratchId: command.scratchId,
                displayName: command.displayName,
            };
            const id = newDocTabId();
            const d = openDoc(doc, { key: canon, title: editorTabTitle(buffer), payload: buffer }, id);
            return next(state, d, opened(d, id));
        }

        case "PromoteScratch": {
            const tab = tabById(doc, command.tabId);
            if (!tab) return same(state);
            const d = patchBuffer(updateDoc(doc, tab.id, { dirty: false }), tab.id, {
                filePath: canonicalizePath(command.newPath),
                language: deriveLanguage(command.newPath),
                isScratch: false,
                scratchId: undefined,
                displayName: undefined,
            });
            return next(state, d, [{ type: "TabSaved", tabId: tab.id }]);
        }

        case "RenameFile": {
            const canonOld = canonicalizePath(command.oldPath);
            const tab = doc.tabs.find((t) => t.key === canonOld);
            if (!tab) return same(state);
            return next(state, patchBuffer(doc, tab.id, { filePath: canonicalizePath(command.newPath) }));
        }
    }
}

// ── Persistence (SPEC_DOCUMENT_TABS §5.3) ────────────────────────────────

/** What a tab keeps in the block's `doctabs` record: where its file is,
 *  and a scratch buffer's identity. Never the buffer's content. */
export function serializeEditorBuffer(b: EditorBuffer): unknown {
    return {
        path: b.filePath,
        language: b.language,
        ...(b.isScratch && b.scratchId ? { scratchId: b.scratchId, displayName: b.displayName } : {}),
    };
}

export function deserializeEditorBuffer(state: unknown): EditorBuffer | null {
    const s = state as { path?: unknown; language?: unknown; scratchId?: unknown; displayName?: unknown } | null;
    if (!s || typeof s.path !== "string" || !s.path) return null;
    const b = freshBuffer(s.path, typeof s.language === "string" ? s.language : undefined);
    if (typeof s.scratchId === "string") {
        b.isScratch = true;
        b.scratchId = s.scratchId;
        if (typeof s.displayName === "string") b.displayName = s.displayName;
    }
    return b;
}

// ─────────────────────────────────────────────────────────────────────
// Slot store
// ─────────────────────────────────────────────────────────────────────

interface Slot {
    state: EditorPaneState;
}

const slots = new Map<string, Slot>();

/**
 * Event sink — installed once by the editor model (editor-model.ts fans it
 * out to every pane). The default is a no-op so tests run without DOM. The
 * sink receives the events array from a single dispatch call, so a pane can
 * act on a batch (a close plus the activation it caused) at once.
 */
type EventSink = (events: EditorPaneEvent[]) => void;
let eventSink: EventSink | null = null;

export function setEventSink(sink: EventSink | null): void {
    eventSink = sink;
}

/**
 * Register an editor pane. Call SYNCHRONOUSLY from the model's
 * constructor so subsequent dispatches see a live slot. Re-registering
 * an existing blockId is a no-op (the slot keeps its state — see
 * `agent-pane-state-store.ts`'s comment on idempotency for hot-reload
 * paths).
 */
export function registerEditorPane(blockId: string): void {
    if (slots.has(blockId)) return;
    slots.set(blockId, { state: initialState() });
}

export function unregisterEditorPane(blockId: string): void {
    slots.delete(blockId);
}

/**
 * Apply a command. Throws on unregistered blockId — silent drops would
 * defeat the reducer's audit value (same rule as the other slices).
 */
export function dispatch(
    blockId: string,
    command: EditorPaneCommand,
    source: CommandSource = "system",
): EditorPaneEvent[] {
    const slot = slots.get(blockId);
    if (!slot) {
        throw new Error(
            `[editor-pane] dispatch for unregistered pane ${blockId.slice(0, 7)} (cmd=${command.type}). registerEditorPane must be called synchronously in the EditorViewModel constructor.`,
        );
    }
    const prev = slot.state;
    const result = update(prev, command);
    slot.state = result.state;

    // Call the sink whenever STATE changed, even if no semantic events were
    // emitted (e.g. TabContentLoaded mutates the tab record but doesn't
    // currently emit an event). Subscribers use this as the cue to re-read
    // their projections; without it, view-side derivations bound to slice
    // state (active-tab `contentLoaded`, `loadError`, etc.) silently miss
    // the update and the UI looks frozen. The empty-events case is a no-op
    // for code that iterates `events` and a wake-up for code that doesn't.
    if (eventSink && result.state !== prev) {
        eventSink(result.events);
    }

    recordDispatch({
        slice: "editor-pane",
        key: blockId,
        command,
        events: result.events,
        source,
        at: Date.now(),
    });

    return result.events;
}

/**
 * Soft-dispatch variant. Returns an empty event array if the slot is
 * already gone instead of throwing. Use ONLY from async contexts
 * (RAF / setTimeout / await continuations / subscription handlers)
 * where a normal dispatch can race against the pane's onCleanup
 * unregistering the slot. Synchronous component-body dispatches MUST
 * continue to use `dispatch` — a missing slot there is a registration-
 * order bug and the throw is the right signal.
 */
export function dispatchIfRegistered(
    blockId: string,
    command: EditorPaneCommand,
    source: CommandSource = "system",
): EditorPaneEvent[] {
    if (!slots.has(blockId)) return [];
    return dispatch(blockId, command, source);
}

/** Snapshot — diagnostics + tests only. */
export function snapshot(blockId: string): EditorPaneState | null {
    return slots.get(blockId)?.state ?? null;
}

/** Return the scratchId of every scratch tab currently open across ALL editor panes. */
export function getAllActiveScratchIds(): string[] {
    const ids: string[] = [];
    for (const slot of slots.values()) {
        for (const tab of slot.state.tabs) {
            if (tab.isScratch && tab.scratchId) ids.push(tab.scratchId);
        }
    }
    return ids;
}

/** Test/dev helper — clears every slot AND resets the event sink. */
export function resetAllSlots(): void {
    slots.clear();
    eventSink = null;
}
