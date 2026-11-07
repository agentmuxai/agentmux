// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// LogView, the state: which rows of the log are drawn for the scroll position, the follow-the-output machine that
// keeps the newest line in sight, and the context menu's copy. (log-view-parts.tsx draws the rows.)

import { createEffect, createMemo, createSignal, on, onCleanup, onMount } from "solid-js";

import { openMenuAt } from "@/app/element/ui";
import { MENU_ICON } from "@/app/util/menu-icons";
import { redactSecrets } from "@/app/errors/redact";
import { writeText as clipboardWriteText } from "@/util/clipboard";
import { createFollowScroll, type FollowScroll } from "@/app/scroll/follow-scroll";

import type { LogLine } from "./install-session";

/** Must match `.install-log-line` line-height in log-view.scss. */
export const LOG_ROW_PX = 16;
const OVERSCAN = 20;
// Used before the element has a layout size (e.g. inside a closed <details>).
const FALLBACK_VIEW_PX = 200;

export interface LogViewApi {
    /** Brings line `index` (into `lines()`) near the top and stops following. */
    scrollToLine(index: number): void;
    /** Re-measures, and jumps to the newest line if following. Call when shown. */
    refresh(): void;
}

export interface LogViewProps {
    lines: () => readonly LogLine[];
    trimmedLines: () => number;
    /** Text for the context menu's Copy all. */
    copyAllText: () => string;
    apiRef?: (api: LogViewApi) => void;
}

export interface Row {
    text: string;
    tone: LogLine["tone"] | "marker";
}

export type LogViewState = ReturnType<typeof createLogView>;

const geometry = (e: HTMLElement) => ({ scrollTop: e.scrollTop, scrollHeight: e.scrollHeight, clientHeight: e.clientHeight });

/** Which rows are drawn: the scroll position and the box's height, the marker row for trimmed lines, and the window with overscan. */
function createLogWindow(props: LogViewProps, el: () => HTMLDivElement | undefined) {
    const [scrollTop, setScrollTop] = createSignal(0);
    const [viewPx, setViewPx] = createSignal(FALLBACK_VIEW_PX);

    // Row 0 is a marker when earlier lines were trimmed.
    const markerRows = () => (props.trimmedLines() > 0 ? 1 : 0);
    const total = () => props.lines().length + markerRows();

    const visible = createMemo(() => {
        const lines = props.lines();
        const marker = markerRows();
        const count = lines.length + marker;
        const first = Math.max(0, Math.floor(scrollTop() / LOG_ROW_PX) - OVERSCAN);
        const last = Math.min(count, Math.ceil((scrollTop() + viewPx()) / LOG_ROW_PX) + OVERSCAN);
        const rows: Row[] = [];
        for (let i = first; i < last; i++) {
            if (i < marker) rows.push({ text: `… ${props.trimmedLines()} earlier lines trimmed`, tone: "marker" });
            else rows.push(lines[i - marker]);
        }
        return { first, rows };
    });

    const measure = () => {
        const e = el();
        if (!e) return;
        if (e.clientHeight > 0) setViewPx(e.clientHeight);
        setScrollTop(e.scrollTop);
    };
    return { markerRows, total, visible, measure };
}

export function createLogView(props: LogViewProps) {
    let el: HTMLDivElement | undefined;
    const setEl = (e: HTMLDivElement) => {
        el = e;
    };
    // The follow state machine (bound on mount).
    let follow: FollowScroll | null = null;
    const scrollTo = (top: number) => {
        if (!el) return;
        el.scrollTop = top;
    };
    const { markerRows, total, visible, measure } = createLogWindow(props, () => el);

    const followIfStuck = () => {
        requestAnimationFrame(() => {
            // Re-check at execution time: the user may have scrolled away
            // between scheduling and now.
            if (!el?.isConnected || !follow?.observed("content", geometry(el))) return;
            scrollTo(el.scrollHeight);
            follow.noteGeometry(geometry(el));
            measure();
        });
    };

    const onScroll = () => {
        if (!el) return;
        follow?.scrolled(geometry(el), follow.windowOpen() || follow.pointerHeld());
        measure();
    };

    // Follow new output. A log that shrank was cleared for a new run
    // (Retry), which starts following again whatever the last run's
    // scroll-away state was.
    createEffect(
        on(
            () => props.lines().length + props.trimmedLines(),
            (size, prev) => {
                if (prev !== undefined && size < prev) {
                    follow?.reset();
                }
                followIfStuck();
            },
        ),
    );

    onMount(() => {
        if (el) {
            follow = createFollowScroll(el, {
                pin: followIfStuck,
                loadOlder: () => {},
                hasOlderHistory: () => false,
                loadingOlder: () => false,
                transition: () => {},
            });
            onCleanup(() => follow?.dispose());
        }
        measure();
        // A resize with no scroll — the narrow-pane breakpoint changing the
        // box from 200px to 140px, a window resize, Details opening — would
        // otherwise leave the rendered window sized for the old height
        // (ReAgent P2 on #3684). Absent in jsdom.
        if (el && typeof ResizeObserver !== "undefined") {
            const ro = new ResizeObserver(() => measure());
            ro.observe(el);
            onCleanup(() => ro.disconnect());
        }
        props.apiRef?.({
            scrollToLine: (index) => {
                if (!el) return;
                follow?.jumpToNode(`line-${index}`);
                scrollTo(Math.max(0, (index + markerRows() - 2) * LOG_ROW_PX));
                measure();
            },
            refresh: () => {
                measure();
                followIfStuck();
            },
        });
    });

    return { setEl, onScroll, total, visible, onContextMenu: (e: MouseEvent) => openLogMenu(props, e) };
}

/** Copy and Copy All, through the secret redactor. */
function openLogMenu(props: LogViewProps, e: MouseEvent): void {
    e.preventDefault();
    const sel = window.getSelection()?.toString() ?? "";
    // SPEC_ERROR_COPY_EVERYWHERE_2026_09_24.md §5 — moved to the
    // redacted path. This log is install diagnostic output, not
    // an ordinary text view (editor/terminal/chat code block) —
    // the spec's "a user's own selection isn't redacted" carve-out
    // is scoped to those, not this surface.
    openMenuAt(
        [
            {
                label: "Copy",
                icon: MENU_ICON.copy,
                enabled: sel.length > 0,
                click: () => void clipboardWriteText(redactSecrets(sel)).catch((err) => console.log("clipboard write failed", err)),
            },
            {
                label: "Copy All",
                icon: MENU_ICON.copy,
                enabled: props.lines().length > 0,
                click: () =>
                    void clipboardWriteText(redactSecrets(props.copyAllText())).catch((err) =>
                        console.log("clipboard write failed", err),
                    ),
            },
        ],
        e,
    );
}
