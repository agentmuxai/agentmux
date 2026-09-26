// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * LogView — Layer 2's read-only install console
 * (SPEC_UNIVERSAL_INSTALL_DIALOG_2026_09_23.md §4.2). A virtualised list of
 * fixed-height monospace rows rendered straight from the session's retained
 * log, replacing xterm.js: no focus trap, no FitAddon sizing, no scrollback
 * smaller than the log, and native text selection.
 *
 * Only the rows in view (plus overscan) are in the DOM, so a 20,000-line
 * log costs the same as a short one. Lines don't wrap — long lines scroll
 * horizontally — which keeps every row the same height and makes "scroll
 * to line N" exact.
 *
 * Sticks to the bottom while output streams unless the user scrolled up
 * (40px threshold, re-checked inside the rAF — the pattern proven in
 * ToolOverlayLog.tsx and SystemToolInstallInline, codex P2 on PR #3165).
 */

import { createEffect, createMemo, createSignal, For, on, onCleanup, onMount, type JSX } from "solid-js";

import { ContextMenuModel } from "@/app/store/contextmenu";
import { redactSecrets } from "@/app/errors/redact";
import { writeText as clipboardWriteText } from "@/util/clipboard";

import { parseAnsi } from "./ansi";
import type { LogLine } from "./install-session";
import "./log-view.scss";

/** Must match `.install-log-line` line-height in log-view.scss. */
export const LOG_ROW_PX = 16;
const OVERSCAN = 20;
// Used before the element has a layout size (e.g. inside a closed <details>).
const FALLBACK_VIEW_PX = 200;
const STICK_THRESHOLD_PX = 40;

export interface LogViewApi {
    /** Brings line `index` (into `lines()`) near the top and stops following. */
    scrollToLine(index: number): void;
    /** Re-measures, and jumps to the newest line if following. Call when shown. */
    refresh(): void;
}

interface LogViewProps {
    lines: () => readonly LogLine[];
    trimmedLines: () => number;
    /** Text for the context menu's Copy all. */
    copyAllText: () => string;
    apiRef?: (api: LogViewApi) => void;
}

interface Row {
    text: string;
    tone: LogLine["tone"] | "marker";
}

export const LogView = (props: LogViewProps): JSX.Element => {
    let el: HTMLDivElement | undefined;
    let stickToBottom = true;
    // Where this component last scrolled itself. A programmatic scroll's
    // event arrives a frame later, often after a burst of lines has grown
    // the log, so it reads as "the user left the bottom". An event at the
    // position we set ourselves is ours and never changes stickiness —
    // seen live on a real install, where following stopped mid-stream.
    let ownScrollTop: number | null = null;
    const scrollTo = (top: number) => {
        if (!el) return;
        el.scrollTop = top;
        ownScrollTop = el.scrollTop;
    };
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
        if (!el) return;
        if (el.clientHeight > 0) setViewPx(el.clientHeight);
        setScrollTop(el.scrollTop);
    };

    const followIfStuck = () => {
        requestAnimationFrame(() => {
            // Re-check at execution time: the user may have scrolled away
            // between scheduling and now.
            if (stickToBottom && el && el.isConnected) {
                scrollTo(el.scrollHeight);
                measure();
            }
        });
    };

    const onScroll = () => {
        if (!el) return;
        if (el.scrollTop !== ownScrollTop) {
            ownScrollTop = null;
            stickToBottom = el.scrollHeight - el.scrollTop - el.clientHeight < STICK_THRESHOLD_PX;
        }
        measure();
    };

    // Follow new output. A log that shrank was cleared for a new run
    // (Retry), which starts following again whatever the last run's
    // scroll-away state was.
    createEffect(
        on(
            () => props.lines().length + props.trimmedLines(),
            (size, prev) => {
                if (prev !== undefined && size < prev) stickToBottom = true;
                followIfStuck();
            },
        ),
    );

    onMount(() => {
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
                stickToBottom = false;
                scrollTo(Math.max(0, (index + markerRows() - 2) * LOG_ROW_PX));
                measure();
            },
            refresh: () => {
                measure();
                followIfStuck();
            },
        });
    });

    return (
        <div
            class="install-log"
            ref={el}
            role="log"
            aria-label="Install output"
            tabIndex={0}
            onScroll={onScroll}
            onContextMenu={(e) => {
                e.preventDefault();
                const sel = window.getSelection()?.toString() ?? "";
                // SPEC_ERROR_COPY_EVERYWHERE_2026_09_24.md §5 — moved to the
                // redacted path. This log is install diagnostic output, not
                // an ordinary text view (editor/terminal/chat code block) —
                // the spec's "a user's own selection isn't redacted" carve-out
                // is scoped to those, not this surface.
                ContextMenuModel.showContextMenu(
                    [
                        {
                            label: "Copy",
                            enabled: sel.length > 0,
                            click: () => void clipboardWriteText(redactSecrets(sel)).catch((err) => console.log("clipboard write failed", err)),
                        },
                        {
                            label: "Copy All",
                            enabled: props.lines().length > 0,
                            click: () =>
                                void clipboardWriteText(redactSecrets(props.copyAllText())).catch((err) =>
                                    console.log("clipboard write failed", err),
                                ),
                        },
                    ],
                    e,
                );
            }}
        >
            <div class="install-log-spacer" style={{ height: `${total() * LOG_ROW_PX}px` }}>
                <div class="install-log-window" style={{ transform: `translateY(${visible().first * LOG_ROW_PX}px)` }}>
                    <For each={visible().rows}>
                        {(row) => (
                            <div class="install-log-line" data-tone={row.tone}>
                                <For each={parseAnsi(row.text)}>
                                    {(seg) => (seg.cls ? <span class={seg.cls}>{seg.text}</span> : seg.text)}
                                </For>
                            </div>
                        )}
                    </For>
                </div>
            </div>
        </div>
    );
};
