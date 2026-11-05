// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What the Files pane draws: the toolbar, Places, the list with its column header, notices and
 * windowed rows or tiles, the preview, the status line, and the menu and confirmation on top. Split
 * out of FilesView; each part reads the hook groups the view built.
 */

import { ConfirmDialog } from "@/app/components/confirm-dialog";
import { ContextMenu } from "@/app/components/context-menu";
import { endPathDrag } from "@/app/drag/file-drop";
import { Button } from "@/app/element/ui";
import type { PaneTabHostContext } from "@/app/store/pane-tab-registry";
import { readZoom } from "@/app/store/zoom-factor";
import { For, Match, Show, Switch, type JSX } from "solid-js";
import { FilesFooter } from "./FilesFooter";
import { FilesPlaces } from "./FilesPlaces";
import { FilesToolbar } from "./FilesToolbar";
import type { FilesModel } from "./files-model";
import { openEntryWithOs } from "./files-open";
import { FilesPreview } from "./files-preview";
import { FileRow, GridTile, ROW_HEIGHT, TILE_H, TILE_W } from "./files-rows";
import { folderAt } from "./files-selection";
import type { SortKey } from "./files-sort";
import type { useFilesDrag } from "./files-view-drag";
import type { FilesKeys } from "./files-view-keys";
import type { FilesOpening } from "./files-view-opening";
import type { useFilesPointer } from "./files-view-pointer";
import type { FilesLocal } from "./files-view-state";
import type { FilesWindow } from "./files-view-window";

const COLUMNS: { key: SortKey; label: string; class: string }[] = [
    { key: "name", label: "Name", class: "files-col-name" },
    { key: "modified", label: "Modified", class: "files-col-modified" },
    { key: "size", label: "Size", class: "files-col-size" },
    { key: "kind", label: "Kind", class: "files-col-kind" },
];

/** The hook groups FilesView built, handed to the parts that draw them. */
export interface FilesViewParts {
    model: FilesModel;
    ctx: PaneTabHostContext;
    fv: FilesLocal;
    drag: ReturnType<typeof useFilesDrag>;
    win: FilesWindow;
    opening: FilesOpening;
    keys: FilesKeys;
    pointer: ReturnType<typeof useFilesPointer>;
}

export function FilesViewLayout({ v }: { v: FilesViewParts }): JSX.Element {
    const { model, ctx, fv, win, opening, keys, pointer } = v;
    const { refs } = fv;
    return (
        <div
            class="files-view"
            // CSS zoom from the shared `term:zoom`: everything scales together, and
            // the @container on this element measures in zoomed pixels.
            style={{ zoom: readZoom(ctx.meta()) }}
            // The folder shown, for tooling (verify-shortcuts.mjs's temp-tree guard).
            data-path={model.path()}
            data-connection={model.connection()}
            onContextMenu={(e) => e.preventDefault()}
            onKeyDown={keys.onPaneKeyDown}
        >
            <FilesToolbar
                model={model}
                grid={win.grid}
                editingPath={fv.editingPath}
                filterOpen={fv.filterOpen}
                startEditingPath={fv.startEditingPath}
                commitPath={fv.commitPath}
                onStopEditing={() => fv.setEditingPath(false)}
                openFilter={fv.openFilter}
                closeFilter={fv.closeFilter}
                onToggleViewMode={pointer.toggleViewMode}
                focusList={() => refs.list?.focus()}
                pathInputRef={(el) => (refs.pathInput = el)}
                crumbsRef={(el) => (refs.crumbs = el)}
                filterRef={(el) => (refs.filterInput = el)}
            />

            <div class="files-body">
                <FilesPlaces model={model} />

                <FilesList v={v} />
                <Show when={model.showPreview()}>
                    <FilesPreview
                        entry={win.previewEntry()}
                        pathOf={(name) => model.pathOf(name)}
                        blockId={model.blockId}
                        onOpen={opening.openEntry}
                        onOpenWithOs={(entry) => openEntryWithOs(model, entry.name)}
                    />
                </Show>
            </div>

            <FilesFooter model={model} summary={pointer.summary} />

            <Show when={fv.menu()}>
                {(m) => <ContextMenu items={m().items} x={m().x} y={m().y} onClose={() => fv.setMenu(null)} />}
            </Show>
            <Show when={fv.confirm()}>
                {(c) => (
                    <ConfirmDialog
                        title={c().title}
                        message={c().message}
                        confirmLabel={c().confirmLabel}
                        onConfirm={() => {
                            c().onConfirm();
                            fv.setConfirm(null);
                            refs.list?.focus();
                        }}
                        onCancel={() => {
                            fv.setConfirm(null);
                            refs.list?.focus();
                        }}
                    />
                )}
            </Show>
        </div>
    );
}

/** The list: its header, a notice when there is no listing to draw, and the windowed rows or tiles. */
function FilesList({ v }: { v: FilesViewParts }): JSX.Element {
    const { model, fv, win, opening, keys, pointer } = v;
    const { refs } = fv;
    return (
        <div
            ref={(el) => (refs.list = el)}
            class="files-list"
            role="grid"
            aria-label="Files"
            aria-multiselectable="true"
            aria-rowcount={win.entries().length + 1}
            aria-activedescendant={win.focusIndex() >= 0 ? `files-${model.blockId}-row-${win.focusIndex()}` : undefined}
            tabIndex={0}
            onKeyDown={keys.onKeyDown}
            onScroll={(e) => fv.setScrollTop(e.currentTarget.scrollTop)}
            // Over blank space, the header or a notice: no folder row is
            // the target any more (ReAgent on #4224). A row's own
            // dragover runs first and sets or clears it.
            onDragOver={(e) => {
                if (!(e.target instanceof Element && e.target.closest(".files-rows .files-row, .files-tile"))) fv.setDropRow(null);
            }}
            onDragLeave={(e) => {
                if (!(e.relatedTarget instanceof Node && e.currentTarget.contains(e.relatedTarget))) fv.setDropRow(null);
            }}
            onContextMenu={pointer.onListContextMenu}
            // Windows: a middle press over the list starts autoscroll, and no auxclick follows.
            onMouseDown={(e) => e.button === 1 && folderAt(e.target, win.byName()) && e.preventDefault()}
            onAuxClick={(e) => {
                // Middle-click a folder: open it in a new tab.
                const entry = e.button === 1 && folderAt(e.target, win.byName());
                if (!entry) return;
                e.preventDefault();
                opening.openInNewTab(model.pathOf(entry.name));
            }}
            onClick={(e) => {
                if (e.target === e.currentTarget) model.setSelection({ ...model.selection(), names: new Set() });
            }}
        >
            <FilesColumnHeader model={model} />
            <FilesNotices v={v} />
            <Show when={model.phase() === "ready" && win.grid()}>
                <FilesGridTiles v={v} />
            </Show>
            <Show when={model.phase() === "ready" && !win.grid()}>
                <FilesDetailRows v={v} />
            </Show>
        </div>
    );
}

/** The sortable column header. */
function FilesColumnHeader(props: { model: FilesModel }): JSX.Element {
    const model = props.model;
    return (
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
    );
}

/** Why there are no rows: a macOS prompt to answer, a slow load, an error, or an empty folder. */
function FilesNotices({ v }: { v: FilesViewParts }): JSX.Element {
    const { model, win, pointer } = v;
    const { placeLabel } = pointer;
    return (
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
            <Match when={model.phase() === "ready" && win.entries().length === 0 && !model.partial()}>
                <div class="files-notice">
                    {model.filter()
                        ? `Nothing here matches “${model.filter()}”.`
                        : model.rawEntries().length > 0
                          ? "Only hidden files here."
                          : "This folder is empty."}
                </div>
            </Match>
        </Switch>
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

/** The grid view: the tiles in the window, each at its place. */
function FilesGridTiles({ v }: { v: FilesViewParts }): JSX.Element {
    const { model, fv, win, drag, opening, pointer } = v;
    return (
        <div class="files-grid" style={{ height: `${Math.ceil(win.entries().length / win.gridCols()) * TILE_H}px` }}>
            <For each={win.visibleNames()}>
                {(name, i) => {
                    const entry = win.entryOf(name);
                    const index = () => win.range().first + i();
                    return (
                        <Show when={win.byName().has(name)}>
                            <GridTile
                                model={model}
                                entry={entry()}
                                index={index()}
                                left={(index() % win.gridCols()) * TILE_W}
                                top={Math.floor(index() / win.gridCols()) * TILE_H}
                                selected={model.selection().names.has(name)}
                                focused={model.selection().focus === name}
                                renaming={model.renaming() === name}
                                git={model.gitStateOf().get(name)}
                                touch={win.touchOf(name)}
                                dropTarget={fv.dropRow() === name}
                                onClick={(e) => pointer.onRowClick(e, entry())}
                                onOpen={() => opening.openEntry(entry())}
                                onContextMenu={(e) => pointer.onRowContextMenu(e, entry())}
                                onDragStart={(e) => drag.onRowDragStart(e, entry())}
                                onDragOver={() => drag.onRowDragOver(entry())}
                                onDragEnd={() => endPathDrag()}
                                onRenameDone={() => fv.refs.list?.focus()}
                            />
                        </Show>
                    );
                }}
            </For>
        </div>
    );
}

/** The details view: the rows in the window, each at its place. */
function FilesDetailRows({ v }: { v: FilesViewParts }): JSX.Element {
    const { model, fv, win, drag, opening, pointer } = v;
    return (
        <div class="files-rows" style={{ height: `${win.entries().length * ROW_HEIGHT}px` }}>
            <For each={win.visibleNames()}>
                {(name, i) => {
                    const entry = win.entryOf(name);
                    return (
                        <Show when={win.byName().has(name)}>
                            <FileRow
                                model={model}
                                entry={entry()}
                                index={win.range().first + i()}
                                selected={model.selection().names.has(name)}
                                focused={model.selection().focus === name}
                                renaming={model.renaming() === name}
                                git={model.gitStateOf().get(name)}
                                touch={win.touchOf(name)}
                                onClick={(e) => pointer.onRowClick(e, entry())}
                                onOpen={() => opening.openEntry(entry())}
                                onContextMenu={(e) => pointer.onRowContextMenu(e, entry())}
                                onDragStart={(e) => drag.onRowDragStart(e, entry())}
                                onDragOver={() => drag.onRowDragOver(entry())}
                                dropTarget={fv.dropRow() === name}
                                onDragEnd={() => endPathDrag()}
                                onRenameDone={() => fv.refs.list?.focus()}
                            />
                        </Show>
                    );
                }}
            </For>
        </div>
    );
}
