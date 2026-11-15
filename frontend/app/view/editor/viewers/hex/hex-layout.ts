// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The hex viewer's arithmetic: rows and offsets, the scroll position of a
 * file with more rows than an element can be tall, and how a byte and an
 * offset are written. Pure.
 * docs/specs/SPEC_EDITOR_REVAMP_UNIFIED_VIEWERS_2026_10_15.md §4.4.5.
 */

/** A row's height, in CSS pixels (the stylesheet reads it as `--hex-row-h`). */
export const ROW_H = 20;

/** Browsers cap an element's height (Chromium at about 33 million pixels).
 *  A file with more rows than fit below this scrolls in proportion. */
export const MAX_SCROLL_PX = 8_000_000;

export type RowWidth = 8 | 16 | 32;
export const ROW_WIDTHS: readonly RowWidth[] = [8, 16, 32];

/** Rows of `width` bytes a file of `size` bytes takes; an empty file has none. */
export function rowCount(size: number, width: number): number {
    return Math.ceil(size / width);
}

/** The height of the scrolled content for `rows` rows. */
export function contentHeight(rows: number): number {
    return Math.min(rows * ROW_H, MAX_SCROLL_PX);
}

/** Whether `rows` rows scroll in proportion (one pixel is more than one row). */
export function isScaled(rows: number): boolean {
    return rows * ROW_H > MAX_SCROLL_PX;
}

/** Rows that fit a view `viewPx` tall, whole ones only, at least one. */
export function rowsInView(viewPx: number): number {
    return Math.max(1, Math.floor(viewPx / ROW_H));
}

/** The last row that can be the first one shown. */
export function maxFirstRow(rows: number, viewPx: number): number {
    return Math.max(0, rows - rowsInView(viewPx));
}

/** The first row shown at `scrollTop`. */
export function firstRowAt(scrollTop: number, rows: number, viewPx: number): number {
    const last = maxFirstRow(rows, viewPx);
    if (!isScaled(rows)) return Math.min(last, Math.max(0, Math.floor(scrollTop / ROW_H)));
    const reach = MAX_SCROLL_PX - viewPx;
    if (reach <= 0) return 0;
    return Math.min(last, Math.max(0, Math.round((scrollTop / reach) * last)));
}

/** The `scrollTop` that shows `row` first. */
export function scrollTopFor(row: number, rows: number, viewPx: number): number {
    const last = maxFirstRow(rows, viewPx);
    const at = Math.min(last, Math.max(0, row));
    if (!isScaled(rows)) return at * ROW_H;
    const reach = MAX_SCROLL_PX - viewPx;
    return last === 0 || reach <= 0 ? 0 : Math.round((at / last) * reach);
}

const HEX = Array.from({ length: 256 }, (_, b) => b.toString(16).toUpperCase().padStart(2, "0"));

/** A byte as two hex digits. */
export function hexOf(byte: number): string {
    return HEX[byte & 0xff];
}

/** A byte in the decoded column: its ASCII character, or a dot. */
export function charOf(byte: number): string {
    return byte >= 0x20 && byte <= 0x7e ? String.fromCharCode(byte) : ".";
}

/** Hex digits an offset column needs for a file of `size` bytes: 8, or more. */
export function offsetDigits(size: number): number {
    return Math.max(8, Math.max(0, size - 1).toString(16).length);
}

/** An offset as the offset column writes it. */
export function offsetText(offset: number, digits: number): string {
    return offset.toString(16).toUpperCase().padStart(digits, "0");
}

/** What was typed in the offset box: decimal, or hex with `0x` (or a trailing
 *  `h`). Null when it isn't a number or is past the end of the file. */
export function parseOffset(text: string, size: number): number | null {
    const t = text.trim().replace(/[\s_,]/g, "");
    let n: number;
    if (/^0x[0-9a-f]+$/i.test(t)) n = Number.parseInt(t.slice(2), 16);
    else if (/^[0-9a-f]+h$/i.test(t)) n = Number.parseInt(t.slice(0, -1), 16);
    else if (/^[0-9]+$/.test(t)) n = Number.parseInt(t, 10);
    else return null;
    return Number.isSafeInteger(n) && n >= 0 && n < size ? n : null;
}

/** Where a key moves the cursor: an offset inside the file. `page` is the
 *  rows a page key moves. Null for a key that doesn't move it. */
export function moveCursor(key: string, mod: boolean, at: number, size: number, width: number, page: number): number | null {
    if (size <= 0) return null;
    const last = size - 1;
    const rowStart = at - (at % width);
    let to: number;
    switch (key) {
        case "ArrowLeft":
            to = at - 1;
            break;
        case "ArrowRight":
            to = at + 1;
            break;
        case "ArrowUp":
            to = at - width >= 0 ? at - width : at;
            break;
        case "ArrowDown":
            to = at + width <= last ? at + width : at;
            break;
        case "PageUp":
            to = at - width * page >= 0 ? at - width * page : at % width;
            break;
        case "PageDown":
            to = at + width * page <= last ? at + width * page : at;
            break;
        case "Home":
            to = mod ? 0 : rowStart;
            break;
        case "End":
            to = mod ? last : rowStart + width - 1;
            break;
        default:
            return null;
    }
    return Math.min(last, Math.max(0, to));
}
