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
import { isMacOS } from "@/util/platformutil";
import { createEffect, createMemo, createSignal, For, Match, on, onCleanup, onMount, Show, Switch, type JSX } from "solid-js";
import { errorText, type FilesModel, windowsNames } from "./files-model";
import { openInPane, openTargetOf, openTerminalHere, openWithOs, revealInOs } from "./files-open";
import { crumbsOf, joinPath, nameProblem, stemLength } from "./files-path";
import { clickRow, moveFocus, selectAll, toggleFocused } from "./files-selection";
import { extensionOf, type SortKey } from "./files-sort";
import { TypeAhead } from "./typeahead";
import "./files.scss";

export const ROW_HEIGHT = 24;
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
    const [editingPath, setEditingPath] = createSignal(false);
    const [menu, setMenu] = createSignal<{ x: number; y: number; items: ContextMenuItem[] } | null>(null);
    const [confirm, setConfirm] = createSignal<Confirm | null>(null);
    const typeahead = new TypeAhead();
    let listEl: HTMLDivElement | undefined;
    let pathInput: HTMLInputElement | undefined;

    model.focusList = () => listEl?.focus();
    onCleanup(() => (model.focusList = null));

    onMount(() => {
        void model.start();
        if (listEl) {
            const ro = new ResizeObserver(() => setViewHeight(listEl?.clientHeight ?? 400));
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
    const range = createMemo(
        () => {
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

    const focusIndex = createMemo(() => {
        const f = model.selection().focus;
        return f == null ? -1 : order().indexOf(f);
    });

    /** Keeps the focused row inside the scrolled view. */
    const reveal = (index: number): void => {
        if (!listEl || index < 0) return;
        const top = index * ROW_HEIGHT;
        const header = ROW_HEIGHT;
        if (top < listEl.scrollTop) listEl.scrollTop = top;
        else if (top + ROW_HEIGHT > listEl.scrollTop + listEl.clientHeight - header)
            listEl.scrollTop = top + ROW_HEIGHT - listEl.clientHeight + header;
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

    const copyPaths = (list: FsEntry[]): void => {
        const text = list.map((e) => model.pathOf(e.name)).join("\n");
        void navigator.clipboard?.writeText(text).then(
            () => model.setStatus({ text: list.length === 1 ? "Copied the path" : `Copied ${list.length} paths`, tone: "info" }, 2500),
            () => model.setStatus({ text: "Couldn't copy to the clipboard", tone: "error" })
        );
    };

    // ── Keyboard (§5.3.1) ──────────────────────────────────────────────────

    const onKeyDown = (e: KeyboardEvent): void => {
        if (model.renaming() || editingPath()) return;
        const sel = model.selection();
        const page = Math.max(1, Math.floor((listEl?.clientHeight ?? 400) / ROW_HEIGHT) - 2);
        const move = (m: { delta?: number; to?: number }) => {
            model.setSelection(moveFocus(sel, order(), m, { extend: e.shiftKey, focusOnly: isMod(e) && !e.shiftKey }));
            reveal(order().indexOf(model.selection().focus ?? ""));
        };
        let handled = true;
        if (e.altKey && e.key === "ArrowLeft") model.goBack();
        else if (e.altKey && e.key === "ArrowRight") model.goForward();
        else if (e.altKey && e.key === "ArrowUp") model.goUp();
        else if (e.key === "ArrowDown") move({ delta: 1 });
        else if (e.key === "ArrowUp") move({ delta: -1 });
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
            if (sel.focus) model.setRenaming(sel.focus);
        } else if (e.key === "F5") model.refresh();
        else if (e.key === "Delete" && e.shiftKey) askDeletePermanently(model.selectedEntries());
        else if (e.key === "Delete") void model.trash(model.selectedEntries());
        else if (isMod(e) && e.key.toLowerCase() === "a") model.setSelection(selectAll(sel, order()));
        else if (isMod(e) && e.key.toLowerCase() === "z") void model.undo();
        else if (isMod(e) && e.key.toLowerCase() === "c" && !e.shiftKey) copyPaths(model.selectedEntries());
        else if (isMod(e) && e.key.toLowerCase() === "l") startEditingPath();
        else if (isMod(e) && e.shiftKey && e.key.toLowerCase() === "n") void model.createNew("dir");
        else if (e.key === " " && !typeahead.active()) model.setSelection(toggleFocused(sel));
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
            { type: "action", label: many ? `Copy ${list.length} paths` : "Copy path", shortcut: isMacOS() ? "⌘C" : "Ctrl+C", onSelect: () => copyPaths(many ? list : [entry]) },
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

    const folderMenu = (): ContextMenuItem[] => {
        const fail = (err: unknown) => model.setStatus({ text: errorText(err), tone: "error" });
        return [
            { type: "action", label: "New folder", shortcut: isMacOS() ? "⌘⇧N" : "Ctrl+Shift+N", onSelect: () => void model.createNew("dir") },
            { type: "action", label: "New file", onSelect: () => void model.createNew("file") },
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
        const count = `${all.length} item${all.length === 1 ? "" : "s"}${model.partial() ? "…" : ""}`;
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
                    <nav class="files-breadcrumb" aria-label="Folder path" onDblClick={startEditingPath} title="Double-click or Ctrl+L to type a path">
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
                                {model.rawEntries().length > 0 ? "Only hidden files here." : "This folder is empty."}
                            </div>
                        </Match>
                    </Switch>

                    <Show when={model.phase() === "ready"}>
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
                                                onClick={(e) => onRowClick(e, entry())}
                                                onOpen={() => openEntry(entry())}
                                                onContextMenu={(e) => onRowContextMenu(e, entry())}
                                                onRenameDone={() => listEl?.focus()}
                                            />
                                        )}
                                    </Show>
                                )}
                            </For>
                        </div>
                    </Show>
                </div>
            </div>

            <div class="files-status" role="status">
                <Show when={model.status()} fallback={<span>{summary()}</span>}>
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
}): JSX.Element {
    return (
        <div
            id={`files-${props.model.blockId}-row-${props.index}`}
            class="files-row"
            classList={{
                "files-row-selected": props.selected,
                "files-row-focused": props.focused,
                "files-row-hidden": props.entry.hidden,
            }}
            role="row"
            aria-rowindex={props.index + 2}
            aria-selected={props.selected}
            style={{ top: `${props.index * ROW_HEIGHT}px` }}
            onClick={props.onClick}
            onDblClick={props.onOpen}
            onContextMenu={props.onContextMenu}
            title={props.entry.error ?? (props.entry.link_target ? `→ ${props.entry.link_target}` : undefined)}
        >
            <div class="files-cell files-col-name" role="gridcell">
                <i class={`fa fa-${iconOf(props.entry)} files-icon`} classList={{ "files-icon-dir": props.entry.is_dir }} aria-hidden="true" />
                <Show when={props.entry.is_symlink}>
                    <i class="fa fa-share files-link-mark" aria-label="link" />
                </Show>
                <Show when={props.renaming} fallback={<span class="files-name">{props.entry.name}</span>}>
                    <RenameInput model={props.model} entry={props.entry} onDone={props.onRenameDone} />
                </Show>
                <Show when={props.entry.error}>
                    <i class="fa fa-triangle-exclamation files-entry-error" aria-label={props.entry.error} />
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
    const finish = async (commit: boolean): Promise<void> => {
        if (busy || finished) return;
        const value = input?.value ?? props.entry.name;
        if (commit && value !== props.entry.name) {
            const why = nameProblem(value, windowsNames());
            if (why) {
                setProblem(why);
                return;
            }
            busy = true;
            const ok = await props.model.rename(props.entry.name, value);
            busy = false;
            if (!ok) return;
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
                onBlur={() => void finish(true)}
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
