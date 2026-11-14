// Copyright 2025, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * useToolChunkStream — the single per-block MPS subscription for `tool_chunk`
 * events (`agentmux-bashwrap exec` output). Pushes every chunk into the
 * shared `StreamFlushQueue` rather than dispatching or scheduling its own
 * flush — a second independent RAF/`batch()` here would reintroduce the
 * reconcileArrays/replaceChild crash documented in
 * RETRO_REPLACECHILD_CRASH_2026-06-06.md. See stream-flush-queue.ts's
 * module doc for the full rationale.
 *
 * Installed at BODY scope by the caller (called directly, not from inside
 * `onMount`) so the subscription tears down even if the caller's own
 * `onMount` early-returns (e.g. `enabled: false`) — the only other
 * teardown path lives inside that `onMount`'s `onCleanup`, which is
 * skipped on early-return.
 */

import { onCleanup } from "solid-js";
import { onMuxEvent } from "@/app/store/mps";
import type { StreamFlushQueue } from "../stream-flush-queue";

import { WpsEvent } from "@/app/store/mps-events";
export interface UseToolChunkStreamOptions {
    blockId: string;
    queue: StreamFlushQueue;
    /** Every output chunk's text, by tool call id (the live status reads a
     *  test runner's progress from it). */
    onOutput?: (toolId: string, content: string) => void;
}

export function useToolChunkStream(opts: UseToolChunkStreamOptions): void {
    // Single per-block MPS subscription for `tool_chunk` events.
    // `agentmux-bashwrap exec` publishes every stdout/stderr line to a
    // fixed event name with `scopes: ["block:<id>"]` and the tool_use_id
    // in the payload. The broker persists ~1024 events per scope, so
    // the subscription installed on mount picks up any chunks that
    // landed before Claude's stream-json caught up enough for the
    // frontend to learn the tool_use_id — closes the late-subscribe
    // race that the previous per-tool subscription model could not.
    // See `docs/specs/SPEC_STREAMING_BASH_RUNNER_2026_05_11.md` §6.
    // srv refuses a `tool_chunk` that isn't a `ToolChunk` (mps_events::client_events!), so `data`
    // is one of its ops.
    const blockChunkUnsub = onMuxEvent(
        WpsEvent.ToolChunk,
        (data) => {
            // A pid is not stream content. srv files it in the background-task registry itself, from the same
            // event, so a pane that is closed or parked still gets it (`background_task_pid.rs`).
            if (data.op === "pid") return;
            const toolId = data.tool_id;
            if (!toolId) return;
            if (data.op === "terminal") {
                opts.queue.pushToolChunk(toolId, {
                    kind: "system",
                    content: `[exited ${data.exit_code}]`,
                    timestamp: data.timestamp,
                });
                opts.queue.scheduleFlush();
                return;
            }
            if (data.op !== "chunk") return;
            opts.onOutput?.(toolId, data.content);
            opts.queue.pushToolChunk(toolId, {
                kind: data.kind,
                content: data.content,
                timestamp: data.timestamp,
            });
            opts.queue.scheduleFlush();
        },
        `block:${opts.blockId}`
    );

    // Own the tool_chunk subscription at body scope so it is torn down even if
    // the caller's onMount early-returns (e.g. enabled:false). Without this
    // the global handler would leak one per mount.
    onCleanup(() => { try { blockChunkUnsub(); } catch { /* ignore */ } });
}
