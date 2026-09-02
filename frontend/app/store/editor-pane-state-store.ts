// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Editor pane state store — slice #10 of the frontend reducer roadmap.
 * Phase 1A: pure reducer + slot store + audit-ring integration. No view
 * wiring, no saga, no CodeMirror references — those land in Phase 1B/1C.
 * Spec: `docs/specs/SPEC_EDITOR_TABS_2026-05-26.md` §"State management" and
 * §"Phase 1A".
 *
 * The editor pane today owns one file at a time; this slice is the
 * foundation for the multi-file tab strip. The slot cell owns:
 *   - `tabs[]` — ordered list, each with id/filePath/language/dirty/etc.
 *   - `activeTabId` — id of the currently-active tab, or null when empty.
 *   - `recentlyClosed[]` — bounded ring (max 10) feeding Ctrl+Shift+T.
 *
 * Per-tab CodeMirror state lives OUTSIDE this cell (the view holds it
 * in a Map keyed by tabId — it's not serializable and shouldn't be in
 * the audit ring). The reducer only owns the data the persistence /
 * audit / LSP-coupling layers need.
 *
 * Pattern matches slice #4 (`agent-pane-state-store.ts`) and slice #9
 * (`browser-pane-state-store.ts`) — same `update(state, command) →
 * { state, events }` shape, same slot lifecycle, same
 * `recordDispatch` audit integration, same "throw on unregistered
 * dispatch" rule.
 */

import { type CommandSource, recordDispatch } from "./command-source";

// ─────────────────────────────────────────────────────────────────────
// Types
// ─────────────────────────────────────────────────────────────────────

export interface EditorTab {
    /** Stable per-pane id (uuid). Survives reorders. */
    id: string;
    /** Canonicalized absolute path (see `canonicalizePath`). */
    filePath: string;
    /** Derived from extension at open time. */
    language: string;
    /** Set when content-load returns read-only. */
    readOnly: boolean;
    /** True between first change and save. */
    dirty: boolean;
    /** sha256 of last-loaded content; `""` before load resolves. */
    contentHash: string;
    /** Error message from the most recent load attempt; null when fine. */
    loadError: string | null;
    /** Transient — true once the lazy fetch resolved. Not persisted. */
    contentLoaded: boolean;
    /** Preview tab — at most one per pane. A single-click in the tree
     *  opens into the preview slot (replacing the current preview's file).
     *  Double-click opens as pinned. Editing or explicit pin promotes a
     *  preview to pinned. Matches VS Code semantics. */
    isPreview: boolean;
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

interface ClosedTab {
    filePath: string;
    closedAt: number;
}

export interface EditorPaneState {
    tabs: EditorTab[];
    activeTabId: string | null;
    /** Capped at MAX_RECENTLY_CLOSED, oldest evicted on overflow. */
    recentlyClosed: ClosedTab[];
}

export const MAX_RECENTLY_CLOSED = 10;

export const initialState = (): EditorPaneState => ({
    tabs: [],
    activeTabId: null,
    recentlyClosed: [],
});

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
 * Optional `source` tag on every command — the echo-loop guard hook
 * for Phase 1B. The view dispatches CodeMirror updates with
 * `source: "cm-update"`; the reducer skips emitting `TabContentChanged`
 * for commands whose source indicates the change originated from the
 * reducer itself (e.g. `"hydrate"` for the initial doc the view writes
 * back to CodeMirror after a HydrateFromMeta). See slice #2 convention.
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
           *  appends a non-preview tab. Activating an already-open tab
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
    | { type: "ReorderTab"; tabId: string; toIndex: number; source?: EditorCommandSource }
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
 * same tab. Phase 1A approach (cheap, deterministic, no I/O):
 *   - strip Windows' `\\?\` extended-length ("verbatim") prefix
 *   - normalize backslashes to forward slashes
 *   - collapse repeated slashes
 *   - lowercase the Windows drive letter
 *   - strip a trailing slash (except for a bare "/" or drive root)
 *
 * **The `\\?\` strip closes a real live-reload bug**, confirmed by live
 * repro (2026-08-22): `EditorFileWatcher`'s published `editor:file_changed`
 * WPS event carries a path produced by Rust's `Path::canonicalize()`,
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
 * filesystem I/O which can't sit inside a pure reducer. The saga in
 * Phase 1C may canonicalize further before dispatching; the reducer's
 * job is just to make trivially-equivalent paths collide.
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

/** Derive a CodeMirror-friendly language id from a file extension.
 *  Phase 1A keeps this minimal — the view's existing extension-to-mode
 *  table can supersede it once we wire 1B. */
function deriveLanguage(path: string): string {
    const m = /\.([A-Za-z0-9]+)$/.exec(path);
    if (!m) return "text";
    return m[1].toLowerCase();
}

function newTabId(): string {
    // crypto.randomUUID is available in modern Chromium (host runtime)
    // and the test environment (vitest on Node ≥ 19). Fallback path
    // covers older Node just in case.
    if (typeof crypto !== "undefined" && typeof crypto.randomUUID === "function") {
        return crypto.randomUUID();
    }
    return `tab-${Math.random().toString(36).slice(2)}-${Date.now()}`;
}

function findTabIndex(state: EditorPaneState, tabId: string): number {
    return state.tabs.findIndex((t) => t.id === tabId);
}

function findTabByPath(
    state: EditorPaneState,
    canonicalPath: string,
): EditorTab | undefined {
    return state.tabs.find((t) => t.filePath === canonicalPath);
}

/** Build a fresh tab record for the given path/language. */
function makeTab(path: string, language?: string, isPreview = false): EditorTab {
    const canon = canonicalizePath(path);
    return {
        id: newTabId(),
        filePath: canon,
        language: language ?? deriveLanguage(canon),
        readOnly: false,
        dirty: false,
        contentHash: "",
        loadError: null,
        contentLoaded: false,
        isPreview,
    };
}

/**
 * Pick the new active id after `tabId` has been removed from `prevTabs`.
 * Matches VS Code: prefer the right neighbor of the closed tab; fall
 * back to the left when closing the rightmost tab; null when no tabs
 * remain.
 */
function pickNextActiveId(
    prevTabs: EditorTab[],
    closedIndex: number,
): string | null {
    const remaining = prevTabs.length - 1;
    if (remaining <= 0) return null;
    // After removing closedIndex, the right neighbor's NEW index is
    // closedIndex (since everything to the right shifts left). If
    // closedIndex was at the end, fall back to the new last tab
    // (which is the old left neighbor).
    if (closedIndex < remaining) {
        return prevTabs[closedIndex + 1].id;
    }
    return prevTabs[closedIndex - 1].id;
}

function pushRecentlyClosed(
    list: ClosedTab[],
    entry: ClosedTab,
): ClosedTab[] {
    const next = [...list, entry];
    if (next.length > MAX_RECENTLY_CLOSED) {
        return next.slice(next.length - MAX_RECENTLY_CLOSED);
    }
    return next;
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
 *   2. Tab `id`s are unique within a pane (the helper that mints ids
 *      uses uuids; activate-existing prevents accidental dup-by-path).
 *   3. `recentlyClosed.length <= MAX_RECENTLY_CLOSED`.
 *   4. `MarkDirty` / `ClearDirty` emit their events only on actual
 *      transitions (idempotent).
 */
export function update(
    state: EditorPaneState,
    command: EditorPaneCommand,
): ReducerResult {
    switch (command.type) {
        case "OpenFile": {
            const canon = canonicalizePath(command.path);
            const existing = findTabByPath(state, canon);
            const requestedMode = command.mode ?? "pinned";
            if (existing) {
                // If the existing tab is a preview and the caller asked
                // for a pinned open (e.g. tree double-click on the file
                // that's currently in preview), pin it as part of the
                // activation. Other transitions (preview→preview,
                // pinned→pinned, pinned→preview) are no-ops for isPreview.
                const shouldPin = existing.isPreview && requestedMode === "pinned";
                const existingIdx = state.tabs.findIndex((t) => t.id === existing.id);
                const updatedTab = shouldPin
                    ? { ...existing, isPreview: false }
                    : existing;
                const nextTabs = shouldPin
                    ? [
                          ...state.tabs.slice(0, existingIdx),
                          updatedTab,
                          ...state.tabs.slice(existingIdx + 1),
                      ]
                    : state.tabs;
                if (state.activeTabId === existing.id && !shouldPin) {
                    // Already active and no pin transition → no state change.
                    return {
                        state,
                        events: [
                            {
                                type: "TabActivated",
                                tabId: existing.id,
                                filePath: existing.filePath,
                            },
                        ],
                    };
                }
                return {
                    state: {
                        ...state,
                        tabs: nextTabs,
                        activeTabId: existing.id,
                    },
                    events: [
                        {
                            type: "TabActivated",
                            tabId: existing.id,
                            filePath: existing.filePath,
                        },
                    ],
                };
            }

            // New tab. In preview mode, replace the existing preview tab if
            // one exists — only ONE preview tab per pane. Pinned mode always
            // appends. Default is "pinned" — that's the safe choice for
            // programmatic callers (ReopenLastClosed, tests, future drag-
            // drop). Tree single-click is the only caller that should pass
            // `mode: "preview"` explicitly.
            if (requestedMode === "preview") {
                const previewIdx = state.tabs.findIndex((t) => t.isPreview);
                if (previewIdx >= 0) {
                    // Replace the preview slot's contents in place. Reset
                    // load state so the view re-fetches. The tab id stays
                    // the same so view-side CodeMirror state for OTHER
                    // tabs is unaffected.
                    const newPreview: EditorTab = {
                        ...state.tabs[previewIdx],
                        filePath: canon,
                        language: command.language ?? deriveLanguage(canon),
                        readOnly: false,
                        dirty: false,
                        contentHash: "",
                        loadError: null,
                        contentLoaded: false,
                        isPreview: true,
                    };
                    const nextTabs = [
                        ...state.tabs.slice(0, previewIdx),
                        newPreview,
                        ...state.tabs.slice(previewIdx + 1),
                    ];
                    return {
                        state: {
                            ...state,
                            tabs: nextTabs,
                            activeTabId: newPreview.id,
                        },
                        events: [
                            {
                                type: "TabActivated",
                                tabId: newPreview.id,
                                filePath: newPreview.filePath,
                            },
                        ],
                    };
                }
            }

            const tab = makeTab(command.path, command.language, requestedMode === "preview");
            const nextTabs = [...state.tabs, tab];
            return {
                state: {
                    ...state,
                    tabs: nextTabs,
                    activeTabId: tab.id,
                },
                events: [
                    {
                        type: "TabOpened",
                        tabId: tab.id,
                        filePath: tab.filePath,
                        atIndex: nextTabs.length - 1,
                    },
                ],
            };
        }

        case "PinTab": {
            const idx = findTabIndex(state, command.tabId);
            if (idx < 0 || !state.tabs[idx].isPreview) {
                // Defensive: pinning a non-preview tab is a no-op.
                return { state, events: [] };
            }
            const nextTab = { ...state.tabs[idx], isPreview: false };
            const nextTabs = [
                ...state.tabs.slice(0, idx),
                nextTab,
                ...state.tabs.slice(idx + 1),
            ];
            return { state: { ...state, tabs: nextTabs }, events: [] };
        }

        case "CloseTab": {
            const idx = findTabIndex(state, command.tabId);
            if (idx < 0) {
                // Defensive no-op — closing an already-closed tab is
                // benign (double-click on the × button, stale IPC).
                return { state, events: [] };
            }
            const tab = state.tabs[idx];
            if (tab.dirty && !command.force) {
                // The view shows a confirm modal; on confirm it
                // re-dispatches the same command with `force: true`.
                return {
                    state,
                    events: [
                        {
                            type: "RequestDirtyConfirm",
                            tabId: tab.id,
                            originalCommand: command,
                        },
                    ],
                };
            }
            const nextTabs = [...state.tabs.slice(0, idx), ...state.tabs.slice(idx + 1)];
            const wasActive = state.activeTabId === tab.id;
            const nextActiveId = wasActive
                ? pickNextActiveId(state.tabs, idx)
                : state.activeTabId;
            const closedEntry: ClosedTab = {
                filePath: tab.filePath,
                closedAt: Date.now(),
            };
            const nextState: EditorPaneState = {
                tabs: nextTabs,
                activeTabId: nextActiveId,
                recentlyClosed: pushRecentlyClosed(state.recentlyClosed, closedEntry),
            };
            const events: EditorPaneEvent[] = [
                { type: "TabClosed", tabId: tab.id, filePath: tab.filePath },
            ];
            if (wasActive && nextActiveId != null) {
                const newActive = nextTabs.find((t) => t.id === nextActiveId)!;
                events.push({
                    type: "TabActivated",
                    tabId: newActive.id,
                    filePath: newActive.filePath,
                });
            }
            return { state: nextState, events };
        }

        case "SwitchTab": {
            const idx = findTabIndex(state, command.tabId);
            if (idx < 0) {
                // Defensive — clicking a stale tab id (e.g. a tab
                // that was closed between render and click) is a
                // no-op, not a crash.
                return { state, events: [] };
            }
            if (state.activeTabId === command.tabId) {
                return { state, events: [] };
            }
            const tab = state.tabs[idx];
            return {
                state: { ...state, activeTabId: tab.id },
                events: [
                    { type: "TabActivated", tabId: tab.id, filePath: tab.filePath },
                ],
            };
        }

        case "ReorderTab": {
            const idx = findTabIndex(state, command.tabId);
            if (idx < 0) return { state, events: [] };
            if (state.tabs.length <= 1) return { state, events: [] };
            const clamped = Math.max(
                0,
                Math.min(state.tabs.length - 1, command.toIndex),
            );
            if (clamped === idx) return { state, events: [] };
            const next = [...state.tabs];
            const [moved] = next.splice(idx, 1);
            next.splice(clamped, 0, moved);
            return { state: { ...state, tabs: next }, events: [] };
        }

        case "MarkDirty": {
            const idx = findTabIndex(state, command.tabId);
            if (idx < 0) return { state, events: [] };
            const tab = state.tabs[idx];
            if (tab.dirty) {
                // Already dirty → no event re-emission. The view
                // model's title `*` is already on; no listener needs
                // a redundant signal.
                return { state, events: [] };
            }
            // Editing a preview tab promotes it to pinned (matches
            // VS Code's behavior — once you've started editing, the
            // tab should survive the next preview-mode tree click).
            const nextTab: EditorTab = { ...tab, dirty: true, isPreview: false };
            const nextTabs = [
                ...state.tabs.slice(0, idx),
                nextTab,
                ...state.tabs.slice(idx + 1),
            ];
            return {
                state: { ...state, tabs: nextTabs },
                events: [{ type: "TabDirtied", tabId: tab.id }],
            };
        }

        case "ClearDirty": {
            const idx = findTabIndex(state, command.tabId);
            if (idx < 0) return { state, events: [] };
            const tab = state.tabs[idx];
            if (!tab.dirty) {
                return { state, events: [] };
            }
            const nextTab: EditorTab = { ...tab, dirty: false };
            const nextTabs = [
                ...state.tabs.slice(0, idx),
                nextTab,
                ...state.tabs.slice(idx + 1),
            ];
            return {
                state: { ...state, tabs: nextTabs },
                events: [{ type: "TabSaved", tabId: tab.id }],
            };
        }

        case "TabContentLoaded": {
            const idx = findTabIndex(state, command.tabId);
            if (idx < 0) return { state, events: [] };
            const tab = state.tabs[idx];
            const nextTab: EditorTab = {
                ...tab,
                contentLoaded: true,
                contentHash: command.contentHash,
                loadError: null,
                readOnly: command.readOnly ?? tab.readOnly,
            };
            const nextTabs = [
                ...state.tabs.slice(0, idx),
                nextTab,
                ...state.tabs.slice(idx + 1),
            ];
            return { state: { ...state, tabs: nextTabs }, events: [] };
        }

        case "TabContentLoadFailed": {
            const idx = findTabIndex(state, command.tabId);
            if (idx < 0) return { state, events: [] };
            const tab = state.tabs[idx];
            // Preserve contentLoaded if it was already true — only the
            // initial-load failure path needs to flip it to false. For
            // operational failures (e.g. save errors), the disk-side
            // content the view holds is still valid and the centered
            // error panel must NOT replace CodeMirror; the small top
            // banner picks the error up via the loadError accessor.
            const nextTab: EditorTab = {
                ...tab,
                loadError: command.error,
                // Stay loaded if we already were; only flip to false
                // for never-loaded tabs (where contentLoaded is already
                // false anyway, so this is a no-op for that case).
                contentLoaded: tab.contentLoaded,
            };
            const nextTabs = [
                ...state.tabs.slice(0, idx),
                nextTab,
                ...state.tabs.slice(idx + 1),
            ];
            return { state: { ...state, tabs: nextTabs }, events: [] };
        }

        case "ReopenLastClosed": {
            if (state.recentlyClosed.length === 0) {
                return { state, events: [] };
            }
            const last = state.recentlyClosed[state.recentlyClosed.length - 1];
            const trimmed = state.recentlyClosed.slice(0, -1);
            // Re-route through the OpenFile pathway so we get the
            // standard activate-if-exists + TabOpened semantics. The
            // popped entry is removed BEFORE the OpenFile reduces so
            // a tab that happens to still be in the list (e.g. it
            // was reopened by another route between close+reopen)
            // just activates.
            const sub = update(
                { ...state, recentlyClosed: trimmed },
                { type: "OpenFile", path: last.filePath },
            );
            return sub;
        }

        case "HydrateFromMeta":
        case "HydrateFromDefaults": {
            const fromDefaults = command.type === "HydrateFromDefaults";
            const tabs: EditorTab[] = command.tabs.map((t) => ({
                id: t.id,
                filePath: canonicalizePath(t.filePath),
                language: t.language ?? deriveLanguage(t.filePath),
                readOnly: t.readOnly ?? false,
                dirty: false,
                contentHash: "",
                loadError: null,
                contentLoaded: false,
                // Hydrated tabs are always pinned — a persisted preview
                // wouldn't survive the round-trip semantically (the user
                // committed by closing/reopening the session).
                isPreview: false,
            }));
            // Enforce invariant 1 — activeTabId must point at a tab
            // in the list, or be null when empty.
            const activeStillPresent =
                command.activeTabId != null &&
                tabs.some((t) => t.id === command.activeTabId);
            const activeId = activeStillPresent
                ? command.activeTabId
                : tabs.length > 0
                  ? tabs[0].id
                  : null;
            const nextState: EditorPaneState = {
                tabs,
                activeTabId: activeId,
                recentlyClosed: state.recentlyClosed,
            };
            return {
                state: nextState,
                events: [
                    {
                        type: "TabsRestored",
                        tabIds: tabs.map((t) => t.id),
                        activeTabId: activeId,
                        fromDefaults,
                    },
                ],
            };
        }

        case "OpenScratch": {
            const canon = canonicalizePath(command.filePath);
            // If a scratch tab already points to this file, just activate it.
            const existing = state.tabs.find((t) => t.filePath === canon && t.isScratch);
            if (existing) {
                return {
                    state: { ...state, activeTabId: existing.id },
                    events: [{ type: "TabActivated", tabId: existing.id, filePath: existing.filePath }],
                };
            }
            const tab: EditorTab = {
                id: newTabId(),
                filePath: canon,
                language: command.language ?? "markdown",
                readOnly: false,
                dirty: false,
                contentHash: "",
                loadError: null,
                contentLoaded: false,
                isPreview: false,
                isScratch: true,
                scratchId: command.scratchId,
                displayName: command.displayName,
            };
            const nextTabs = [...state.tabs, tab];
            return {
                state: { ...state, tabs: nextTabs, activeTabId: tab.id },
                events: [{ type: "TabOpened", tabId: tab.id, filePath: tab.filePath, atIndex: nextTabs.length - 1 }],
            };
        }

        case "PromoteScratch": {
            const idx = findTabIndex(state, command.tabId);
            if (idx < 0) return { state, events: [] };
            const tab = state.tabs[idx];
            const nextTab: EditorTab = {
                ...tab,
                filePath: canonicalizePath(command.newPath),
                language: deriveLanguage(command.newPath),
                isScratch: false,
                scratchId: undefined,
                displayName: undefined,
                dirty: false,
            };
            const nextTabs = [
                ...state.tabs.slice(0, idx),
                nextTab,
                ...state.tabs.slice(idx + 1),
            ];
            return { state: { ...state, tabs: nextTabs }, events: [{ type: "TabSaved", tabId: tab.id }] };
        }

        case "RenameFile": {
            const canonOld = canonicalizePath(command.oldPath);
            const canonNew = canonicalizePath(command.newPath);
            const idx = state.tabs.findIndex((t) => t.filePath === canonOld);
            if (idx < 0) return { state, events: [] };
            const tab = state.tabs[idx];
            const nextTab: EditorTab = { ...tab, filePath: canonNew };
            const nextTabs = [
                ...state.tabs.slice(0, idx),
                nextTab,
                ...state.tabs.slice(idx + 1),
            ];
            return { state: { ...state, tabs: nextTabs }, events: [] };
        }
    }
}

// ─────────────────────────────────────────────────────────────────────
// Slot store
// ─────────────────────────────────────────────────────────────────────

interface Slot {
    state: EditorPaneState;
}

const slots = new Map<string, Slot>();

/**
 * Event sink — installed by the view (Phase 1B) and by the saga
 * (Phase 1C). The default is a no-op so tests run without DOM. The
 * sink receives the events array from a single dispatch call.
 *
 * Signature differs from slices #4/#9 (which pass blockId + single
 * event) — the editor saga needs the full event batch atomically so
 * it can write `editor:tabs` + `editor:active_tab_id` in one block-meta
 * mutation rather than racing N separate writes. The `blockId` is
 * already part of every event's downstream lookup via the dispatch
 * site that emitted the batch.
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
