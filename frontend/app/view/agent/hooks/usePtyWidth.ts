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
 * The backend (`shell.rs`) accepts `termsize: { rows, cols }` on incoming
 * `controllerinput` messages and forwards it to `master.resize(...)`, and
 * also seeds the PTY at that size on spawn (`rtopts.termsize` on the resync;
 * see launch-flow.ts). This hook supplies the value for *changes*:
 *
 *   1. Attaches a `ResizeObserver` to the supplied container ref.
 *   2. Converts width-in-pixels to cols using the computed monospace
 *      cell width (`font-size × ~0.6`).
 *   3. Debounces by `DEBOUNCE_MS` so a drag-resize emits at most one
 *      RPC per gesture.
 *   4. Defers the send until the controller is ready to accept input
 *      ("running"), then sends `RpcApi.ControllerInputCommand` with
 *      `termsize` — so a resize never races the per-turn PTY spawn.
 *
 * Caveats: (a) lines already in the live-log buffer stay wrapped at the cols
 * active when they were captured — only NEW output reflows
 * (`docs/analysis/AGENT_PANE_PTY_WRAP_2026_05_23.md`); (b) the initial-resize
 * race and its fix are in `docs/analysis/AGENT_PANE_PTY_RESIZE_RACE_2026_06_16.md`.
 */

import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { BlockService } from "@/app/store/services";
import { waveEventSubscribe } from "@/app/store/wps";
import * as WOS from "@/app/store/wos";
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
    return 15; // matches the SCSS fallback for --termfontsize.
}

/**
 * Compute a `{rows, cols}` TermSize from an element's current width using the
 * same monospace math as the live resize path. Returns undefined when the
 * element is absent or not yet laid out (`clientWidth <= 0`) so callers omit
 * the value rather than send a bogus size.
 *
 * Used to seed the PTY at spawn via the resync `rtopts.termsize`
 * (`launch-flow.ts` Phase 3 → `shell.rs::pty_size_from_rt_opts`), so the agent
 * CLI is born at the right width instead of relying on a post-spawn resize that
 * races controller startup. See
 * docs/analysis/AGENT_PANE_PTY_RESIZE_RACE_2026_06_16.md.
 */
export function computeTermSizeFromEl(
    el: HTMLElement | undefined,
): { rows: number; cols: number } | undefined {
    if (!el) return undefined;
    const width = el.clientWidth;
    if (width <= 0) return undefined;
    return { rows: DEFAULT_ROWS, cols: computeCols(width, readFontSizePx(el)) };
}

/**
 * Only resize failures caused by the controller not yet (or no longer) being
 * able to accept input are worth retrying. Anything else (malformed RPC, etc.)
 * is permanent — retrying just spams the activity log.
 */
function isRetryableResizeError(msg: string): boolean {
    return msg.includes("controller is not running") || msg.includes("no controller for block");
}

export function usePtyWidth(opts: UsePtyWidthOpts): void {
    onMount(() => {
        const el = opts.elementRef();
        if (!el) return;
        // `ResizeObserver` is browser-native (Chromium-based host).
        if (typeof ResizeObserver === "undefined") return;

        let lastCols = -1;
        let timer: ReturnType<typeof setTimeout> | undefined;

        // Readiness gate. The backend's `send_input` rejects a resize with
        // "controller is not running" until the controller's input channel
        // exists, and for an agent pane the process is per-turn — so the PTY
        // can only accept a resize while a turn is live ("running"). Sending
        // before that (e.g. the at-mount ResizeObserver delivery during the
        // launch-flow spawn) is what produced the spurious "failed after 3
        // attempts" warning. So we DEFER sends until the controller is ready,
        // coalesce the latest pending width, and flush on "running". `lastCols`
        // is still set only AFTER the RPC resolves, so a failed send is retried.
        //
        // The initial width is normally already correct: the launch flow seeds
        // the PTY at spawn via `rtopts.termsize` (launch-flow.ts), so this send
        // is a correction, not the thing that fixes wrap.
        // See docs/analysis/AGENT_PANE_PTY_RESIZE_RACE_2026_06_16.md.
        let controllerReady = false;
        let pendingCols: number | null = null;

        const send = (cols: number, retriesLeft: number = 2) => {
            if (cols === lastCols) return;
            // Not live yet — stash the latest width; flushed on "running".
            if (!controllerReady) {
                pendingCols = cols;
                return;
            }
            RpcApi.ControllerInputCommand(TabRpcClient, {
                blockid: opts.blockId,
                termsize: { rows: DEFAULT_ROWS, cols },
            })
                .then(() => {
                    lastCols = cols;
                })
                .catch((err: unknown) => {
                    const msg = err instanceof Error ? err.message : String(err);
                    // Only controller-not-ready races are transient; jitter the
                    // backoff so many panes mounting together don't retry in
                    // lockstep. Layer A makes this path non-load-bearing.
                    if (retriesLeft > 0 && isRetryableResizeError(msg)) {
                        const delay = (3 - retriesLeft) * 400 + Math.floor(Math.random() * 200);
                        opts.log?.(
                            "pty",
                            `resize to ${cols} cols failed: ${msg} (retrying in ${delay}ms)`,
                            "warn",
                        );
                        setTimeout(() => send(cols, retriesLeft - 1), delay);
                    } else {
                        // Either the retry budget is spent or the error is
                        // permanent — say which, so the log isn't misleading.
                        const reason = isRetryableResizeError(msg) ? "after 3 attempts" : "(not retryable)";
                        opts.log?.(
                            "pty",
                            `resize to ${cols} cols failed ${reason}: ${msg}`,
                            "warn",
                        );
                    }
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

        const markReady = () => {
            controllerReady = true;
            if (pendingCols != null && pendingCols !== lastCols) {
                const cols = pendingCols;
                pendingCols = null;
                send(cols);
            }
        };

        const observer = new ResizeObserver(() => {
            if (timer) clearTimeout(timer);
            timer = setTimeout(compute, DEBOUNCE_MS);
        });
        // observe() delivers an initial notification with the current size, so
        // the at-mount width is captured here (deferred via the gate above) —
        // no separate initial timer needed.
        observer.observe(el);

        // Flush on each turn's spawn: "running" now fires only after the input
        // channel is ready (shell.rs). Clear readiness on "done" so a resize
        // made while the agent is idle is coalesced and re-applied when the
        // next turn starts, rather than failing against a dead PTY.
        const unsubStatus = waveEventSubscribe({
            eventType: "controllerstatus",
            scope: WOS.makeORef("block", opts.blockId),
            handler: (event) => {
                const status = (event as any)?.data?.shellprocstatus;
                if (status === "running") markReady();
                else if (status === "done") controllerReady = false;
            },
        });

        // Re-mount path: the controller may already be running, in which case
        // the event above won't re-fire — probe once so a pending resize flushes.
        BlockService.GetControllerStatus(opts.blockId)
            .then((rts) => {
                if (rts?.shellprocstatus === "running") markReady();
            })
            .catch(() => {
                // status unknown — the subscription covers the fresh-launch path.
            });

        onCleanup(() => {
            if (timer) clearTimeout(timer);
            observer.disconnect();
            unsubStatus();
        });
    });
}

// Export internals for unit tests / sanity checks.
export const __test__ = {
    computeCols,
    computeTermSizeFromEl,
    isRetryableResizeError,
    CELL_WIDTH_RATIO,
    MIN_COLS,
    DEBOUNCE_MS,
};
