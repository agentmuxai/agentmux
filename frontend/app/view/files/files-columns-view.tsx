// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The columns view drawn: a strip of lists, one per level (files-columns-state.ts). Each column windows its own rows,
 * a row is the list's row with the name alone, and a click or a key in a column makes it the one the keys work in.
 */

import { createEffect, createMemo, createSignal, For, on, onCleanup, onMount, Show, type JSX } from "solid-js";
import type { ColumnView } from "./files-columns-state";
import { endPathDrag } from "@/app/drag/file-drop";
import { FileRow, OVERSCAN, ROW_HEIGHT } from "./files-rows";
import type { FilesViewParts } from "./files-view-parts";

/** One column: its rows in a list of its own, a few rows either side of what is in view. */
function FilesColumn(props: { v: FilesViewParts; dir: string; index: number }): JSX.Element {
    const { model, fv, win, drag, opening, pointer } = props.v;
    // Looked up by its folder: the columns are rebuilt on every change, and this one stays mounted (and scrolled).
    const col = createMemo<ColumnView>(() => model.cols.columns().find((c) => c.dir === props.dir) ?? { dir: props.dir, entries: [] });
    let box: HTMLDivElement | undefined;
    const [top, setTop] = createSignal(0);
    const [height, setHeight] = createSignal(400);
    const active = () => model.cols.activeColumn() === props.index;
    // The active column shows what the filter box lets through; the others show all they hold.
    const rows = () => (active() ? model.entries() : col().entries);
    const byName = createMemo(() => new Map(rows().map((e) => [e.name, e])));
    const range = createMemo(() => {
        const first = Math.max(0, Math.floor(top() / ROW_HEIGHT) - OVERSCAN);
        return { first, last: Math.min(rows().length, first + Math.ceil(height() / ROW_HEIGHT) + OVERSCAN * 2) };
    });
    const names = createMemo(() => rows().slice(range().first, range().last).map((e) => e.name));

    onMount(() => {
        if (!box) return;
        setHeight(box.clientHeight || 400);
        if (typeof ResizeObserver === "undefined") return;
        const watch = new ResizeObserver(() => box && setHeight(box.clientHeight || 400));
        watch.observe(box);
        onCleanup(() => watch.disconnect());
    });

    // The focused row stays inside its column's view.
    createEffect(
        on(
            () => model.selection().focus,
            (focus) => {
                const i = focus == null || !box ? -1 : rows().findIndex((e) => e.name === focus);
                if (i < 0 || !box) return;
                const rowTop = i * ROW_HEIGHT;
                if (rowTop < box.scrollTop) box.scrollTop = rowTop;
                else if (rowTop + ROW_HEIGHT > box.scrollTop + box.clientHeight) box.scrollTop = rowTop + ROW_HEIGHT - box.clientHeight;
                setTop(box.scrollTop);
            }
        )
    );

    /** A press in a column makes it the one the keys work in. */
    const enter = (): void => model.cols.focusColumn(props.index);

    return (
        <div ref={(el) => (box = el)} class="files-column" classList={{ "files-column-active": active() }} role="presentation" onScroll={(e) => setTop(e.currentTarget.scrollTop)}>
            <div class="files-rows" style={{ height: `${rows().length * ROW_HEIGHT}px` }}>
                <For each={names()}>
                    {(name, i) => {
                        const entry = () => byName().get(name);
                        return (
                            <Show when={entry()}>
                                {(e) => (
                                    <FileRow
                                        model={model}
                                        entry={e()}
                                        index={range().first + i()}
                                        selected={model.selection().names.has(name)}
                                        focused={model.selection().focus === name}
                                        renaming={model.renaming() === name}
                                        git={model.gitStateOf().get(name)}
                                        touch={win.touchOf(name)}
                                        onClick={(ev) => {
                                            enter();
                                            pointer.onRowClick(ev, e());
                                        }}
                                        onOpen={() => opening.openEntry(e())}
                                        onContextMenu={(ev) => {
                                            enter();
                                            pointer.onRowContextMenu(ev, e());
                                        }}
                                        onDragStart={(ev) => drag.onRowDragStart(ev, e())}
                                        onDragOver={() => drag.onRowDragOver(e())}
                                        dropTarget={fv.dropRow() === name}
                                        onDragEnd={() => endPathDrag()}
                                        onRenameDone={() => fv.refs.list?.focus()}
                                        inColumn={{
                                            // The active column's rows carry the ids the list's aria-activedescendant names.
                                            id: active() ? undefined : `files-${model.blockId}-c${props.index}-row-${range().first + i()}`,
                                            onPath: model.cols.onPath(name),
                                        }}
                                    />
                                )}
                            </Show>
                        );
                    }}
                </For>
            </div>
        </div>
    );
}

/** The strip of columns, the last one kept in view as columns are added. */
export function FilesColumns(props: { v: FilesViewParts }): JSX.Element {
    const { model } = props.v;
    let strip: HTMLDivElement | undefined;
    createEffect(
        on(
            () => model.cols.columns().length,
            () => {
                if (strip) strip.scrollLeft = strip.scrollWidth;
            }
        )
    );
    return (
        <div ref={(el) => (strip = el)} class="files-columns" role="presentation">
            <For each={["", ...model.cols.chain()]}>{(dir, i) => <FilesColumn v={props.v} dir={dir} index={i()} />}</For>
        </div>
    );
}
