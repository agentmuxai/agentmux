// Copyright 2025, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * useShellNodeStream — persistent-shell streaming: the block-scope
 * `shell_node_create` subscription plus the per-shell `shell_chunk` ring
 * subscriptions it spins up. Mirrors useToolChunkStream's contract: every
 * producer here pushes into the shared `StreamFlushQueue` instead of
 * dispatching or scheduling its own flush — see stream-flush-queue.ts's
 * module doc for why a second independent RAF/`batch()` here would
 * reintroduce the reconcileArrays/replaceChild crash
 * (RETRO_REPLACECHILD_CRASH_2026-06-06.md).
 *
 * Installed at BODY scope by the caller (called directly, not from inside
 * `onMount`) — same early-return-safety rationale as useToolChunkStream.
 */

import { onCleanup } from "solid-js";
import { waveEventSubscribe } from "@/app/store/wps";
import { WpsEvent } from "@/app/store/wps-events";
import type { ShellNode } from "../types";
import type { StreamFlushQueue } from "../stream-flush-queue";

export interface UseShellNodeStreamOptions {
    blockId: string;
    queue: StreamFlushQueue;
}

export function useShellNodeStream(opts: UseShellNodeStreamOptions): void {
    // shell_chunk handler — used by the per-shell subscriptions. The payload
    // always carries `shell_id`, so one handler routes correctly. Chunks are
    // delivered via a SINGLE scope (`shell:<id>`); there is no longer a separate
    // block-scope live path, so the same chunk can never arrive twice (the
    // doubled-output bug this fix removes — see shell_node.rs for the full
    // rationale).
    const handleShellChunk = (event: any) => {
        const d = event?.data;
        if (!d || typeof d !== "object") return;
        const shellId = typeof d.shell_id === "string" ? d.shell_id : "";
        if (!shellId) return;
        if (d.op === "exit") {
            const exitCode = typeof d.exit_code === "number" ? d.exit_code : -1;
            // `stopped` = the exit was caused by ShellStop (tree-killed), so show
            // the grey "stopped" status rather than a red exited-err for the
            // non-zero code the kill produces.
            const status: ShellNode["status"] = d.stopped === true
                ? "stopped"
                : exitCode === 0 ? "exited-ok" : "exited-err";
            opts.queue.pushShellExit(shellId, status, exitCode, d.timestamp ?? Date.now());
            opts.queue.scheduleFlush();
            return;
        }
        if (d.op !== "chunk") return;
        opts.queue.pushShellChunk(shellId, {
            kind: d.kind ?? "stdout",
            content: d.content ?? "",
            timestamp: d.timestamp ?? Date.now(),
        });
        opts.queue.scheduleFlush();
    };

    // Per-shell shell_chunk subscriptions. The backend publishes each shell's
    // chunk/exit events under a SINGLE scope: `shell:<shellId>` (a dedicated
    // persist:1024 ring — see shell_node.rs). This is the ONLY delivery path for
    // chunks; we subscribe to it when we learn of the shell (via the persist:64
    // block-scoped shell_node_create that replays on remount). Because the broker
    // persists the ring regardless of subscribers, any output produced before
    // this subscription establishes (the common spawn-beats-resub race) is
    // retained in the ring and replayed exactly once on subscribe — so dropping
    // the old block-scope live subscription cannot lose the first chunks. Each
    // shell having its own ring also means a chatty sibling can't evict another
    // shell's exit event. Tracked here so all per-shell subs tear down on
    // unmount. (SPEC_PERSISTENT_SHELL_NODE — P2 per-shell ring buffer fix.)
    const perShellUnsubs = new Map<string, () => void>();
    const subscribeShellScope = (shellId: string) => {
        if (perShellUnsubs.has(shellId)) return;
        const unsub = waveEventSubscribe({
            eventType: WpsEvent.ShellChunk,
            scope: `shell:${shellId}`,
            handler: handleShellChunk,
        });
        perShellUnsubs.set(shellId, unsub);
    };
    onCleanup(() => {
        for (const unsub of perShellUnsubs.values()) {
            try { unsub(); } catch { /* ignore */ }
        }
        perShellUnsubs.clear();
    });

    // shell_node_create: backend published immediately when Shell tool is called.
    // We build the full ShellNode and queue it for the next RAF flush. We also
    // subscribe to this shell's per-shell `shell_chunk` ring so its chunks/exit
    // replay on remount even if a sibling shell evicted them from the block ring.
    const shellNodeCreateUnsub = waveEventSubscribe({
        eventType: WpsEvent.ShellNodeCreate,
        scope: `block:${opts.blockId}`,
        handler: (event: any) => {
            const d = event?.data;
            if (!d || typeof d !== "object") return;
            const shellId = typeof d.shell_id === "string" ? d.shell_id : "";
            if (!shellId) return;
            const node: ShellNode = {
                type: "shell",
                id: shellId,
                cmd: d.cmd ?? "",
                title: d.title ?? d.cmd ?? "",
                cwd: typeof d.cwd === "string" ? d.cwd : undefined,
                status: "running",
                spawnedAt: d.timestamp ?? Date.now(),
                log: { chunks: [], open: true },
            };
            opts.queue.pushShellCreate(node);
            subscribeShellScope(shellId);
            opts.queue.scheduleFlush();
        },
    });
    onCleanup(() => { try { shellNodeCreateUnsub(); } catch { /* ignore */ } });

    // NOTE: there is intentionally NO block-scope `shell_chunk` subscription.
    // Chunks/exit are delivered solely via the per-shell `shell:<id>` scope
    // (subscribed in subscribeShellScope above). Publishing to both scopes used
    // to double-deliver early output — once live via block, once in the replay
    // burst via shell — which the reducer's last-chunk-only isDuplicate could not
    // collapse. See shell_node.rs shell_scopes() for the full rationale.
}
