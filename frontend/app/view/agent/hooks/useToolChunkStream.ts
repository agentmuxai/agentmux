// Copyright 2025, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * useToolChunkStream — the single per-block WPS subscription for `tool_chunk`
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
import { waveEventSubscribe } from "@/app/store/wps";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import type { StreamFlushQueue } from "../stream-flush-queue";

export interface UseToolChunkStreamOptions {
    blockId: string;
    queue: StreamFlushQueue;
}

/**
 * Pure parse of a `tool_chunk` event's `op: "pid"` payload (bashwrap's own
 * OS pid for a declared-background invocation — see bash_wrap.rs). Returns
 * `null` for anything else, including a `"pid"` op missing a valid
 * `tool_id`/`pid`, so the caller can fail closed with one check. Extracted
 * from the WPS handler below purely for unit-testability, mirroring
 * `useCompactionStream.ts`'s `resolveCompactionStart` pattern.
 */
export function parsePidChunk(data: unknown): { toolId: string; pid: number } | null {
    if (!data || typeof data !== "object") return null;
    const d = data as Record<string, unknown>;
    if (d.op !== "pid") return null;
    const toolId = typeof d.tool_id === "string" ? d.tool_id : "";
    if (!toolId) return null;
    if (typeof d.pid !== "number" || !Number.isFinite(d.pid)) return null;
    return { toolId, pid: d.pid };
}

export function useToolChunkStream(opts: UseToolChunkStreamOptions): void {
    // Single per-block WPS subscription for `tool_chunk` events.
    // `agentmux-bashwrap exec` publishes every stdout/stderr line to a
    // fixed event name with `scopes: ["block:<id>"]` and the tool_use_id
    // in the payload. The broker persists ~1024 events per scope, so
    // the subscription installed on mount picks up any chunks that
    // landed before Claude's stream-json caught up enough for the
    // frontend to learn the tool_use_id — closes the late-subscribe
    // race that the previous per-tool subscription model could not.
    // See `docs/specs/SPEC_STREAMING_BASH_RUNNER_2026_05_11.md` §6.
    const blockChunkUnsub = waveEventSubscribe({
        eventType: "tool_chunk",
        scope: `block:${opts.blockId}`,
        handler: (event: any) => {
            const data = event?.data;
            if (!data || typeof data !== "object") return;
            if (data.op === "pid") {
                // Not renderable stream content, so it bypasses the
                // queue/RAF machinery entirely — a plain fire-and-forget
                // relay into db_background_tasks.pid. See
                // docs/specs/SPEC_BACKGROUND_TASK_PID_CAPTURE_2026_08_20.md.
                const parsed = parsePidChunk(data);
                if (parsed) {
                    void RpcApi.BackgroundTaskPidCommand(TabRpcClient, {
                        blockid: opts.blockId,
                        node_id: parsed.toolId,
                        pid: parsed.pid,
                    }).catch(() => {});
                }
                return;
            }
            const toolId = typeof data.tool_id === "string" ? data.tool_id : "";
            if (!toolId) return;
            if (data.op === "terminal") {
                opts.queue.pushToolChunk(toolId, {
                    kind: "system",
                    content: `[exited ${data.exit_code ?? "?"}]`,
                    timestamp: data.timestamp ?? Date.now(),
                });
                opts.queue.scheduleFlush();
                return;
            }
            if (data.op !== "chunk") return;
            opts.queue.pushToolChunk(toolId, {
                kind: data.kind ?? "stdout",
                content: data.content ?? "",
                timestamp: data.timestamp ?? Date.now(),
            });
            opts.queue.scheduleFlush();
        },
    });

    // Own the tool_chunk subscription at body scope so it is torn down even if
    // the caller's onMount early-returns (e.g. enabled:false). Without this
    // the global handler would leak one per mount.
    onCleanup(() => { try { blockChunkUnsub(); } catch { /* ignore */ } });
}
