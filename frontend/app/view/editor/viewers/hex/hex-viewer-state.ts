// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The hex viewer's state: the file's bytes (read in parts), the rows on
 * screen, the cursor and the selection, and what the keys, the pointer and
 * the toolbar do. Read-only: nothing here writes the file.
 * docs/specs/SPEC_EDITOR_REVAMP_UNIFIED_VIEWERS_2026_10_15.md §4.4.5.
 */

import { pushNotification } from "@/app/store/global";
import { writeText as clipboardWriteText } from "@/util/clipboard";
import { formatBytes } from "@/util/format-bytes";
import { createComputed, createEffect, createMemo, createSignal, createUniqueId, on, onCleanup, onMount, untrack, type Accessor } from "solid-js";
import type { ViewerBarProps } from "../viewer-bar";
import { ChunkCache, fileReaderOf, rangeReaderOf } from "./hex-chunks";
import { COPY_MAX_BYTES, copyText, type CopyAs } from "./hex-copy";
import { createHexFind } from "./hex-find-state";
import { inspect, INSPECT_BYTES, type Inspected } from "./hex-inspector";
import {
    charOf,
    firstRowAt,
    hexOf,
    isScaled,
    maxFirstRow,
    moveCursor,
    offsetDigits,
    offsetText,
    parseOffset,
    ROW_H,
    rowCount,
    rowsInView,
    scrollTopFor,
    type RowWidth,
} from "./hex-layout";

export interface HexViewerProps {
    /** The file on disk ("" for dropped bytes). */
    path: string;
    /** Dropped bytes with no host path, shown instead of `path`. */
    file?: File;
    /** Bumped when the file changed in place: read it again. */
    revision: number;
    /** The file's size, from srv's probe. */
    size?: number;
    blockId?: string;
    /** Names the tab: where it was is kept while the app runs, so coming back to the tab doesn't start at 0. */
    memoryKey: string;
    bar: Omit<ViewerBarProps, "start" | "facts">;
}

export type HexColumn = "hex" | "text";

interface Remembered {
    first: number;
    cursor: number;
    anchor: number;
    width: RowWidth;
    inspector: boolean;
    littleEndian: boolean;
}

const REMEMBERED_MAX = 50;
const remembered = new Map<string, Remembered>();

function remember(key: string, state: Remembered): void {
    remembered.delete(key);
    remembered.set(key, state);
    while (remembered.size > REMEMBERED_MAX) remembered.delete(remembered.keys().next().value as string);
}

/** For tests: forget where every tab was. */
export function forgetHexMemory(): void {
    remembered.clear();
}

/** The file's bytes, a part at a time; a part that arrives redraws the rows. */
function createHexSource(props: HexViewerProps) {
    const [cache, setCache] = createSignal<ChunkCache | null>(null);
    const [size, setSize] = createSignal(0);
    const [loaded, setLoaded] = createSignal(0);
    const [error, setError] = createSignal("");

    createComputed(() => {
        void props.revision;
        const file = props.file;
        const total = file ? file.size : (props.size ?? 0);
        const read = file ? fileReaderOf(file) : props.path ? rangeReaderOf(props.path, props.blockId) : null;
        setSize(read ? total : 0);
        setError("");
        setCache(read ? new ChunkCache(read, total) : null);
        setLoaded((v) => v + 1);
    });

    /** Read what `start..=end` needs, unless it is here. */
    const want = (start: number, end: number): void => {
        const c = untrack(cache);
        if (!c) return;
        if (c.has(start, end)) return c.touch(start, end);
        void c.ensure(start, end).then(
            () => {
                if (untrack(cache) !== c) return;
                setError("");
                setLoaded((v) => v + 1);
            },
            (e) => {
                if (untrack(cache) === c) setError(`Couldn't read the file: ${e instanceof Error ? e.message : String(e)}`);
            }
        );
    };
    const byteAt = (offset: number): number | undefined => {
        void loaded();
        return cache()?.byteAt(offset);
    };
    const retry = () => {
        setError("");
        setLoaded((v) => v + 1);
    };
    return { cache, size, loaded, error, want, byteAt, retry };
}

/** Which rows are on screen. A file with more rows than an element can be
 *  tall scrolls in proportion, and the wheel moves it a row at a time. */
function createHexScroll(size: Accessor<number>, width: Accessor<RowWidth>) {
    const [first, setFirst] = createSignal(0);
    const [viewPx, setViewPx] = createSignal(0);
    const [top, setTop] = createSignal(0);
    const rows = createMemo(() => rowCount(size(), width()));
    let el: HTMLDivElement | undefined;
    // The scrollTop this viewer set itself: its scroll event isn't the user's.
    let own: number | null = null;

    const showRow = (row: number): void => {
        const at = Math.min(maxFirstRow(rows(), viewPx()), Math.max(0, row));
        const to = scrollTopFor(at, rows(), viewPx());
        own = to;
        setFirst(at);
        setTop(to);
        if (el) el.scrollTop = to;
    };
    const onScroll = (): void => {
        // A pane that is hidden has no height, and its scroller reports 0: not where the user scrolled to.
        if (!el || el.clientHeight === 0) return;
        const at = el.scrollTop;
        setTop(at);
        if (own != null && Math.abs(at - own) < 1) return;
        own = null;
        setFirst(firstRowAt(at, rows(), viewPx()));
    };
    const onWheel = (e: WheelEvent): void => {
        // Ctrl+wheel stays pane zoom; a file that scrolls by pixels scrolls natively.
        // Sideways (a trackpad's horizontal swipe) is the browser's, for a narrow pane's other columns.
        if (e.ctrlKey || e.metaKey || !isScaled(rows()) || e.deltaY === 0) return;
        e.preventDefault();
        const lines = e.deltaMode === 1 ? e.deltaY : e.deltaMode === 2 ? e.deltaY * rowsInView(viewPx()) : e.deltaY / ROW_H;
        if (lines) showRow(first() + Math.sign(lines) * Math.max(1, Math.round(Math.abs(lines))));
    };
    /** The scroller's `ref`. The header row inside it takes one row of its height. */
    const bind = (node: HTMLDivElement): void => {
        el = node;
        // The browser puts a scroller's scrollTop back to 0 when its pane is hidden (maximized past, a tab
        // switched away) and says nothing: the rows stay where they were drawn. A resize is the word that
        // it is shown again, so put the scroller where the rows are.
        const measure = () => {
            setViewPx(Math.max(0, node.clientHeight - ROW_H));
            const at = top();
            if (node.clientHeight > 0 && Math.abs(node.scrollTop - at) >= 1) {
                own = at;
                node.scrollTop = at;
            }
        };
        measure();
        node.addEventListener("wheel", onWheel, { passive: false });
        const observer = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(measure);
        observer?.observe(node);
        onCleanup(() => {
            node.removeEventListener("wheel", onWheel);
            observer?.disconnect();
        });
    };
    // The file got shorter (truncated) or longer: the first row and the scroller are put right for the new row
    // count, which no scroll event does when the scrolled height comes out the same.
    createEffect(on(rows, () => showRow(first()), { defer: true }));
    /** Scroll as little as shows `row`. */
    const reveal = (row: number): void => {
        const fits = rowsInView(viewPx());
        if (row < first()) showRow(row);
        else if (row > first() + fits - 1) showRow(row - fits + 1);
    };
    /** Where the rows on screen sit in the scrolled content. */
    const layerTop = () => (isScaled(rows()) ? top() : first() * ROW_H);
    const shown = createMemo(() => {
        const from = Math.min(first(), Math.max(0, rows() - 1));
        const count = Math.min(rows() - from, rowsInView(viewPx()) + 2);
        return Array.from({ length: Math.max(0, count) }, (_, i) => from + i);
    });
    return { first, viewPx, rows, shown, layerTop, showRow, reveal, onScroll, bind, focus: () => el?.focus({ preventScroll: true }) };
}

function notify(title: string, message: string): void {
    pushNotification({ icon: "fa-triangle-exclamation", title, message, timestamp: new Date().toISOString(), type: "error", expiration: Date.now() + 8000 });
}

type HexSource = ReturnType<typeof createHexSource>;
type HexScroll = ReturnType<typeof createHexScroll>;

/** The cursor and the selection, which runs from the anchor to the cursor. */
function createHexSelection(size: Accessor<number>, width: Accessor<RowWidth>, scroll: HexScroll, was: Remembered | undefined) {
    const [cursor, setCursor] = createSignal(was?.cursor ?? 0);
    const [anchor, setAnchor] = createSignal(was?.anchor ?? 0);
    const [column, setColumn] = createSignal<HexColumn>("hex");
    // The file got shorter: the cursor stays inside it.
    createComputed(() => {
        const last = Math.max(0, size() - 1);
        if (untrack(cursor) > last) setCursor(last);
        if (untrack(anchor) > last) setAnchor(last);
    });
    const lo = () => Math.min(anchor(), cursor());
    const hi = () => Math.max(anchor(), cursor());
    /** Put the cursor on `offset`; `extend` keeps the anchor, so the selection grows to it. */
    const place = (offset: number, extend: boolean): void => {
        setCursor(offset);
        if (!extend) setAnchor(offset);
    };
    /** A key's move: scroll as little as shows the cursor. */
    const moveTo = (offset: number, extend: boolean): void => {
        place(offset, extend);
        scroll.reveal(Math.floor(offset / width()));
    };
    /** A jump: a little context above it, when it isn't on screen already. */
    const goTo = (offset: number): void => {
        place(offset, false);
        const row = Math.floor(offset / width());
        if (row < scroll.first() || row > scroll.first() + rowsInView(scroll.viewPx()) - 1) scroll.showRow(row - 2);
    };
    /** A match: its bytes selected, scrolled into view. */
    const select = (offset: number, length: number): void => {
        goTo(offset);
        if (length > 1) place(offset + length - 1, true);
    };
    const selectAll = (): void => {
        setAnchor(0);
        setCursor(Math.max(0, size() - 1));
    };
    return { cursor, anchor, column, setColumn, lo, hi, place, moveTo, goTo, select, selectAll };
}

type HexSelection = ReturnType<typeof createHexSelection>;

/** The selection, or the cursor's offset, to the clipboard. */
function createHexCopy(source: HexSource, sel: HexSelection) {
    const copy = async (as: CopyAs): Promise<void> => {
        const cache = source.cache();
        if (!cache || !source.size()) return;
        if (sel.hi() - sel.lo() + 1 > COPY_MAX_BYTES) return notify("Too much to copy", `Select ${formatBytes(COPY_MAX_BYTES)} or less to copy it.`);
        try {
            await clipboardWriteText(copyText(await cache.bytes(sel.lo(), sel.hi()), as));
        } catch (e) {
            notify("Couldn't copy", e instanceof Error ? e.message : String(e));
        }
    };
    const copyOffset = () => void clipboardWriteText(`0x${offsetText(sel.cursor(), 1)}`).catch(() => {});
    return { copy, copyOffset };
}

/** What a key does in the grid; a key that does nothing here is left to the app. */
function hexKeyHandler(o: {
    size: Accessor<number>;
    width: Accessor<RowWidth>;
    scroll: HexScroll;
    sel: HexSelection;
    copy: (as: CopyAs) => Promise<void>;
    focusOffsetBox: () => void;
    openFind: () => void;
    toggleInspector: () => void;
}): (e: KeyboardEvent) => void {
    const act = (e: KeyboardEvent): boolean => {
        const mod = e.ctrlKey || e.metaKey;
        const key = e.key.length === 1 ? e.key.toLowerCase() : e.key;
        if (mod && key === "g") o.focusOffsetBox();
        else if (mod && key === "f") o.openFind();
        else if (mod && key === "c") void o.copy(o.sel.column() === "text" ? "text" : "hex");
        else if (mod && key === "a") o.sel.selectAll();
        else if (!mod && key === "i") o.toggleInspector();
        // Tab: from the hex column to the text column, then on out of the grid; Shift+Tab the other way.
        else if (!mod && key === "Tab" && o.sel.column() === (e.shiftKey ? "text" : "hex")) o.sel.setColumn(e.shiftKey ? "hex" : "text");
        else {
            // A modified key is the app's, but for Ctrl+Home and Ctrl+End.
            if (mod && key !== "Home" && key !== "End") return false;
            const to = moveCursor(key, mod, o.sel.cursor(), o.size(), o.width(), rowsInView(o.scroll.viewPx()));
            if (to == null) return false;
            o.sel.moveTo(to, e.shiftKey);
        }
        return true;
    };
    return (e) => {
        if (e.altKey || !o.size() || !act(e)) return;
        e.preventDefault();
        e.stopPropagation();
    };
}

/** A press on a byte puts the cursor there; a drag selects to where it goes. */
function createHexPointer(sel: HexSelection) {
    let dragging = false;
    const cellOf = (e: PointerEvent): { offset: number; column: HexColumn } | null => {
        const cell = (e.target as Element | null)?.closest?.("[data-o]");
        if (!cell) return null;
        return { offset: Number(cell.getAttribute("data-o")), column: cell.closest('[data-c="text"]') ? "text" : "hex" };
    };
    const stopDrag = () => (dragging = false);
    onMount(() => {
        window.addEventListener("pointerup", stopDrag);
        onCleanup(() => window.removeEventListener("pointerup", stopDrag));
    });
    const onPointerDown = (e: PointerEvent): void => {
        const at = cellOf(e);
        if (e.button !== 0 || !at) return;
        sel.setColumn(at.column);
        sel.place(at.offset, e.shiftKey);
        dragging = true;
    };
    const onPointerMove = (e: PointerEvent): void => {
        const at = dragging ? cellOf(e) : null;
        if (at) sel.place(at.offset, true);
    };
    return { onPointerDown, onPointerMove };
}

/** The bytes at the cursor as each type, and the bar's facts. */
function createHexReadouts(source: HexSource, sel: HexSelection, littleEndian: Accessor<boolean>) {
    const { size } = source;
    const inspected = createMemo<Inspected[]>(() => {
        const bytes: number[] = [];
        for (let o = sel.cursor(); o < size() && bytes.length < INSPECT_BYTES; o++) {
            const b = source.byteAt(o);
            if (b === undefined) break;
            bytes.push(b);
        }
        return inspect(Uint8Array.from(bytes), littleEndian());
    });
    const facts = (): string[] => {
        if (!size()) return [formatBytes(0)];
        const selected = sel.hi() - sel.lo() + 1;
        return [
            `Offset 0x${offsetText(sel.cursor(), 1)} (${sel.cursor().toLocaleString()})`,
            ...(selected > 1 ? [`${selected.toLocaleString()} bytes selected`] : []),
            formatBytes(size()),
        ];
    };
    return { inspected, facts };
}

/** Go to an offset, or find, from a toolbar button's focus too (the grid's own handler has taken its key already). */
function toolbarKeyHandler(o: { focusOffsetBox: () => void; openFind: () => void }): (e: KeyboardEvent) => void {
    return (e) => {
        if (e.defaultPrevented || e.altKey || !(e.ctrlKey || e.metaKey)) return;
        const key = e.key.toLowerCase();
        if (key !== "g" && key !== "f") return;
        e.preventDefault();
        if (key === "g") o.focusOffsetBox();
        else o.openFind();
    };
}

export function createHexViewer(props: HexViewerProps) {
    const was = remembered.get(props.memoryKey);
    const source = createHexSource(props);
    const { size } = source;
    const [width, setWidth] = createSignal<RowWidth>(was?.width ?? 16);
    const scroll = createHexScroll(size, width);
    const sel = createHexSelection(size, width, scroll, was);
    const [inspector, setInspector] = createSignal(was?.inspector ?? false);
    const [littleEndian, setLittleEndian] = createSignal(was?.littleEndian ?? true);
    const [badOffset, setBadOffset] = createSignal(false);
    const uid = createUniqueId();
    let offsetInput: HTMLInputElement | undefined;

    // The rows on screen, and the bytes the inspector reads past the cursor.
    createEffect(() => {
        void source.loaded();
        const rows = scroll.shown();
        if (!rows.length || !size()) return;
        source.want(rows[0] * width(), Math.min(size(), (rows[rows.length - 1] + 1) * width()) - 1);
        if (inspector()) source.want(sel.cursor(), Math.min(size() - 1, sel.cursor() + INSPECT_BYTES - 1));
    });
    onMount(() => {
        if (was && size() > 0) scroll.showRow(was.first);
    });
    onCleanup(() =>
        remember(props.memoryKey, {
            first: scroll.first(),
            cursor: sel.cursor(),
            anchor: sel.anchor(),
            width: width(),
            inspector: inspector(),
            littleEndian: littleEndian(),
        })
    );

    const toggleInspector = () => setInspector((v) => !v);
    const { copy, copyOffset } = createHexCopy(source, sel);
    const focusOffsetBox = () => {
        offsetInput?.focus();
        offsetInput?.select();
    };
    const find = createHexFind({ path: () => props.path, size, lo: sel.lo, hi: sel.hi, select: sel.select, focusGrid: scroll.focus });
    const goToTyped = (text: string): void => {
        const offset = parseOffset(text, size());
        setBadOffset(offset == null);
        if (offset == null) return;
        sel.goTo(offset);
        scroll.focus();
    };
    const setRowWidth = (next: RowWidth): void => {
        setWidth(next);
        scroll.showRow(Math.floor(sel.cursor() / next) - 2);
    };

    return {
        size,
        width,
        inspector,
        littleEndian,
        badOffset,
        column: sel.column,
        cursor: sel.cursor,
        digits: createMemo(() => offsetDigits(size())),
        ...createHexReadouts(source, sel, littleEndian),
        error: source.error,
        retry: source.retry,
        rows: scroll.rows,
        shownRows: scroll.shown,
        layerTop: scroll.layerTop,
        bindScroller: scroll.bind,
        onScroll: scroll.onScroll,
        onToolbarKeyDown: toolbarKeyHandler({ focusOffsetBox, openFind: find.openStrip }),
        find,
        onKeyDown: hexKeyHandler({ size, width, scroll, sel, copy, focusOffsetBox, openFind: find.openStrip, toggleInspector }),
        ...createHexPointer(sel),
        setRowWidth,
        setLittleEndian,
        toggleInspector,
        goToTyped,
        clearBadOffset: () => setBadOffset(false),
        copy,
        copyOffset,
        bindOffsetInput: (el: HTMLInputElement) => (offsetInput = el),
        cellId: (offset: number) => `hex-${uid}-${offset}`,
        isSelected: (offset: number) => offset >= sel.lo() && offset <= sel.hi(),
        hexAt: (offset: number) => {
            const b = source.byteAt(offset);
            return b === undefined ? "··" : hexOf(b);
        },
        charAt: (offset: number) => {
            const b = source.byteAt(offset);
            return b === undefined ? " " : charOf(b);
        },
    };
}

export type HexViewerState = ReturnType<typeof createHexViewer>;
