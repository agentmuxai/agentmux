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
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import type { ShellNode } from "../types";
import type { StreamFlushQueue } from "../stream-flush-queue";

export interface UseShellNodeStreamOptions {
    blockId: string;
    queue: StreamFlushQueue;
}

/** Payload carried by a `shell_node_create` WPS event. */
interface ShellNodeCreateData {
    shell_id?: unknown;
    cmd?: unknown;
    title?: unknown;
    cwd?: unknown;
    timestamp?: unknown;
}

/**
 * Given a `ShellStatusCommand` response for a shell whose `shell_node_create`
 * was just handled, decide whether a correction is needed — i.e. the shell
 * had ALREADY exited by the time we checked, so the Activity Dock shouldn't
 * keep showing it as "running" until the per-shell exit-chunk replay
 * (subscribed in parallel, but a slower subscribe+ring-replay round trip)
 * eventually corrects it on its own.
 *
 * `shell_node_create` fires identically for a genuinely-live spawn AND for a
 * replay of the block's recent shell history (persist:64 ring, replayed on
 * every pane mount/reconnect) — the event payload carries no status, so the
 * node is always created as "running" first (see the handler below; this
 * keeps the existing ordering guarantee that a shell's own chunk/exit
 * events, which the reducer drops for an unknown id, always find a node
 * already present). Without this correction, every already-long-exited
 * shell rendered as "running" until that slower replay path caught up: a
 * visible flash of stale rows on every load. See
 * docs/retro/retro-activity-dock-stale-shell-flash-on-load-2026-08-22.md.
 *
 * Returns `null` when no correction is needed: `status.known` is `false`
 * (no registry entry yet — do NOT treat this as "exited") or the shell is
 * genuinely still running. `known: false` is not a rare edge case: since
 * `shell_node_create` publishes BEFORE the runner even reaches
 * registration (see server/mod.rs's `handle_shell_create`), a live,
 * freshly-spawned shell routinely has no registry entry yet at the exact
 * moment this check runs. Treating that the same as "confirmed exited"
 * would misreport a genuinely-running shell (e.g. a real `task dev`) as
 * failed for its entire run, since nothing ever restores a status once
 * set — reagent P1 round 2 on PR #2770.
 *
 * `fallbackTimestamp` (the shell's own creation timestamp from the
 * `shell_node_create` payload) stands in for the real exit time, which
 * `ShellStatusResponse` doesn't carry. That's fine for what this exists to
 * fix: for an already-long-exited shell the proxy is already far outside
 * the dock's retention window either way, so the row still renders as
 * invisible the instant this correction lands. The real `exitedAt`/exit
 * code get refined moments later regardless, once the shell's own
 * `shell:<id>` chunk-ring replay delivers its actual exit event.
 */
export function shellStatusCorrection(
    status: { known: boolean; running: boolean; exit_code?: number },
    fallbackTimestamp: number,
): { status: "exited-ok" | "exited-err"; exitCode: number; exitedAt: number } | null {
    if (!status.known || status.running) return null;
    return {
        status: status.exit_code === 0 ? "exited-ok" : "exited-err",
        exitCode: status.exit_code ?? -1,
        exitedAt: fallbackTimestamp,
    };
}

export function useShellNodeStream(opts: UseShellNodeStreamOptions): void {
    // Shell ids whose REAL terminal event (from the per-shell shell:<id>
    // ring — handleShellChunk's exit branch below) has already landed. The
    // ShellStatusCommand correction below and this real exit/stop event are
    // two independent async round trips with no ordering guarantee either
    // way; once the real one lands, it must never be overwritten by the
    // synthesized correction arriving after it — the real event carries the
    // true status (including "stopped", which ShellStatusResponse has no
    // way to express at all) and exact exitedAt, while the synthesized one
    // only distinguishes exited-ok/exited-err and stands in spawnedAt as a
    // proxy timestamp. reagent P1 round 3 on PR #2770 / codex on the same
    // line: without this guard, a shell that legitimately exits/stops for
    // real BEFORE the status check resolves gets its correct row stomped by
    // a stale, less-accurate one moments later (ShellStatusUpdate in the
    // reducer overwrites unconditionally, regardless of current status).
    const reallyResolved = new Set<string>();

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
            reallyResolved.add(shellId);
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

    // shell_node_create: backend published immediately when Shell tool is called
    // — OR replayed (persist:64 ring) on every pane mount/reconnect for every
    // shell in the block's recent history, indistinguishable from a live spawn
    // in the event payload itself. We build the full ShellNode (always
    // "running" at this point — unchanged from before, so a chunk/exit event
    // for this shell can never race ahead of the node existing; see
    // subscribeShellScope below) and queue it for the next RAF flush. We also
    // subscribe to this shell's per-shell `shell_chunk` ring so its chunks/exit
    // replay on remount even if a sibling shell evicted them from the block ring.
    const shellNodeCreateUnsub = waveEventSubscribe({
        eventType: WpsEvent.ShellNodeCreate,
        scope: `block:${opts.blockId}`,
        handler: (event: any) => {
            const d = event?.data as ShellNodeCreateData | undefined;
            if (!d || typeof d !== "object") return;
            const shellId = typeof d.shell_id === "string" ? d.shell_id : "";
            if (!shellId) return;
            const spawnedAt = typeof d.timestamp === "number" ? d.timestamp : Date.now();
            const node: ShellNode = {
                type: "shell",
                id: shellId,
                cmd: typeof d.cmd === "string" ? d.cmd : "",
                title: typeof d.title === "string" ? d.title : (typeof d.cmd === "string" ? d.cmd : ""),
                cwd: typeof d.cwd === "string" ? d.cwd : undefined,
                status: "running",
                spawnedAt,
                log: { chunks: [], open: true },
            };
            opts.queue.pushShellCreate(node);
            subscribeShellScope(shellId);
            opts.queue.scheduleFlush();

            // Resolve the shell's TRUE current status and correct immediately
            // if it already exited — see shellStatusCorrection's doc comment
            // for the full "replay vs. live spawn" rationale. This
            // authoritative registry lookup typically resolves much faster
            // than the per-shell scope subscription's own subscribe+ring-
            // replay round trip just kicked off above, so in practice the
            // correction lands before the "running" row this pushShellCreate
            // just queued ever actually paints.
            RpcApi.ShellStatusCommand(TabRpcClient, { shell_id: shellId })
                .then((status) => {
                    // The real exit/stop event (a separate, independently-
                    // racing async round trip via the per-shell scope
                    // subscribed above) already landed and is authoritative
                    // — never overwrite it with this synthesized guess. See
                    // `reallyResolved`'s doc comment.
                    if (reallyResolved.has(shellId)) return;
                    const correction = shellStatusCorrection(status, spawnedAt);
                    if (!correction) return;
                    opts.queue.pushShellExit(
                        shellId,
                        correction.status,
                        correction.exitCode,
                        correction.exitedAt,
                    );
                    opts.queue.scheduleFlush();
                })
                .catch(() => {
                    // Best-effort — if this shell really did already exit, its
                    // own exit-chunk replay (already subscribed above) still
                    // corrects the dock on its own, just not as fast.
                });
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
