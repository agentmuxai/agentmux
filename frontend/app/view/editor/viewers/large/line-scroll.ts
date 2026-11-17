// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Which lines of the Large text viewer are on screen. A file with more lines
 * than an element can be tall scrolls in proportion, and the wheel then moves
 * it a line at a time. The arithmetic is the hex viewer's (a line is a row).
 * docs/specs/SPEC_EDITOR_REVAMP_UNIFIED_VIEWERS_2026_10_15.md §4.4.6.
 */

import { createEffect, createMemo, createSignal, on, onCleanup, type Accessor } from "solid-js";
import { firstRowAt, isScaled, maxFirstRow, ROW_H, rowsInView, scrollTopFor } from "../hex/hex-layout";

export function createLineScroll(lines: Accessor<number>, initial = 0) {
    const [first, setFirst] = createSignal(initial);
    const [viewPx, setViewPx] = createSignal(0);
    const [top, setTop] = createSignal(0);
    let el: HTMLDivElement | undefined;
    // The scrollTop this viewer set itself: its scroll event isn't the user's.
    let own: number | null = null;

    const showLine = (line: number): void => {
        const at = Math.min(maxFirstRow(lines(), viewPx()), Math.max(0, line));
        const to = scrollTopFor(at, lines(), viewPx());
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
        setFirst(firstRowAt(at, lines(), viewPx()));
    };
    const onWheel = (e: WheelEvent): void => {
        // Ctrl+wheel stays pane zoom; a file that scrolls by pixels scrolls natively.
        if (e.ctrlKey || e.metaKey || !isScaled(lines()) || e.deltaY === 0) return;
        e.preventDefault();
        const rows = e.deltaMode === 1 ? e.deltaY : e.deltaMode === 2 ? e.deltaY * rowsInView(viewPx()) : e.deltaY / ROW_H;
        if (rows) showLine(first() + Math.sign(rows) * Math.max(1, Math.round(Math.abs(rows))));
    };
    /** The scroller's `ref`. */
    const bind = (node: HTMLDivElement): void => {
        el = node;
        // A hidden pane puts a scroller's scrollTop back to 0 and says nothing: a resize is the word that it
        // is shown again, so put the scroller where the lines are.
        const measure = () => {
            setViewPx(Math.max(0, node.clientHeight));
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
    // The file got longer or shorter: put the first line and the scroller right for the new count.
    createEffect(on(lines, () => showLine(first()), { defer: true }));
    const fits = () => rowsInView(viewPx());
    /** Scroll as little as shows `line`. */
    const reveal = (line: number): void => {
        if (line < first()) showLine(line);
        else if (line > first() + fits() - 1) showLine(line - fits() + 1);
    };
    /** Where the lines on screen sit in the scrolled content. */
    const layerTop = () => (isScaled(lines()) ? top() : first() * ROW_H);
    /** The lines to draw: those on screen and two more below them. */
    const shown = createMemo(() => {
        const from = Math.min(first(), Math.max(0, lines() - 1));
        const count = Math.min(lines() - from, fits() + 2);
        return Array.from({ length: Math.max(0, count) }, (_, i) => from + i);
    });
    return { first, viewPx, fits, shown, layerTop, showLine, reveal, onScroll, bind, focus: () => el?.focus({ preventScroll: true }) };
}

export type LineScroll = ReturnType<typeof createLineScroll>;
