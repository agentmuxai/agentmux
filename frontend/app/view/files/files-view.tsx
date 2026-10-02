// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Files pane ("Hangar"): a toolbar with history and a breadcrumb, Places
 * on the left, a sortable details list, and a status line.
 * docs/specs/SPEC_FILE_BROWSER_PANE_2026_10_01.md §5.
 *
 * The list is windowed by hand (fixed-height rows, §6.5): only the rows in
 * view, plus a few either side, are in the DOM, so a 100,000-entry folder
 * scrolls like a small one. Keyboard follows the ARIA grid pattern with focus
 * separate from selection (§5.3).
 */

import type { PaneTabHostContext } from "@/app/block/pane-tab-registry";
import { ConfirmDialog } from "@/app/components/confirm-dialog";
import { ContextMenu, type ContextMenuItem } from "@/app/components/context-menu";
import { formatBytes } from "@/app/element/local-media";
import { holdPaneContent, trackPaneContent } from "@/app/store/pane-content-holds";
import type { FsEntry } from "@/types/rpc/FsEntry";
import type { FsGitState } from "@/types/rpc/FsGitState";
import type { FsGitStatus } from "@/types/rpc/FsGitStatus";
import { childKey, touched, touchesUnder, type Touch } from "@/app/store/touched-files";
import { isMacOS } from "@/util/platformutil";
import { createEffect, createMemo, createSignal, For, Match, on, onCleanup, onMount, Show, Switch, type JSX } from "solid-js";
import { errorText, type FilesModel, windowsNames } from "./files-model";
import { openInPane, openTargetOf, openTerminalHere, openWithOs, revealInOs } from "./files-open";
import { FilesPreview } from "./files-preview";
import { paneWorkdir } from "@/app/drag/file-drop-actions";
import { spliceComposerTokens } from "../agent/hooks/useAgentDropAttach";
import { cachedThumbnail, hasThumbnail, thumbnail } from "./files-thumbs";
import { clipboard, opProgressText } from "./files-ops";
import {
    beginPathDrag,
    dropPathsOnto,
    endPathDrag,
    pathDragPaths,
    pathDragSource,
    registerFileDropTarget,
    visibleDropTargets,
} from "@/app/drag/file-drop";
import { getObjectValue, makeORef } from "@/app/store/mos";
import { Portal } from "solid-js/web";
import { crumbsOf, isWithin, joinPath, nameProblem, samePath, stemLength } from "./files-path";
import { clickRow, moveFocus, selectAll, toggleFocused } from "./files-selection";
import { extensionOf, type SortKey } from "./files-sort";
import { TypeAhead } from "./typeahead";
import "./files.scss";

export const ROW_HEIGHT = 24;
/** A grid tile's box (thumbnail and a two-line name). */
export const TILE_W = 112;
export const TILE_H = 132;
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

export function kindOf(e: FsEntry): string {
    if (e.is_dir) return e.is_symlink ? "Folder link" : "Folder";
    const ext = extensionOf(e.name);
    const base = ext ? `${ext.toUpperCase()} file` : "File";
    return e.is_symlink ? `${base} link` : base;
}

function iconOf(e: FsEntry): string {
    if (e.is_dir) return "folder";
    const n = e.name.toLowerCase();
    if (/\.(ts|tsx|js|jsx|mjs|cjs|py|rs|go|java|kt|swift|c|cpp|h|hpp|cs|rb|php|lua|sh|bash|zsh|ps1)$/.test(n)) return "file-code";
    if (/\.(html|htm|css|scss|sass|less|svg|xml|json|toml|yaml|yml|ini|env|conf)$/.test(n)) return "file-code";
    if (/\.(md|markdown|txt|rst|adoc|log)$/.test(n)) return "file-lines";
    if (/\.(png|jpe?g|gif|webp|bmp|ico|heic|tiff?)$/.test(n)) return "file-image";
    if (/\.(mp4|webm|mov|mkv|avi)$/.test(n)) return "file-video";
    if (/\.(wav|mp3|flac|ogg|m4a)$/.test(n)) return "file-audio";
    if (/\.pdf$/.test(n)) return "file-pdf";
    if (/\.(zip|tar|gz|7z|rar|bz2|xz)$/.test(n)) return "file-zipper";
    return "file";
}

/** The agent pane the user last worked in, for Alt+K: Hangar has the focus
 *  when the key is pressed, so it is remembered as focus moves. */
let lastAgentBlock: string | null = null;
let focusTracking = false;
function trackAgentFocus(): void {
    if (focusTracking || typeof window === "undefined") return;
    focusTracking = true;
    window.addEventListener(
        "focusin",
        (e) => {
            const pane = e.target instanceof Element ? e.target.closest('[data-role="pane"][data-blockid]') : null;
            const id = pane?.getAttribute("data-blockid");
            if (id && getObjectValue<Block>(makeORef("block", id))?.meta?.view === "agent") lastAgentBlock = id;
        },
        true
    );
}

/** `@path` for an agent's composer: relative to its working folder when
 *  inside it, quoted when it has a space. */
export function mentionToken(path: string, workdir: string | undefined): string {
    let p = path;
    if (workdir && isWithin(path, workdir) && !samePath(path, workdir)) {
        p = path.slice(workdir.replace(/[\\/]+$/, "").length + 1);
    }
    return /\s/.test(p) ? `@"${p}"` : `@${p}`;
}

type Confirm = { title: string; message: string; confirmLabel: string; onConfirm: () => void };

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
    // another pane's row is copied; a row from another Hangar pane on the
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
        const fail = (err: unknown) => model.setStatus({ text: `Couldn't open ${entry.name}: ${errorText(err)}`, tone: "error" });
        if (target === "editor" || target === "media") void openInPane(target, path, model.blockId).catch(fail);
        else if (target === "os") void openWithOs(path).catch(fail);
        else
            setConfirm({
                title: `Run ${entry.name}?`,
                message: `${entry.name} is a program or shortcut. Opening it runs it.`,
                confirmLabel: "Run",
                onConfirm: () => void openWithOs(path).catch(fail),
            });
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
        let handled = true;
        if (e.altKey && e.key === "ArrowLeft") model.goBack();
        else if (e.altKey && e.key === "ArrowRight") model.goForward();
        else if (e.altKey && e.key === "ArrowUp") model.goUp();
        else if (e.key === "ArrowDown") move({ delta: step });
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
        else if (e.key === "F2") {
            if (sel.focus && sel.names.size === 1 && sel.names.has(sel.focus)) model.setRenaming(sel.focus);
        } else if (e.key === "F5") model.refresh();
        else if (e.key === "Delete" && e.shiftKey) askDeletePermanently(model.selectedEntries());
        else if (e.key === "Delete") void model.trash(model.selectedEntries());
        else if (isMod(e) && e.key.toLowerCase() === "a") model.setSelection(selectAll(sel, order()));
        else if (isMod(e) && e.key.toLowerCase() === "z") void model.undo();
        else if (isMod(e) && e.key.toLowerCase() === "c" && !e.shiftKey) {
            // Both: the files for a paste in Hangar, the paths for anywhere else.
            model.copyToClipboard("copy", model.selectedEntries());
            copyPaths(model.selectedEntries(), true);
        } else if (isMod(e) && e.key.toLowerCase() === "x") model.copyToClipboard("cut", model.selectedEntries());
        else if (isMod(e) && e.key.toLowerCase() === "v") void model.paste();
        else if (isMod(e) && e.key.toLowerCase() === "l") startEditingPath();
        // By key code: on macOS Option+K types a character instead.
        else if (e.altKey && !e.ctrlKey && !e.metaKey && e.code === "KeyK") mentionIn(model.selectedEntries());
        else if (isMod(e) && e.shiftKey && e.key.toLowerCase() === "n") void model.createNew("dir");
        else if (isMod(e) && e.key.toLowerCase() === "f") openFilter();
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
        } else handled = false;
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

    const rowMenu = (entry: FsEntry): ContextMenuItem[] => {
        const list = model.selectedEntries();
        const many = list.length > 1;
        const path = model.pathOf(entry.name);
        const fail = (err: unknown) => model.setStatus({ text: errorText(err), tone: "error" });
        const items: ContextMenuItem[] = [{ type: "action", label: entry.is_dir ? "Open" : "Open", shortcut: "Enter", onSelect: () => openEntry(entry) }];
        if (!entry.is_dir) {
            items.push(
                { type: "action", label: "Open in Editor", onSelect: () => void openInPane("editor", path, model.blockId).catch(fail) },
                { type: "action", label: "Open with default app", onSelect: () => void openWithOs(path).catch(fail) }
            );
        } else {
            items.push({ type: "action", label: "Open terminal here", onSelect: () => void openTerminalHere(path, model.blockId).catch(fail) });
        }
        items.push(
            { type: "action", label: isMacOS() ? "Reveal in Finder" : "Reveal in file manager", onSelect: () => void revealInOs(path).catch(fail) },
            // One item per agent pane on screen (the DOM menu has no submenus).
            {
                type: "action",
                label: "Mention in agent",
                shortcut: isMacOS() ? "⌥K" : "Alt+K",
                disabled: agentTargets().length === 0,
                onSelect: () => mentionIn(many ? list : [entry]),
            },
            ...agentTargets().map((t): ContextMenuItem => ({
                type: "action",
                label: `Attach to ${t.name}`,
                onSelect: () => attachTo(t, many ? list : [entry]),
            })),
            { type: "separator" },
            { type: "action", label: "Cut", shortcut: isMacOS() ? "⌘X" : "Ctrl+X", onSelect: () => model.copyToClipboard("cut", many ? list : [entry]) },
            { type: "action", label: "Copy", shortcut: isMacOS() ? "⌘C" : "Ctrl+C", onSelect: () => model.copyToClipboard("copy", many ? list : [entry]) },
            { type: "action", label: many ? `Copy ${list.length} paths` : "Copy path", onSelect: () => copyPaths(many ? list : [entry]) },
            { type: "separator" },
            { type: "action", label: "Rename", shortcut: "F2", disabled: many, onSelect: () => model.setRenaming(entry.name) },
            { type: "action", label: many ? `Move ${list.length} items to Trash` : "Move to Trash", shortcut: "Delete", onSelect: () => void model.trash(many ? list : [entry]) },
            {
                type: "action",
                label: "Delete permanently…",
                shortcut: "Shift+Delete",
                danger: true,
                onSelect: () => askDeletePermanently(many ? list : [entry]),
            }
        );
        return items;
    };

    /** The agent panes on screen, by name: "Attach to <agent>" targets. */
    const agentTargets = (): { blockId: string; name: string }[] =>
        visibleDropTargets()
            .map((blockId) => ({ blockId, meta: getObjectValue<Block>(makeORef("block", blockId))?.meta }))
            .filter((t) => t.meta?.view === "agent")
            .map((t) => ({ blockId: t.blockId, name: (t.meta?.["agentName"] as string | undefined)?.trim() || "agent" }));

    /**
     * Alt+K (spec §8.2, route 3): an `@path` mention of each selected entry
     * in the composer of the agent the user was last working with, as
     * Claude Code's own editor integrations do. Relative to the agent's
     * working folder when inside it.
     */
    const mentionIn = (list: FsEntry[]): void => {
        if (list.length === 0) return;
        const agents = agentTargets();
        const target = agents.find((t) => t.blockId === lastAgentBlock) ?? (agents.length === 1 ? agents[0] : undefined);
        if (!target) {
            model.setStatus(
                {
                    text: agents.length === 0 ? "No agent pane is open to mention these in." : "Click into the agent you mean first, then Alt+K here.",
                    tone: "info",
                },
                4000
            );
            return;
        }
        const tokens = list.map((e) => mentionToken(model.pathOf(e.name), paneWorkdir(target.blockId)));
        const root = document.querySelector<HTMLElement>(`[data-role="pane"][data-blockid="${CSS.escape(target.blockId)}"]`);
        if (!root || !spliceComposerTokens(root, tokens)) {
            model.setStatus({ text: `${target.name}'s message box isn't open.`, tone: "error" }, 4000);
        }
    };

    const attachTo = (target: { blockId: string; name: string }, list: FsEntry[]): void => {
        const paths = list.map((e) => model.pathOf(e.name));
        void dropPathsOnto(target.blockId, paths).then(
            (ok) =>
                model.setStatus(
                    ok
                        ? { text: `Attached ${list.length === 1 ? list[0].name : `${list.length} items`} to ${target.name}`, tone: "info" }
                        : { text: `${target.name} can't take files right now`, tone: "error" },
                    4000
                ),
            (err) => model.setStatus({ text: `Couldn't attach: ${errorText(err)}`, tone: "error" })
        );
    };

    const folderMenu = (): ContextMenuItem[] => {
        const fail = (err: unknown) => model.setStatus({ text: errorText(err), tone: "error" });
        return [
            { type: "action", label: "New folder", shortcut: isMacOS() ? "⌘⇧N" : "Ctrl+Shift+N", onSelect: () => void model.createNew("dir") },
            { type: "action", label: "New file", onSelect: () => void model.createNew("file") },
            {
                type: "action",
                label: clipboard()?.kind === "cut" ? `Paste (move ${clipboard()!.paths.length})` : clipboard() ? `Paste (copy ${clipboard()!.paths.length})` : "Paste",
                shortcut: isMacOS() ? "⌘V" : "Ctrl+V",
                disabled: !clipboard(),
                onSelect: () => void model.paste(),
            },
            { type: "separator" },
            { type: "action", label: "Open terminal here", onSelect: () => void openTerminalHere(model.path(), model.blockId).catch(fail) },
            { type: "action", label: isMacOS() ? "Reveal in Finder" : "Reveal in file manager", onSelect: () => void revealInOs(model.path()).catch(fail) },
            { type: "action", label: "Copy folder path", onSelect: () => void navigator.clipboard?.writeText(model.path()) },
            { type: "separator" },
            { type: "action", label: model.showHidden() ? "Hide hidden files" : "Show hidden files", onSelect: () => model.toggleHidden() },
            { type: "action", label: "Refresh", shortcut: "F5", onSelect: () => model.refresh() },
            { type: "action", label: "Undo", shortcut: isMacOS() ? "⌘Z" : "Ctrl+Z", onSelect: () => void model.undo() },
        ];
    };

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

    const placeLabel = (path: string): string => model.protectedPlace(path)?.label ?? path;

    return (
        <div class="files-view" onContextMenu={(e) => e.preventDefault()}>
            <div class="files-toolbar">
                <button type="button" class="files-tool" title="Back (Alt+←)" disabled={!model.canBack()} onClick={() => model.goBack()}>
                    <i class="fa fa-arrow-left" />
                </button>
                <button type="button" class="files-tool" title="Forward (Alt+→)" disabled={!model.canForward()} onClick={() => model.goForward()}>
                    <i class="fa fa-arrow-right" />
                </button>
                <button type="button" class="files-tool" title="Up (Alt+↑)" disabled={crumbsOf(model.path()).length < 2} onClick={() => model.goUp()}>
                    <i class="fa fa-arrow-up" />
                </button>
                <Show
                    when={!editingPath()}
                    fallback={
                        <input
                            ref={pathInput}
                            class="files-path-input"
                            value={model.path()}
                            spellcheck={false}
                            aria-label="Folder path"
                            onKeyDown={(e) => {
                                e.stopPropagation();
                                if (e.key === "Enter") commitPath(e.currentTarget.value);
                                else if (e.key === "Escape") {
                                    setEditingPath(false);
                                    listEl?.focus();
                                }
                            }}
                            onBlur={() => setEditingPath(false)}
                        />
                    }
                >
                    <nav ref={crumbsEl} class="files-breadcrumb" aria-label="Folder path" onDblClick={startEditingPath} title="Double-click or Ctrl+L to type a path">
                        <For each={crumbsOf(model.path())}>
                            {(crumb, i) => (
                                <>
                                    <Show when={i() > 0}>
                                        <i class="fa fa-chevron-right files-crumb-sep" aria-hidden="true" />
                                    </Show>
                                    <button type="button" class="files-crumb" onClick={() => void model.navigate(crumb.path)}>
                                        {crumb.label}
                                    </button>
                                </>
                            )}
                        </For>
                    </nav>
                </Show>
                <Show when={filterOpen() || model.filter() !== ""}>
                    <input
                        ref={filterInput}
                        class="files-filter-input"
                        placeholder="Filter"
                        aria-label="Filter this folder"
                        value={model.filter()}
                        spellcheck={false}
                        onInput={(e) => model.setFilter(e.currentTarget.value)}
                        onKeyDown={(e) => {
                            e.stopPropagation();
                            if (e.key === "Escape") closeFilter();
                            else if (e.key === "Enter" || e.key === "ArrowDown") {
                                e.preventDefault();
                                const first = model.order()[0];
                                if (first) model.setSelection({ names: new Set([first]), focus: first, anchor: first });
                                listEl?.focus();
                            }
                        }}
                    />
                </Show>
                <button type="button" class="files-tool" title={isMacOS() ? "Filter (⌘F)" : "Filter (Ctrl+F)"} onClick={openFilter}>
                    <i class="fa fa-filter" />
                </button>
                <button type="button" class="files-tool" title="New folder" onClick={() => void model.createNew("dir")}>
                    <i class="fa fa-folder-plus" />
                </button>
                <button
                    type="button"
                    class="files-tool"
                    classList={{ "files-tool-on": model.showHidden() }}
                    title={model.showHidden() ? "Hide hidden files" : "Show hidden files"}
                    aria-pressed={model.showHidden()}
                    onClick={() => model.toggleHidden()}
                >
                    <i class={`fa ${model.showHidden() ? "fa-eye" : "fa-eye-slash"}`} />
                </button>
                <button type="button" class="files-tool" title="Refresh (F5)" onClick={() => model.refresh()}>
                    <i class="fa fa-arrows-rotate" />
                </button>
                <button
                    type="button"
                    class="files-tool"
                    classList={{ "files-tool-on": model.showSidebar() }}
                    title={model.showSidebar() ? "Hide Places" : "Show Places"}
                    aria-pressed={model.showSidebar()}
                    onClick={() => model.toggleSidebar()}
                >
                    <i class="fa fa-table-columns" />
                </button>
                <button
                    type="button"
                    class="files-tool"
                    title={grid() ? "Show as a list" : "Show as thumbnails"}
                    aria-pressed={grid()}
                    onClick={() => {
                        model.toggleViewMode();
                        if (listEl) listEl.scrollTop = 0;
                        setScrollTop(0);
                    }}
                >
                    <i class={`fa ${grid() ? "fa-list" : "fa-table-cells-large"}`} />
                </button>
                <button
                    type="button"
                    class="files-tool"
                    classList={{ "files-tool-on": model.showPreview() }}
                    title={model.showPreview() ? "Hide preview (Space)" : "Show preview (Space)"}
                    aria-pressed={model.showPreview()}
                    onClick={() => model.togglePreview()}
                >
                    <i class="fa fa-eye" />
                </button>
            </div>

            <div class="files-body">
                <Show when={model.showSidebar()}>
                    <nav class="files-places" aria-label="Places">
                        <div class="files-places-heading">Places</div>
                        <For each={model.places().filter((p) => p.kind !== "drive")}>
                            {(p) => (
                                <button type="button" class="files-place" title={p.path} onClick={() => void model.navigate(p.path)}>
                                    <i class={`fa fa-${p.kind === "home" ? "house" : "folder"}`} />
                                    <span>{p.label}</span>
                                </button>
                            )}
                        </For>
                        <Show when={model.places().some((p) => p.kind === "drive")}>
                            <div class="files-places-heading">Drives</div>
                            <For each={model.places().filter((p) => p.kind === "drive")}>
                                {(p) => (
                                    <button type="button" class="files-place" title={p.path} onClick={() => void model.navigate(p.path)}>
                                        <i class="fa fa-hard-drive" />
                                        <span>{p.label}</span>
                                    </button>
                                )}
                            </For>
                        </Show>
                        <Show when={model.agents().length > 0}>
                            <div class="files-places-heading">Agents</div>
                            <For each={model.agents()}>
                                {(a) => (
                                    <button type="button" class="files-place" title={a.path} onClick={() => void model.navigate(a.path)}>
                                        <i class="fa fa-robot" />
                                        <span>{a.name}</span>
                                    </button>
                                )}
                            </For>
                        </Show>
                    </nav>
                </Show>

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
                    onClick={(e) => {
                        if (e.target === e.currentTarget) model.setSelection({ ...model.selection(), names: new Set() });
                    }}
                >
                    <div class="files-row files-header" role="row" aria-rowindex={1}>
                        <For each={COLUMNS}>
                            {(col) => (
                                <button
                                    type="button"
                                    role="columnheader"
                                    class={`files-cell ${col.class}`}
                                    aria-sort={model.sortKey() === col.key ? (model.sortDir() === "asc" ? "ascending" : "descending") : "none"}
                                    onClick={() => model.setSort(col.key)}
                                >
                                    {col.label}
                                    <Show when={model.sortKey() === col.key}>
                                        <i class={`fa fa-caret-${model.sortDir() === "asc" ? "up" : "down"} files-sort-mark`} aria-hidden="true" />
                                    </Show>
                                </button>
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
                                <button type="button" class="files-button" onClick={() => void model.navigate(model.path(), { push: false, consented: true })}>
                                    Open {placeLabel(model.path())}
                                </button>
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
                                <button type="button" class="files-button" onClick={() => model.refresh()}>
                                    Try again
                                </button>
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
                                {(name, i) => (
                                    <Show when={byName().get(name)}>
                                        {(entry) => {
                                            const index = () => range().first + i();
                                            return (
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
                                            );
                                        }}
                                    </Show>
                                )}
                            </For>
                        </div>
                    </Show>
                    <Show when={model.phase() === "ready" && !grid()}>
                        <div class="files-rows" style={{ height: `${entries().length * ROW_HEIGHT}px` }}>
                            <For each={visibleNames()}>
                                {(name, i) => (
                                    <Show when={byName().get(name)}>
                                        {(entry) => (
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
                                        )}
                                    </Show>
                                )}
                            </For>
                        </div>
                    </Show>
                </div>
                <Show when={model.showPreview()}>
                    <FilesPreview
                        entry={previewEntry()}
                        pathOf={(name) => model.pathOf(name)}
                        onOpen={openEntry}
                        onOpenWithOs={(entry) =>
                            void openWithOs(model.pathOf(entry.name)).catch((err) =>
                                model.setStatus({ text: `Couldn't open ${entry.name}: ${errorText(err)}`, tone: "error" })
                            )
                        }
                    />
                </Show>
            </div>

            <For each={model.ops.ops().filter((o) => o.state === "running" || o.state === "conflict")}>
                {(op) => (
                    <div class="files-op" role="status">
                        <span class="files-op-text">{opProgressText(op)}</span>
                        <span class="files-op-bar" aria-hidden="true">
                            <span
                                class="files-op-fill"
                                style={{ width: `${op.total_bytes > 0 ? Math.floor((op.done_bytes / op.total_bytes) * 100) : op.total_items > 0 ? Math.floor((op.done_items / op.total_items) * 100) : 0}%` }}
                            />
                        </span>
                        <button type="button" class="files-status-undo" onClick={() => void model.ops.cancel(op.op_id)}>
                            Cancel
                        </button>
                    </div>
                )}
            </For>
            <Show when={model.ops.ops().find((o) => o.state === "conflict" && o.conflict)}>
                {(op) => <ConflictDialog op={op()} onResolve={(choice, all) => void model.ops.resolve(op().op_id, choice, all)} onCancel={() => void model.ops.cancel(op().op_id)} />}
            </Show>
            <div class="files-status" role="status">
                <Show
                    when={model.status()}
                    fallback={
                        <span>
                            {summary()}
                            <Show when={model.git()}>
                                {(g) => (
                                    <span class="files-git-summary" title="git branch, commits ahead/behind its upstream, and changes under this folder">
                                        <i class="fa fa-code-branch" aria-hidden="true" /> {gitSummary(g())}
                                    </span>
                                )}
                            </Show>
                        </span>
                    }
                >
                    {(msg) => (
                        <span classList={{ "files-status-error": msg().tone === "error" }}>
                            {msg().text}
                            <Show when={msg().undo}>
                                <button type="button" class="files-status-undo" onClick={() => void model.undo()}>
                                    Undo
                                </button>
                            </Show>
                        </span>
                    )}
                </Show>
            </div>

            <Show when={menu()}>
                {(m) => <ContextMenu items={m().items} x={m().x} y={m().y} onClose={() => setMenu(null)} />}
            </Show>
            <Show when={confirm()}>
                {(c) => (
                    <ConfirmDialog
                        title={c().title}
                        message={c().message}
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
export const GIT_MARKS: Record<Exclude<FsGitState, "ignored">, { letter: string; title: string }> = {
    untracked: { letter: "U", title: "Untracked: new, not yet added to git" },
    added: { letter: "A", title: "Added to git, not yet committed" },
    renamed: { letter: "R", title: "Renamed" },
    deleted: { letter: "D", title: "Deleted" },
    modified: { letter: "M", title: "Changed since the last commit" },
    conflicted: { letter: "!", title: "Merge conflict" },
};

/** "main ↑1 ↓2 · 3 changes" for the status line. */
export function gitSummary(g: FsGitStatus): string {
    const parts: string[] = [];
    let head = g.branch ?? "detached";
    if (g.ahead) head += ` ↑${g.ahead}`;
    if (g.behind) head += ` ↓${g.behind}`;
    parts.push(head);
    if (g.changes > 0) parts.push(`${g.changes} change${g.changes === 1 ? "" : "s"}`);
    return parts.join(" · ");
}

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
            title={props.entry.error ?? (props.entry.link_target ? `→ ${props.entry.link_target}` : undefined)}
        >
            <div class="files-cell files-col-name" role="gridcell">
                <i class={`fa fa-${iconOf(props.entry)} files-icon`} classList={{ "files-icon-dir": props.entry.is_dir }} aria-hidden="true" />
                <Show when={props.entry.is_symlink}>
                    <i class="fa fa-share files-link-mark" aria-label="link" />
                </Show>
                <Show when={props.touch}>
                    {(t) => (
                        <span
                            class="files-touch"
                            style={t().color ? { "background-color": t().color } : undefined}
                            title={`${props.entry.is_dir ? "Something inside was changed" : "Changed"} by ${t().agentName}, ${formatModified(t().at)} (${t().tool})`}
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
                        <span class={`files-git files-git-${props.git}`} title={mark().title} aria-label={mark().title}>
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
    const [thumb, setThumb] = createSignal<string | undefined>(cachedThumbnail(path(), props.entry.mtime));
    createEffect(
        on(
            () => [path(), props.entry.mtime, props.entry.size] as const,
            ([p, mtime, size]) => {
                if (!hasThumbnail(props.entry.name, size)) {
                    setThumb(undefined);
                    return;
                }
                const hit = cachedThumbnail(p, mtime);
                setThumb(hit);
                if (hit) return;
                let live = true;
                onCleanup(() => (live = false));
                void thumbnail(p, mtime).then((url) => {
                    if (live && url) setThumb(url);
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
            title={props.entry.name}
        >
            <div class="files-tile-art">
                <Show
                    when={thumb()}
                    fallback={<i class={`fa fa-${iconOf(props.entry)} files-tile-icon`} classList={{ "files-icon-dir": props.entry.is_dir }} aria-hidden="true" />}
                >
                    {(src) => <img class="files-tile-thumb" src={src()} alt="" draggable={false} />}
                </Show>
                <Show when={props.touch}>
                    {(t) => (
                        <span
                            class="files-touch files-tile-touch"
                            style={t().color ? { "background-color": t().color } : undefined}
                            title={`Changed by ${t().agentName}, ${formatModified(t().at)} (${t().tool})`}
                        />
                    )}
                </Show>
                <Show when={props.git && props.git !== "ignored" && GIT_MARKS[props.git]}>
                    {(mark) => <span class={`files-git files-tile-git files-git-${props.git}`} title={mark().title}>{mark().letter}</span>}
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
                class="files-rename-input"
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

/** The full path of an entry in the folder shown (for tests and callers). */
export const entryPath = (dir: string, name: string): string => joinPath(dir, name);

/** A copy or move found something already at the destination (§7.2). */
function ConflictDialog(props: {
    op: import("./files-ops").OpView;
    onResolve: (choice: "replace" | "skip" | "keep_both", applyToAll: boolean) => void;
    onCancel: () => void;
}): JSX.Element {
    const [all, setAll] = createSignal(false);
    const c = () => props.op.conflict!;
    const name = () => c().dest.split(/[\\/]/).pop() ?? c().dest;
    const side = (size?: number, mtime?: number) =>
        [size != null ? formatBytes(size) : null, mtime != null ? new Date(mtime).toLocaleString() : null].filter(Boolean).join(", ");
    let first: HTMLButtonElement | undefined;
    onMount(() => first?.focus());
    return (
        <Portal>
            <div
                class="files-conflict-overlay"
                onKeyDown={(e) => {
                    if (e.key === "Escape") {
                        e.preventDefault();
                        props.onCancel();
                    }
                }}
            >
                <div class="files-conflict" role="alertdialog" aria-labelledby="files-conflict-title">
                    <div id="files-conflict-title" class="files-conflict-title">
                        “{name()}” already exists here
                    </div>
                    <div class="files-conflict-body">
                        <div>
                            {c().dest_is_dir ? "A folder" : "A file"} with that name is already in the destination
                            {c().source_is_dir && c().dest_is_dir ? "." : `${c().dest_is_dir !== c().source_is_dir ? `, and the one being ${props.op.kind === "move" ? "moved" : "copied"} is a ${c().source_is_dir ? "folder" : "file"}.` : "."}`}
                        </div>
                        <Show when={!c().source_is_dir && !c().dest_is_dir}>
                            <div class="files-conflict-sides">
                                <div>Existing: {side(c().dest_size ?? undefined, c().dest_mtime ?? undefined)}</div>
                                <div>Incoming: {side(c().source_size ?? undefined, c().source_mtime ?? undefined)}</div>
                            </div>
                        </Show>
                        <label class="files-conflict-all">
                            <input type="checkbox" checked={all()} onChange={(e) => setAll(e.currentTarget.checked)} /> Do this for every conflict
                        </label>
                    </div>
                    <div class="files-conflict-actions">
                        <button ref={first} type="button" class="files-button" onClick={() => props.onResolve("keep_both", all())}>
                            Keep both
                        </button>
                        <button type="button" class="files-button" onClick={() => props.onResolve("skip", all())}>
                            Skip
                        </button>
                        <button type="button" class="files-button files-button-danger" onClick={() => props.onResolve("replace", all())}>
                            Replace
                        </button>
                        <button type="button" class="files-button" onClick={props.onCancel}>
                            Stop
                        </button>
                    </div>
                </div>
            </div>
        </Portal>
    );
}
