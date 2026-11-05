// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Files pane: a toolbar with history and a breadcrumb, Places
 * on the left, a sortable details list, and a status line.
 * docs/specs/SPEC_FILE_BROWSER_PANE_2026_10_01.md §5.
 *
 * The list is windowed by hand (fixed-height rows, §6.5): only the rows in
 * view, plus a few either side, are in the DOM, so a 100,000-entry folder
 * scrolls like a small one. Keyboard follows the ARIA grid pattern with focus
 * separate from selection (§5.3).
 */

import type { PaneTabHostContext } from "@/app/store/pane-tab-registry";
import { ConfirmModal } from "@/app/element/confirm-modal";
import { ContextMenu, type ContextMenuItem } from "@/app/components/context-menu";
import { formatBytes } from "@/util/format-bytes";
import { holdPaneContent, trackPaneContent } from "@/app/store/pane-content-holds";
import { readZoom } from "@/app/store/zoom-factor";
import type { FsEntry } from "@/types/rpc/FsEntry";
import type { FsGitState } from "@/types/rpc/FsGitState";
import { childKey, touched, touchesUnder, type Touch } from "@/app/store/touched-files";
import { isMacOS } from "@/util/platformutil";
import { createEffect, createMemo, createSignal, For, Match, on, onCleanup, onMount, Show, Switch, type JSX } from "solid-js";
import { errorText, type FilesModel, windowsNames } from "./files-model";
import { closeOwnTab, openEntryWithOs, openFolderInNewTab, openInEditor, openInPane, openTargetOf, openWithOs } from "./files-open";
import { FilesPreview } from "./files-preview";
import { cachedThumbnail, hasThumbnail, thumbnail } from "./files-thumbs";
import { TypeIcon } from "./files-type-icon";
import { beginPathDrag, endPathDrag, pathDragPaths, pathDragSource, registerFileDropTarget } from "@/app/drag/file-drop";
import { nameProblem, samePath, stemLength } from "./files-path";
import { clickRow, folderAt, moveFocus, selectAll, toggleFocused } from "./files-selection";
import { extensionOf, type SortKey } from "./files-sort";
import { FilesFooter } from "./FilesFooter";
import { FilesPlaces } from "./FilesPlaces";
import { FilesToolbar } from "./FilesToolbar";
import { TypeAhead } from "./typeahead";
import { buildFolderMenu, buildRowMenu } from "./files-menus";
import { mentionInAgent, trackAgentFocus } from "./files-agent-mention";
import "./files.scss";
import { isGlobalKey, paneCommandFor, registerPaneCommandRunner } from "@/app/keybindings";
import { isEditableTarget } from "@/util/focusutil";
import { Button } from "@/app/element/ui";
import { tip } from "@/app/element/tip";

const ROW_HEIGHT = 24;
/** A grid tile's box (thumbnail and a two-line name). */
const TILE_W = 112;
const TILE_H = 132;
const OVERSCAN = 8;

const COLUMNS: { key: SortKey; label: string; class: string }[] = [
    { key: "name", label: "Name", class: "files-col-name" },
    { key: "modified", label: "Modified", class: "files-col-modified" },
    { key: "size", label: "Size", class: "files-col-size" },
    { key: "kind", label: "Kind", class: "files-col-kind" },
];

/** "just now", "5 min ago", "3 h ago", "yesterday", else the date. */
export function formatModified(ms: number | undefined, now: number = Date.now()): string {
    if (ms == null) return "";
    const diff = now - ms;
    if (diff < 0) return new Date(ms).toLocaleDateString();
    const min = Math.floor(diff / 60_000);
    if (min < 1) return "just now";
    if (min < 60) return `${min} min ago`;
    const h = Math.floor(min / 60);
    if (h < 24) return `${h} h ago`;
    if (h < 48) return "yesterday";
    return new Date(ms).toLocaleDateString(undefined, { year: "numeric", month: "short", day: "numeric" });
}

function kindOf(e: FsEntry): string {
    if (e.is_dir) return e.is_symlink ? "Folder link" : "Folder";
    const ext = extensionOf(e.name);
    const base = ext ? `${ext.toUpperCase()} file` : "File";
    return e.is_symlink ? `${base} link` : base;
}

type Confirm = { title: string; message: string; confirmLabel: string; onConfirm: () => void };

export { mentionToken } from "./files-agent-mention";

export function FilesView(props: { model: FilesModel; ctx: PaneTabHostContext }): JSX.Element {
    const model = props.model;
    const ctx = props.ctx;

    // Settled-content contract (§6.5): the pane counts as painted once the
    // first page of the first listing is on screen, or it has said why not.
    onCleanup(trackPaneContent(model.blockId));
    const release = holdPaneContent(model.blockId);
    onCleanup(release);
    model.onFirstSettled = release;

    const [scrollTop, setScrollTop] = createSignal(0);
    const [viewHeight, setViewHeight] = createSignal(400);
    const [viewWidth, setViewWidth] = createSignal(600);
    const [editingPath, setEditingPath] = createSignal(false);
    const [filterOpen, setFilterOpen] = createSignal(false);
    let filterInput: HTMLInputElement | undefined;
    const openFilter = (): void => {
        setFilterOpen(true);
        queueMicrotask(() => {
            filterInput?.focus();
            filterInput?.select();
        });
    };
    const closeFilter = (): void => {
        model.setFilter("");
        setFilterOpen(false);
        listEl?.focus();
    };
    // Navigating clears the filter (the model does); close the box with it.
    createEffect(
        on(model.path, () => {
            if (!model.filter()) setFilterOpen(false);
        })
    );
    const [menu, setMenu] = createSignal<{ x: number; y: number; items: ContextMenuItem[] } | null>(null);
    const [confirm, setConfirm] = createSignal<Confirm | null>(null);
    const typeahead = new TypeAhead();
    let listEl: HTMLDivElement | undefined;
    let pathInput: HTMLInputElement | undefined;
    let crumbsEl: HTMLElement | undefined;

    // Files dropped on the pane go into the folder it shows: an OS drop or
    // another pane's row is copied; a row from another Files pane on the
    // same drive is moved (§8.2).
    onMount(() => {
        const dispose = registerFileDropTarget(model.blockId, {
            needsPaths: true,
            onNoPaths: () => model.setStatus({ text: "Those files have no path on disk, so they can't be copied here.", tone: "error" }),
            accept(drag) {
                if (model.phase() !== "ready") return { ok: false, reason: "Open a folder first" };
                // From this same pane only a folder row is somewhere new; the
                // verdict is asked once per drag, so the drop itself checks.
                if (pathDragSource() === model.blockId) return { ok: true, message: "Drop on a folder to move it there", icon: "fa-folder-open" };
                const what = drag.count === 1 ? (drag.names?.[0] ?? "1 item") : `${drag.count} items`;
                const inApp = pathDragPaths();
                const kind = inApp ? model.dropKind(inApp, pathDragSource() != null) : "copy";
                return kind === "move"
                    ? { ok: true, message: `Move ${what} here`, icon: "fa-right-left" }
                    : { ok: true, message: `Copy ${what} here`, icon: "fa-copy" };
            },
            async drop({ paths }) {
                // Onto a folder row: into that folder; anywhere else: here.
                const row = dropRow();
                setDropRow(null);
                if (paths.length === 0) return;
                if (!row && pathDragSource() === model.blockId) return; // Already here.
                const dest = row ? model.pathOf(row) : model.path();
                // A folder dropped onto itself goes nowhere.
                const sources = paths.filter((p) => !samePath(p, dest));
                if (sources.length === 0) return;
                await model.transfer(model.dropKind(sources, pathDragSource() != null, dest), sources, dest);
            },
        });
        onCleanup(dispose);
    });

    // The folder row under a drag, which a drop goes into (§8.2).
    const [dropRow, setDropRow] = createSignal<string | null>(null);
    const clearDropRow = () => setDropRow(null);
    window.addEventListener("dragend", clearDropRow, true);
    onCleanup(() => window.removeEventListener("dragend", clearDropRow, true));
    const onRowDragOver = (entry: FsEntry): void => {
        const dragged = pathDragPaths();
        // Not onto a folder that is itself being dragged.
        const self = dragged?.some((p) => samePath(p, model.pathOf(entry.name)));
        setDropRow(entry.is_dir && !self ? entry.name : null);
    };

    const onRowDragStart = (e: DragEvent, entry: FsEntry): void => {
        if (!e.dataTransfer) return;
        // A host's paths mean nothing to this computer's panes and apps; a confined folder's are not handed out.
        if (model.connection() || model.root()) {
            e.preventDefault();
            return;
        }
        if (!model.selection().names.has(entry.name)) {
            model.setSelection({ names: new Set([entry.name]), focus: entry.name, anchor: entry.name });
        }
        const paths = model.selectedEntries().map((x) => model.pathOf(x.name));
        beginPathDrag(e.dataTransfer, paths, model.blockId);
    };

    trackAgentFocus();
    model.focusList = () => listEl?.focus();
    onCleanup(() => (model.focusList = null));

    onMount(() => {
        void model.start();
        if (listEl) {
            const ro = new ResizeObserver(() => {
                setViewHeight(listEl?.clientHeight ?? 400);
                setViewWidth(listEl?.clientWidth ?? 600);
            });
            ro.observe(listEl);
            onCleanup(() => ro.disconnect());
        }
    });

    createEffect(
        on(
            () => ctx.visibility(),
            (v) => {
                if (v === "active") model.onShown();
            }
        )
    );

    // A long path shows its end: the folder you're in matters more than the
    // drive it's on.
    createEffect(
        on(model.path, () => {
            queueMicrotask(() => {
                if (crumbsEl) crumbsEl.scrollLeft = crumbsEl.scrollWidth;
            });
        })
    );

    // OpenFiles' selection request, whenever it lands in the block's meta:
    // pane.open writes the meta in more than one step, so it can arrive
    // after the first listing (or come back after the pane cleared it).
    createEffect(
        on(
            () => ctx.meta()?.["files:select"],
            (req) => {
                if (req != null) model.applyRequestedSelection();
            }
        )
    );

    // A new folder starts at the top.
    createEffect(
        on(model.path, () => {
            if (listEl) listEl.scrollTop = 0;
            setScrollTop(0);
        })
    );

    const entries = model.entries;
    const order = model.order;
    // The window only changes when a row crosses an edge, not on every
    // scroll event (ReAgent on #4201).
    /** Tiles per grid row. */
    const gridCols = createMemo(() => Math.max(1, Math.floor((viewWidth() - 8) / TILE_W)));
    const grid = () => model.viewMode() === "grid";
    const range = createMemo(
        () => {
            if (grid()) {
                // Whole rows of tiles, a couple either side.
                const cols = gridCols();
                const firstRow = Math.max(0, Math.floor(scrollTop() / TILE_H) - 2);
                const rows = Math.ceil(viewHeight() / TILE_H) + 4;
                return { first: firstRow * cols, last: Math.min(entries().length, (firstRow + rows) * cols) };
            }
            const first = Math.max(0, Math.floor(scrollTop() / ROW_HEIGHT) - OVERSCAN);
            const count = Math.ceil(viewHeight() / ROW_HEIGHT) + OVERSCAN * 2;
            return { first, last: Math.min(entries().length, first + count) };
        },
        { first: 0, last: 0 },
        { equals: (a, b) => a.first === b.first && a.last === b.last }
    );
    // Rows are keyed by NAME: a string is the same key in every listing, so
    // scrolling, a re-sort or a re-list after a change on disk reuses the row
    // (and an open rename box) instead of remounting it.
    const visibleNames = createMemo(() => {
        const { first, last } = range();
        return order().slice(first, last);
    });
    const byName = createMemo(() => new Map(entries().map((e) => [e.name, e])));
    // A row's entry, kept while it leaves: a selection change in the update that drops its name
    // re-runs the row once more, and reading the <Show>'s accessor then throws (a stale read).
    const entryOf = (name: string) => createMemo<FsEntry | undefined>((prev) => byName().get(name) ?? prev) as () => FsEntry;
    // Files an agent changed in the last half hour (§8.4), by child name.
    // A minute's tick lets an old badge go without waiting for a change.
    const [minute, setMinute] = createSignal(Date.now());
    const minuteTimer = setInterval(() => setMinute(Date.now()), 60_000);
    onCleanup(() => clearInterval(minuteTimer));
    const touches = createMemo(() => {
        touched();
        return touchesUnder(model.path(), minute());
    });
    const touchOf = (name: string) => touches().get(childKey(model.path(), name));

    /** The one selected entry the preview shows (none for several). */
    const previewEntry = createMemo(() => {
        const names = model.selection().names;
        if (names.size !== 1) return null;
        const [name] = names;
        return byName().get(name) ?? null;
    });

    const focusIndex = createMemo(() => {
        const f = model.selection().focus;
        return f == null ? -1 : order().indexOf(f);
    });

    /** Keeps the focused row inside the scrolled view. */
    const reveal = (index: number): void => {
        if (!listEl || index < 0) return;
        const height = grid() ? TILE_H : ROW_HEIGHT;
        const top = grid() ? Math.floor(index / gridCols()) * TILE_H : index * ROW_HEIGHT;
        const header = ROW_HEIGHT;
        if (top < listEl.scrollTop) listEl.scrollTop = top;
        else if (top + height > listEl.scrollTop + listEl.clientHeight - header)
            listEl.scrollTop = top + height - listEl.clientHeight + header;
        // Move the window now rather than on the scroll event, so the row
        // (and a rename box on it) mounts in this same update.
        setScrollTop(listEl.scrollTop);
    };

    createEffect(
        on(model.revealRequest, (req) => {
            if (req) reveal(order().indexOf(req.name));
        })
    );

    const isMod = (e: KeyboardEvent | MouseEvent): boolean => (isMacOS() ? e.metaKey : e.ctrlKey);

    // ── Opening ────────────────────────────────────────────────────────────

    const openEntry = (entry: FsEntry): void => {
        const path = model.pathOf(entry.name);
        if (entry.is_dir) {
            void model.navigate(path);
            return;
        }
        const target = openTargetOf(entry.name);
        if (target !== "editor" && model.refuseInRoot("Opening this file")) return;
        const fail = (err: unknown) => model.setStatus({ text: `Couldn't open ${entry.name}: ${errorText(err)}`, tone: "error" });
        // On a host: in an editor on that host; nothing else opens there yet.
        if (model.connection()) {
            if (target === "editor") void openInPane("editor", path, model.blockId, model.connection()).catch(fail);
            else model.notOnHost("Opening programs and other files");
            return;
        }
        if (target === "editor") void openInEditor(model, path).catch(fail);
        else if (target === "os") void openWithOs(path, model.blockId).catch(fail);
        else
            setConfirm({
                title: `Run ${entry.name}?`,
                message: `${entry.name} is a program or shortcut. Opening it runs it.`,
                confirmLabel: "Run",
                onConfirm: () => void openWithOs(path, model.blockId).catch(fail),
            });
    };

    /** A folder in a new Files tab of this pane (Ctrl+Enter, middle-click,
     *  the menu); Ctrl+T opens the folder shown. */
    const openInNewTab = (dir: string): void => {
        if (model.refuseInRoot("A new tab")) return;
        void openFolderInNewTab(model.blockId, dir, model.connection() || undefined).catch((err) =>
            model.setStatus({ text: `Couldn't open a new tab: ${errorText(err)}`, tone: "error" })
        );
    };

    const askDeletePermanently = (list: FsEntry[]): void => {
        if (list.length === 0) return;
        const what = list.length === 1 ? `"${list[0].name}"` : `${list.length} items`;
        const folders = list.some((e) => e.is_dir) ? " Folders are deleted with everything in them." : "";
        setConfirm({
            title: `Delete ${what} permanently?`,
            message: `This can't be undone: it skips the Trash.${folders}`,
            confirmLabel: "Delete permanently",
            onConfirm: () => void model.deletePermanently(list),
        });
    };

    const copyPaths = (list: FsEntry[], quiet = false): void => {
        const text = list.map((e) => model.pathOf(e.name)).join("\n");
        void navigator.clipboard?.writeText(text).then(
            () => {
                if (!quiet) model.setStatus({ text: list.length === 1 ? "Copied the path" : `Copied ${list.length} paths`, tone: "info" }, 2500);
            },
            () => model.setStatus({ text: "Couldn't copy to the clipboard", tone: "error" })
        );
    };

    // ── Keyboard (§5.3.1) ──────────────────────────────────────────────────

    /** Runs a `files:*` command, for its keys and RunCommand alike; false if unknown. */
    const runFilesCommand = (command: string): boolean => {
        const sel = model.selection();
        switch (command) {
            case "files:back":
                model.goBack();
                return true;
            case "files:forward":
                model.goForward();
                return true;
            case "files:up":
                model.goUp();
                return true;
            case "files:openInNewTab":
                // Folders in new tabs; files open as usual.
                for (const entry of model.selectedEntries()) {
                    if (entry.is_dir) openInNewTab(model.pathOf(entry.name));
                    else openEntry(entry);
                }
                return true;
            case "files:newTabHere":
                openInNewTab(model.path());
                return true;
            case "files:closeTab":
                void closeOwnTab(model.blockId).then(
                    (closed) => {
                        if (!closed) model.setStatus({ text: "This is the pane's only tab. Use the pane's × to close it.", tone: "info" }, 3000);
                    },
                    (err) => model.setStatus({ text: `Couldn't close this tab: ${errorText(err)}`, tone: "error" })
                );
                return true;
            case "files:rename":
                if (sel.focus && sel.names.size === 1 && sel.names.has(sel.focus)) model.setRenaming(sel.focus);
                return true;
            case "files:refresh":
                model.refresh();
                return true;
            case "files:trash":
                void model.trash(model.selectedEntries());
                return true;
            case "files:deletePermanently":
                askDeletePermanently(model.selectedEntries());
                return true;
            case "files:selectAll":
                model.setSelection(selectAll(sel, order()));
                return true;
            case "files:undo":
                void model.undo();
                return true;
            case "files:copy":
                // Both: the files for a paste in the Files pane, the paths for anywhere else.
                model.copyToClipboard("copy", model.selectedEntries());
                copyPaths(model.selectedEntries(), true);
                return true;
            case "files:cut":
                model.copyToClipboard("cut", model.selectedEntries());
                return true;
            case "files:paste":
                void model.paste();
                return true;
            case "files:editPath":
                startEditingPath();
                return true;
            case "files:mention":
                mentionIn(model.selectedEntries());
                return true;
            case "files:newFolder":
                void model.createNew("dir");
                return true;
            case "files:filter":
                openFilter();
                return true;
        }
        return false;
    };
    onCleanup(registerPaneCommandRunner(model.blockId, runFilesCommand));

    // Files commands from anywhere in the pane (toolbar, sidebar), not only
    // the list: the global shortcuts that share these keys stand aside in
    // this pane. Text fields (filter, rename, path) keep their own keys, and
    // the list handles its own focus first.
    const onPaneKeyDown = (e: KeyboardEvent): void => {
        if (e.defaultPrevented || model.renaming() || editingPath() || isEditableTarget(e.target)) return;
        if (listEl?.contains(e.target as Node)) return;
        const command = paneCommandFor(e, "files");
        if (command && runFilesCommand(command)) {
            e.preventDefault();
            e.stopPropagation();
        }
    };

    const onKeyDown = (e: KeyboardEvent): void => {
        if (model.renaming() || editingPath()) return;
        const sel = model.selection();
        // In the grid, up and down move a row of tiles, left and right one.
        const step = grid() ? gridCols() : 1;
        const page = grid()
            ? Math.max(1, Math.floor((listEl?.clientHeight ?? 400) / TILE_H) - 1) * gridCols()
            : Math.max(1, Math.floor((listEl?.clientHeight ?? 400) / ROW_HEIGHT) - 2);
        const move = (m: { delta?: number; to?: number }) => {
            model.setSelection(moveFocus(sel, order(), m, { extend: e.shiftKey, focusOnly: isMod(e) && !e.shiftKey }));
            reveal(order().indexOf(model.selection().focus ?? ""));
        };
        const listKey = (): boolean => {
            if (e.key === "ArrowDown") move({ delta: step });
            else if (e.key === "ArrowUp") move({ delta: -step });
            else if (grid() && e.key === "ArrowRight" && !e.altKey) move({ delta: 1 });
            else if (grid() && e.key === "ArrowLeft" && !e.altKey) move({ delta: -1 });
            else if (e.key === "PageDown") move({ delta: page });
            else if (e.key === "PageUp") move({ delta: -page });
            else if (e.key === "Home") move({ to: 0 });
            else if (e.key === "End") move({ to: order().length - 1 });
            else if (e.key === "Enter") {
                const list = model.selectedEntries();
                if (list.length === 1) openEntry(list[0]);
                else list.filter((x) => !x.is_dir).forEach(openEntry);
            } else if (e.key === "Backspace") model.goBack();
            else if (e.key === "/" && !typeahead.active()) openFilter();
            else if (e.key === "Escape" && model.filter()) closeFilter();
            // Ctrl+Space toggles the focused row (as in Explorer); Space alone is
            // quick look, the preview panel (spec §5.3.1).
            else if (e.key === " " && isMod(e)) model.setSelection(toggleFocused(sel));
            else if (e.key === " " && !typeahead.active()) model.togglePreview();
            else if (e.key.length === 1 && !e.ctrlKey && !e.metaKey && !e.altKey) {
                const hit = typeahead.next(e.key, order(), sel.focus);
                if (hit) {
                    model.setSelection({ names: new Set([hit]), focus: hit, anchor: hit });
                    reveal(order().indexOf(hit));
                }
            } else return false;
            return true;
        };
        let handled = true;
        // Command keys are the `files:*` rows of the shortcut table, so they show in
        // the help pane. The list's own keys (arrows, paging, Enter, type-ahead, Esc)
        // stay here; with a modifier besides Shift, a global command wins (⌃⇧↑, ⌘/).
        const command = paneCommandFor(e, "files");
        handled = command ? runFilesCommand(command) : !((e.ctrlKey || e.metaKey || e.altKey) && isGlobalKey(e)) && listKey();
        if (handled) {
            e.preventDefault();
            e.stopPropagation();
        }
    };

    // ── Mouse ──────────────────────────────────────────────────────────────

    const onRowClick = (e: MouseEvent, entry: FsEntry): void => {
        listEl?.focus();
        model.setSelection(clickRow(model.selection(), order(), entry.name, { toggle: isMod(e), range: e.shiftKey }));
    };

    const mentionIn = (list: FsEntry[]): void => mentionInAgent(model, list);
    const rowMenu = (entry: FsEntry): ContextMenuItem[] => buildRowMenu(model, entry, { openEntry, openInNewTab, copyPaths, askDeletePermanently });
    const folderMenu = (): ContextMenuItem[] => buildFolderMenu(model, { openInNewTab });

    const onRowContextMenu = (e: MouseEvent, entry: FsEntry): void => {
        e.preventDefault();
        e.stopPropagation();
        if (!model.selection().names.has(entry.name)) {
            model.setSelection({ names: new Set([entry.name]), focus: entry.name, anchor: entry.name });
        }
        setMenu({ x: e.clientX, y: e.clientY, items: rowMenu(entry) });
    };

    const onListContextMenu = (e: MouseEvent): void => {
        e.preventDefault();
        // Like a row: stop here, or the click bubbles to the pane frame and the
        // generic pane menu opens on top of this one.
        e.stopPropagation();
        setMenu({ x: e.clientX, y: e.clientY, items: folderMenu() });
    };

    // ── Path editing (Ctrl+L) ──────────────────────────────────────────────

    const startEditingPath = (): void => {
        setEditingPath(true);
        queueMicrotask(() => {
            pathInput?.focus();
            pathInput?.select();
        });
    };

    const commitPath = (value: string): void => {
        setEditingPath(false);
        const v = value.trim();
        if (v && v !== model.path()) void model.navigate(v);
        listEl?.focus();
    };

    // ── Status line ────────────────────────────────────────────────────────

    const summary = createMemo(() => {
        const all = entries();
        const sel = model.selectedEntries().filter((e) => model.selection().names.has(e.name));
        const total = model.rawEntries().filter((e) => model.showHidden() || !e.hidden).length;
        const count = model.filter()
            ? `${all.length} of ${total} items match`
            : `${all.length} item${all.length === 1 ? "" : "s"}${model.partial() ? "…" : ""}`;
        if (sel.length === 0) return count;
        const bytes = sel.reduce((n, e) => n + (e.size ?? 0), 0);
        const size = sel.some((e) => !e.is_dir) ? ` · ${formatBytes(bytes)}` : "";
        return `${sel.length} selected${size} · ${count}`;
    });

    const toggleViewMode = (): void => {
        model.toggleViewMode();
        if (listEl) listEl.scrollTop = 0;
        setScrollTop(0);
    };

    const placeLabel = (path: string): string => model.protectedPlace(path)?.label ?? path;

    return (
        <div
            class="files-view"
            // CSS zoom from the shared `term:zoom`: everything scales together, and
            // the @container on this element measures in zoomed pixels.
            style={{ zoom: readZoom(props.ctx.meta()) }}
            // The folder shown, for tooling (verify-shortcuts.mjs's temp-tree guard).
            data-path={model.path()}
            data-connection={model.connection()}
            onContextMenu={(e) => e.preventDefault()}
            onKeyDown={onPaneKeyDown}
        >
            <FilesToolbar
                model={model}
                grid={grid}
                editingPath={editingPath}
                filterOpen={filterOpen}
                startEditingPath={startEditingPath}
                commitPath={commitPath}
                onStopEditing={() => setEditingPath(false)}
                openFilter={openFilter}
                closeFilter={closeFilter}
                onToggleViewMode={toggleViewMode}
                focusList={() => listEl?.focus()}
                pathInputRef={(el) => (pathInput = el)}
                crumbsRef={(el) => (crumbsEl = el)}
                filterRef={(el) => (filterInput = el)}
            />

            <div class="files-body">
                <FilesPlaces model={model} />

                <div
                    ref={listEl}
                    class="files-list"
                    role="grid"
                    aria-label="Files"
                    aria-multiselectable="true"
                    aria-rowcount={entries().length + 1}
                    aria-activedescendant={focusIndex() >= 0 ? `files-${model.blockId}-row-${focusIndex()}` : undefined}
                    tabIndex={0}
                    onKeyDown={onKeyDown}
                    onScroll={(e) => setScrollTop(e.currentTarget.scrollTop)}
                    // Over blank space, the header or a notice: no folder row is
                    // the target any more (ReAgent on #4224). A row's own
                    // dragover runs first and sets or clears it.
                    onDragOver={(e) => {
                        if (!(e.target instanceof Element && e.target.closest(".files-rows .files-row, .files-tile"))) setDropRow(null);
                    }}
                    onDragLeave={(e) => {
                        if (!(e.relatedTarget instanceof Node && e.currentTarget.contains(e.relatedTarget))) setDropRow(null);
                    }}
                    onContextMenu={onListContextMenu}
                    // Windows: a middle press over the list starts autoscroll, and no auxclick follows.
                    onMouseDown={(e) => e.button === 1 && folderAt(e.target, byName()) && e.preventDefault()}
                    onAuxClick={(e) => {
                        // Middle-click a folder: open it in a new tab.
                        const entry = e.button === 1 && folderAt(e.target, byName());
                        if (!entry) return;
                        e.preventDefault();
                        openInNewTab(model.pathOf(entry.name));
                    }}
                    onClick={(e) => {
                        if (e.target === e.currentTarget) model.setSelection({ ...model.selection(), names: new Set() });
                    }}
                >
                    <div class="files-row files-header" role="row" aria-rowindex={1}>
                        <For each={COLUMNS}>
                            {(col) => (
                                <Button
                                    tone="quiet"
                                    role="columnheader"
                                    class={`files-cell ${col.class}`}
                                    aria-sort={model.sortKey() === col.key ? (model.sortDir() === "asc" ? "ascending" : "descending") : "none"}
                                    onClick={() => model.setSort(col.key)}
                                >
                                    {col.label}
                                    <Show when={model.sortKey() === col.key}>
                                        <i class={`fa fa-caret-${model.sortDir() === "asc" ? "up" : "down"} files-sort-mark`} aria-hidden="true" />
                                    </Show>
                                </Button>
                            )}
                        </For>
                    </div>

                    <Switch>
                        <Match when={model.phase() === "gated"}>
                            <div class="files-notice" role="status">
                                <p>
                                    macOS will ask to let AgentMux open your {placeLabel(model.path())} folder. That's so it can show
                                    what's inside. It asks again after each AgentMux update.
                                </p>
                                <Button class="files-button" onClick={() => void model.navigate(model.path(), { push: false, consented: true })}>
                                    Open {placeLabel(model.path())}
                                </Button>
                            </div>
                        </Match>
                        <Match when={model.phase() === "loading"}>
                            <Show when={model.slow()}>
                                <div class="files-notice" role="status">
                                    {model.protectedPlace(model.path())
                                        ? "Waiting for you to answer macOS's prompt…"
                                        : "Loading…"}
                                </div>
                            </Show>
                        </Match>
                        <Match when={model.phase() === "error"}>
                            <div class="files-notice files-notice-error" role="alert">
                                <p>{errorMessage(model.error()?.kind, model.error()?.message, placeLabel(model.path()))}</p>
                                <Button class="files-button" onClick={() => model.refresh()}>
                                    Try again
                                </Button>
                            </div>
                        </Match>
                        <Match when={model.phase() === "ready" && entries().length === 0 && !model.partial()}>
                            <div class="files-notice">
                                {model.filter()
                                    ? `Nothing here matches “${model.filter()}”.`
                                    : model.rawEntries().length > 0
                                      ? "Only hidden files here."
                                      : "This folder is empty."}
                            </div>
                        </Match>
                    </Switch>

                    <Show when={model.phase() === "ready" && grid()}>
                        <div class="files-grid" style={{ height: `${Math.ceil(entries().length / gridCols()) * TILE_H}px` }}>
                            <For each={visibleNames()}>
                                {(name, i) => {
                                    const entry = entryOf(name);
                                    const index = () => range().first + i();
                                    return (
                                        <Show when={byName().has(name)}>
                                            <GridTile
                                                model={model}
                                                entry={entry()}
                                                index={index()}
                                                left={(index() % gridCols()) * TILE_W}
                                                top={Math.floor(index() / gridCols()) * TILE_H}
                                                selected={model.selection().names.has(name)}
                                                focused={model.selection().focus === name}
                                                renaming={model.renaming() === name}
                                                git={model.gitStateOf().get(name)}
                                                touch={touchOf(name)}
                                                dropTarget={dropRow() === name}
                                                onClick={(e) => onRowClick(e, entry())}
                                                onOpen={() => openEntry(entry())}
                                                onContextMenu={(e) => onRowContextMenu(e, entry())}
                                                onDragStart={(e) => onRowDragStart(e, entry())}
                                                onDragOver={() => onRowDragOver(entry())}
                                                onDragEnd={() => endPathDrag()}
                                                onRenameDone={() => listEl?.focus()}
                                            />
                                        </Show>
                                    );
                                }}
                            </For>
                        </div>
                    </Show>
                    <Show when={model.phase() === "ready" && !grid()}>
                        <div class="files-rows" style={{ height: `${entries().length * ROW_HEIGHT}px` }}>
                            <For each={visibleNames()}>
                                {(name, i) => {
                                    const entry = entryOf(name);
                                    return (
                                        <Show when={byName().has(name)}>
                                            <FileRow
                                                model={model}
                                                entry={entry()}
                                                index={range().first + i()}
                                                selected={model.selection().names.has(name)}
                                                focused={model.selection().focus === name}
                                                renaming={model.renaming() === name}
                                                git={model.gitStateOf().get(name)}
                                                touch={touchOf(name)}
                                                onClick={(e) => onRowClick(e, entry())}
                                                onOpen={() => openEntry(entry())}
                                                onContextMenu={(e) => onRowContextMenu(e, entry())}
                                                onDragStart={(e) => onRowDragStart(e, entry())}
                                                onDragOver={() => onRowDragOver(entry())}
                                                dropTarget={dropRow() === name}
                                                onDragEnd={() => endPathDrag()}
                                                onRenameDone={() => listEl?.focus()}
                                            />
                                        </Show>
                                    );
                                }}
                            </For>
                        </div>
                    </Show>
                </div>
                <Show when={model.showPreview()}>
                    <FilesPreview
                        entry={previewEntry()}
                        pathOf={(name) => model.pathOf(name)}
                        blockId={model.blockId}
                        onOpen={openEntry}
                        onOpenWithOs={(entry) => openEntryWithOs(model, entry.name)}
                    />
                </Show>
            </div>

            <FilesFooter model={model} summary={summary} />

            <Show when={menu()}>
                {(m) => <ContextMenu items={m().items} x={m().x} y={m().y} onClose={() => setMenu(null)} />}
            </Show>
            <Show when={confirm()}>
                {(c) => (
                    <ConfirmModal
                        open={true} scope="pane" destructive
                        title={c().title} description={c().message}
                        confirmLabel={c().confirmLabel}
                        onConfirm={() => {
                            c().onConfirm();
                            setConfirm(null);
                            listEl?.focus();
                        }}
                        onCancel={() => {
                            setConfirm(null);
                            listEl?.focus();
                        }}
                    />
                )}
            </Show>
        </div>
    );
}

/** The letter beside a name and what it means (spec §12, git decorations).
 *  A folder shows the most pressing state of what's inside it. */
const GIT_MARKS: Record<Exclude<FsGitState, "ignored">, { letter: string; title: string }> = {
    untracked: { letter: "U", title: "Untracked: new, not yet added to git" },
    added: { letter: "A", title: "Added to git, not yet committed" },
    renamed: { letter: "R", title: "Renamed" },
    deleted: { letter: "D", title: "Deleted" },
    modified: { letter: "M", title: "Changed since the last commit" },
    conflicted: { letter: "!", title: "Merge conflict" },
};

/** What went wrong listing a folder, in words, never as an empty folder
 *  (§9.1.4 item 4). */
export function errorMessage(kind: string | undefined, message: string | undefined, place: string): string {
    switch (kind) {
        case "not_found":
            return "This folder doesn't exist any more.";
        case "not_a_directory":
            return "This is a file, not a folder.";
        case "permission_denied":
            return "You don't have permission to open this folder.";
        case "os_blocked":
            return `macOS blocked AgentMux from reading ${place}. Open System Settings → Privacy & Security → Files & Folders, turn AgentMux on, then try again.`;
        default:
            return `Couldn't open this folder: ${message ?? "unknown error"}`;
    }
}

function FileRow(props: {
    model: FilesModel;
    entry: FsEntry;
    index: number;
    selected: boolean;
    focused: boolean;
    renaming: boolean;
    onClick: (e: MouseEvent) => void;
    onOpen: () => void;
    onContextMenu: (e: MouseEvent) => void;
    onRenameDone: () => void;
    onDragStart: (e: DragEvent) => void;
    onDragEnd: () => void;
    git?: FsGitState;
    touch?: Touch;
    onDragOver: () => void;
    dropTarget: boolean;
}): JSX.Element {
    return (
        <div
            id={`files-${props.model.blockId}-row-${props.index}`}
            class="files-row"
            classList={{
                "files-row-selected": props.selected,
                "files-row-focused": props.focused,
                "files-row-hidden": props.entry.hidden,
                "files-row-ignored": props.git === "ignored",
                "files-row-droptarget": props.dropTarget,
            }}
            role="row"
            aria-rowindex={props.index + 2}
            aria-selected={props.selected}
            style={{ top: `${props.index * ROW_HEIGHT}px` }}
            onClick={props.onClick}
            onDblClick={props.onOpen}
            onContextMenu={props.onContextMenu}
            draggable={!props.renaming}
            onDragStart={props.onDragStart}
            onDragOver={props.onDragOver}
            onDragEnd={props.onDragEnd}
            {...tip(props.entry.error ?? (props.entry.link_target ? `→ ${props.entry.link_target}` : undefined))}
        >
            <div class="files-cell files-col-name" role="gridcell">
                <span class="files-icon-box"><TypeIcon model={props.model} entry={props.entry} cssPx={16} glyphClass="files-icon" /></span>
                <Show when={props.entry.is_symlink}>
                    <i class="fa fa-share files-link-mark" aria-label="link" />
                </Show>
                <Show when={props.touch}>
                    {(t) => (
                        <span
                            class="files-touch"
                            style={t().color ? { "background-color": t().color } : undefined}
                            {...tip(`${props.entry.is_dir ? "Something inside was changed" : "Changed"} by ${t().agentName}, ${formatModified(t().at)} (${t().tool})`)}
                            aria-label={`changed by ${t().agentName}`}
                        />
                    )}
                </Show>
                <Show when={props.renaming} fallback={<span class="files-name">{props.entry.name}</span>}>
                    <RenameInput model={props.model} entry={props.entry} onDone={props.onRenameDone} />
                </Show>
                <Show when={props.entry.error}>
                    <i class="fa fa-triangle-exclamation files-entry-error" aria-label={props.entry.error} />
                </Show>
                <Show when={props.git && props.git !== "ignored" && GIT_MARKS[props.git]}>
                    {(mark) => (
                        <span class={`files-git files-git-${props.git}`} {...tip(mark().title)} aria-label={mark().title}>
                            {mark().letter}
                        </span>
                    )}
                </Show>
            </div>
            <div class="files-cell files-col-modified" role="gridcell">
                {formatModified(props.entry.mtime)}
            </div>
            <div class="files-cell files-col-size" role="gridcell">
                {props.entry.is_dir || props.entry.size == null ? "" : formatBytes(props.entry.size)}
            </div>
            <div class="files-cell files-col-kind" role="gridcell">
                {kindOf(props.entry)}
            </div>
        </div>
    );
}

/** One tile of the grid view: a thumbnail for an image, else a big icon,
 *  and the name below it. Same handlers as a details row. */
function GridTile(props: {
    model: FilesModel;
    entry: FsEntry;
    index: number;
    left: number;
    top: number;
    selected: boolean;
    focused: boolean;
    renaming: boolean;
    git?: FsGitState;
    touch?: Touch;
    dropTarget: boolean;
    onClick: (e: MouseEvent) => void;
    onOpen: () => void;
    onContextMenu: (e: MouseEvent) => void;
    onDragStart: (e: DragEvent) => void;
    onDragOver: () => void;
    onDragEnd: () => void;
    onRenameDone: () => void;
}): JSX.Element {
    const path = () => props.model.pathOf(props.entry.name);
    // Thumbnails read through this computer's media stream: none for a host's
    // files, which would show whatever is at that path here.
    const local = () => !props.model.connection() && !props.model.root();
    const [thumb, setThumb] = createSignal<string | undefined>(local() ? cachedThumbnail(path(), props.entry.mtime) : undefined);
    createEffect(
        on(
            () => [path(), props.entry.mtime, props.entry.size] as const,
            ([p, mtime, size]) => {
                if (!local() || !hasThumbnail(props.entry.name, size)) {
                    setThumb(undefined);
                    return;
                }
                const hit = cachedThumbnail(p, mtime);
                setThumb(hit);
                if (hit) return;
                // Scrolled past before it was made: stop asking for it.
                const gone = new AbortController();
                onCleanup(() => gone.abort());
                void thumbnail(p, mtime, gone.signal).then((url) => {
                    if (!gone.signal.aborted && url) setThumb(url);
                });
            }
        )
    );
    return (
        <div
            id={`files-${props.model.blockId}-row-${props.index}`}
            class="files-tile"
            classList={{
                "files-row-selected": props.selected,
                "files-row-focused": props.focused,
                "files-row-hidden": props.entry.hidden,
                "files-row-ignored": props.git === "ignored",
                "files-row-droptarget": props.dropTarget,
            }}
            role="gridcell"
            aria-selected={props.selected}
            style={{ left: `${props.left}px`, top: `${props.top}px` }}
            onClick={props.onClick}
            onDblClick={props.onOpen}
            onContextMenu={props.onContextMenu}
            draggable={!props.renaming}
            onDragStart={props.onDragStart}
            onDragOver={props.onDragOver}
            onDragEnd={props.onDragEnd}
            {...tip(props.entry.name)}
        >
            <div class="files-tile-art">
                <Show
                    when={thumb()}
                    fallback={<TypeIcon model={props.model} entry={props.entry} cssPx={48} glyphClass="files-tile-icon" />}
                >
                    {(src) => <img class="files-tile-thumb" src={src()} alt="" draggable={false} />}
                </Show>
                <Show when={props.touch}>
                    {(t) => (
                        <span
                            class="files-touch files-tile-touch"
                            style={t().color ? { "background-color": t().color } : undefined}
                            {...tip(`Changed by ${t().agentName}, ${formatModified(t().at)} (${t().tool})`, { label: true })}
                        />
                    )}
                </Show>
                <Show when={props.git && props.git !== "ignored" && GIT_MARKS[props.git]}>
                    {(mark) => <span class={`files-git files-tile-git files-git-${props.git}`} {...tip(mark().title)}>{mark().letter}</span>}
                </Show>
            </div>
            <Show when={props.renaming} fallback={<div class="files-tile-name">{props.entry.name}</div>}>
                <RenameInput model={props.model} entry={props.entry} onDone={props.onRenameDone} />
            </Show>
        </div>
    );
}

function RenameInput(props: { model: FilesModel; entry: FsEntry; onDone: () => void }): JSX.Element {
    let input: HTMLInputElement | undefined;
    const [problem, setProblem] = createSignal<string | null>(null);
    let busy = false;
    // Enter or Escape unmounts the input, and its removal can fire blur,
    // which must not commit a second time (ReAgent on #4201).
    let finished = false;
    // Scrolled out of the window, the row (and this box) unmounts: leave
    // rename mode with it, or the list's keyboard would wait on a box that
    // isn't there.
    onCleanup(() => {
        if (!finished && props.model.renaming() === props.entry.name) props.model.setRenaming(null);
    });
    onMount(() => {
        input?.focus();
        input?.setSelectionRange(0, stemLength(props.entry.name, props.entry.is_dir));
    });
    /** Enter commits and stays in the box on a problem, so it can be fixed.
     *  Leaving the box (blur) commits too, but on a problem it cancels: with
     *  the focus gone, the list's keys would otherwise wait on a box nobody
     *  is typing in (ReAgent on #4201). */
    const finish = async (commit: boolean, leaving = false): Promise<void> => {
        if (busy || finished) return;
        const value = input?.value ?? props.entry.name;
        if (commit && value !== props.entry.name) {
            const why = nameProblem(value, windowsNames());
            if (why && !leaving) {
                setProblem(why);
                return;
            }
            if (!why) {
                busy = true;
                const ok = await props.model.rename(props.entry.name, value);
                busy = false;
                if (!ok && !leaving) return;
            }
        }
        finished = true;
        props.model.setRenaming(null);
        props.onDone();
    };
    return (
        <span class="files-rename">
            <input
                ref={input}
                class="files-rename-input" data-raw-text-field="the inline rename in a file row: it sits in the row's own line and commits or cancels on Enter, Escape or blur, not a form field"
                value={props.entry.name}
                spellcheck={false}
                aria-label={`New name for ${props.entry.name}`}
                aria-invalid={problem() != null}
                onClick={(e) => e.stopPropagation()}
                onDblClick={(e) => e.stopPropagation()}
                onInput={() => setProblem(null)}
                onKeyDown={(e) => {
                    e.stopPropagation();
                    if (e.key === "Enter") void finish(true);
                    else if (e.key === "Escape") void finish(false);
                }}
                onBlur={() => {
                    // The window losing focus (Alt+Tab) blurs the box but
                    // leaves it the active element: the user hasn't left the
                    // rename, so keep it open for when they come back.
                    if (document.activeElement === input) return;
                    void finish(true, true);
                }}
            />
            <Show when={problem()}>
                <span class="files-rename-problem" role="alert">
                    {problem()}
                </span>
            </Show>
        </span>
    );
}
