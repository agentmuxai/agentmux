// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What the Files pane keeps for itself: the scroll and size of the list, the open filter, path
 * editing, the menu and confirmation on screen, the folder row under a drag, and the elements it
 * reaches into. Split out of FilesView; every hook of the pane is called in the order it was there.
 */

import type { ContextMenuItem } from "@/app/components/context-menu";
import { holdPaneContent, trackPaneContent } from "@/app/store/pane-content-holds";
import type { PaneTabHostContext } from "@/app/store/pane-tab-registry";
import { createEffect, createSignal, on, onCleanup } from "solid-js";
import { createColumnWidths } from "./files-columns";
import type { FilesModel } from "./files-model";
import { TypeAhead } from "./typeahead";

export type Confirm = { title: string; message: string; confirmLabel: string; onConfirm: () => void };
export type FilesLocal = ReturnType<typeof useFilesLocal>;

/** The elements the pane focuses, scrolls and measures; filled in as they are drawn. */
interface FilesRefs {
    list?: HTMLDivElement;
    pathInput?: HTMLInputElement;
    crumbs?: HTMLElement;
    filterInput?: HTMLInputElement;
    filterButton?: HTMLElement;
}

export function useFilesLocal(model: FilesModel, ctx: PaneTabHostContext) {
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
    const refs: FilesRefs = {};
    const openFilter = (): void => {
        setFilterOpen(true);
        queueMicrotask(() => {
            refs.filterInput?.focus();
            refs.filterInput?.select();
        });
    };
    // Escape: clear the filter and hide the box.
    const closeFilter = (): void => {
        model.setFilter("");
        setFilterOpen(false);
        refs.list?.focus();
    };
    // Hide the box and keep what it says applied (the toolbar button shows that a filter is on).
    const hideFilter = (): void => {
        setFilterOpen(false);
    };
    const toggleFilter = (): void => (filterOpen() ? hideFilter() : openFilter());
    // A press outside the box and the button hides the box, while it is shown.
    createEffect(() => {
        if (!filterOpen()) return;
        const onPress = (e: Event): void => {
            const target = e.target;
            if (target instanceof Node && (refs.filterInput?.contains(target) || refs.filterButton?.contains(target))) return;
            hideFilter();
        };
        document.addEventListener("pointerdown", onPress, true);
        onCleanup(() => document.removeEventListener("pointerdown", onPress, true));
    });
    // Navigating clears the filter (the model does); close the box with it.
    createEffect(
        on(model.path, () => {
            if (!model.filter()) setFilterOpen(false);
        })
    );
    const [menu, setMenu] = createSignal<{ x: number; y: number; items: ContextMenuItem[] } | null>(null);
    const [confirm, setConfirm] = createSignal<Confirm | null>(null);
    const typeahead = new TypeAhead();
    // The folder row under a drag, which a drop goes into (§8.2).
    const [dropRow, setDropRow] = createSignal<string | null>(null);

    // ── Path editing (Ctrl+L) ──────────────────────────────────────────────

    const startEditingPath = (): void => {
        setEditingPath(true);
        queueMicrotask(() => {
            refs.pathInput?.focus();
            refs.pathInput?.select();
        });
    };

    const commitPath = (value: string): void => {
        setEditingPath(false);
        const v = value.trim();
        if (v && v !== model.path()) void model.navigate(v);
        refs.list?.focus();
    };

    return {
        model,
        ctx,
        refs,
        scrollTop, setScrollTop,
        viewHeight, setViewHeight,
        viewWidth, setViewWidth,
        editingPath, setEditingPath,
        filterOpen,
        openFilter, closeFilter, hideFilter, toggleFilter,
        columns: createColumnWidths(ctx),
        menu, setMenu,
        confirm, setConfirm,
        typeahead,
        dropRow, setDropRow,
        startEditingPath, commitPath,
    };
}
