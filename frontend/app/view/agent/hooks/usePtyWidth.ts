// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * usePtyWidth — observe the agent pane's width and push a matching
 * `cols` value to the backing PTY whenever it changes.
 *
 * Background: the agent pane is a custom UI, not an xterm.js terminal,
 * so the PTY that hosts the agent CLI never sees a resize event from
 * a `fitAddon`. As a result tools like `git`, `ls`, `claude` wrap
 * their output at whatever cols the PTY was opened with (80) and
 * the captured live-log shows hard wraps that don't match the pane.
 *
 * The backend (`shell.rs`) already accepts `termsize: { rows, cols }`
 * on incoming `controllerinput` messages and forwards it to
 * `master.resize(...)`. This hook supplies the value:
 *
 *   1. Attaches a `ResizeObserver` to the supplied container ref.
 *   2. Converts width-in-pixels to cols using the computed monospace
 *      cell width (`font-size × ~0.6`).
 *   3. Debounces by `DEBOUNCE_MS` so a drag-resize emits at most one
 *      RPC per gesture.
 *   4. Sends `RpcApi.ControllerInputCommand` with `termsize`.
 *
 * Caveat (documented in
 * `docs/analysis/AGENT_PANE_PTY_WRAP_2026_05_23.md`): lines already
 * in the live-log buffer stay wrapped at whatever cols was active
 * when they were captured. Only NEW output reflows.
 */

import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { onCleanup, onMount, type Accessor } from "solid-js";

export interface UsePtyWidthOpts {
    blockId: string;
    /** Accessor returning the element whose width drives the PTY cols. */
    elementRef: Accessor<HTMLElement | undefined>;
    /** Optional logger; warnings are surfaced through the activity log. */
    log?: (tag: string, text: string, level?: "info" | "warn" | "error") => void;
}

/** Approximate monospace cell-width ratio (cell_width ≈ font-size × 0.6). */
const CELL_WIDTH_RATIO = 0.6;
/** Coalesce burst resize events (drag) into a single RPC. */
const DEBOUNCE_MS = 150;
/** Minimum cols floor — avoids pathological `cols: 0` from very narrow panes. */
const MIN_COLS = 40;
/** Default rows; the agent pane is scrollable so the value rarely matters. */
const DEFAULT_ROWS = 25;
/** Horizontal padding fudge (px); the pane has small inset around content. */
const PADDING_X_PX = 16;

function computeCols(widthPx: number, fontSizePx: number): number {
    const cellWidth = Math.max(1, fontSizePx * CELL_WIDTH_RATIO);
    const usable = Math.max(0, widthPx - PADDING_X_PX);
    const raw = Math.floor(usable / cellWidth);
    return Math.max(MIN_COLS, raw);
}

function readFontSizePx(el: HTMLElement): number {
    // computed font-size is always returned in px by getComputedStyle.
    const cs = getComputedStyle(el);
    const parsed = parseFloat(cs.fontSize);
    if (Number.isFinite(parsed) && parsed > 0) return parsed;
    return 13; // matches the SCSS fallback for --termfontsize.
}

export function usePtyWidth(opts: UsePtyWidthOpts): void {
    onMount(() => {
        const el = opts.elementRef();
        if (!el) return;
        // `ResizeObserver` is browser-native (Chromium-based host).
        if (typeof ResizeObserver === "undefined") return;

        let lastCols = -1;
        let timer: ReturnType<typeof setTimeout> | undefined;

        const send = (cols: number) => {
            if (cols === lastCols) return;
            lastCols = cols;
            RpcApi.ControllerInputCommand(TabRpcClient, {
                blockid: opts.blockId,
                termsize: { rows: DEFAULT_ROWS, cols },
            }).catch((err: unknown) => {
                opts.log?.(
                    "pty",
                    `resize to ${cols} cols failed: ${
                        err instanceof Error ? err.message : String(err)
                    }`,
                    "warn",
                );
            });
        };

        const compute = () => {
            const target = opts.elementRef();
            if (!target) return;
            const width = target.clientWidth;
            if (width <= 0) return;
            const fontSize = readFontSizePx(target);
            send(computeCols(width, fontSize));
        };

        const observer = new ResizeObserver(() => {
            if (timer) clearTimeout(timer);
            timer = setTimeout(compute, DEBOUNCE_MS);
        });
        observer.observe(el);

        // Fire once after mount so the agent gets the right cols before
        // its first tool invocation, not just after the user resizes.
        // Use the same debounced path so this initial value can be
        // coalesced with any layout-driven ResizeObserver burst at mount.
        if (timer) clearTimeout(timer);
        timer = setTimeout(compute, DEBOUNCE_MS);

        onCleanup(() => {
            if (timer) clearTimeout(timer);
            observer.disconnect();
        });
    });
}

// Export internals for unit tests / sanity checks.
export const __test__ = { computeCols, CELL_WIDTH_RATIO, MIN_COLS, DEBOUNCE_MS };
