// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The windowed list of the Files pane: which rows are in view, the entry behind each name, the
 * agent touches on them, the one the preview shows, and scrolling the focused row into view.
 * Split out of FilesView; the hooks are called in the order they were there.
 */

import { childKey, touched, touchesUnder } from "@/app/store/touched-files";
import type { FsEntry } from "@/types/rpc/FsEntry";
import { createEffect, createMemo, createSignal, on, onCleanup } from "solid-js";
import { OVERSCAN, ROW_HEIGHT, TILE_H, TILE_W } from "./files-rows";
import type { FilesLocal } from "./files-view-state";

export type FilesWindow = ReturnType<typeof useFilesWindow>;

export function useFilesWindow(fv: FilesLocal) {
    const { model, refs, scrollTop, setScrollTop, viewHeight, viewWidth } = fv;
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
        const listEl = refs.list;
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

    return { entries, order, gridCols, grid, range, visibleNames, byName, entryOf, touchOf, previewEntry, focusIndex, reveal };
}
