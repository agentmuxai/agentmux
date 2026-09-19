// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * useTrackedProcesses — reactive list of the processes a block started.
 *
 * Replaces `useProcessCount`, which kept only a running total for the
 * composer's `⚙ N` badge. The Shell drawer's info panel needs the processes
 * themselves (name, pid, memory), so the list is the primitive and the count
 * is `list().length`. See
 * `docs/specs/SPEC_AGENT_SHELL_DRAWER_INFO_PANEL_2026_09_19.md` §4.
 *
 * Membership comes from the backend's per-block tracker, which since #3430
 * reports only what the agent started through a shell — not the agent CLI
 * itself, its conhost, or its MCP servers.
 *
 * Works for any tracked block id, including the drawer shell's own sub-block
 * (whose root is a shell, so its children count).
 */

import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import * as MOS from "@/app/store/mos";
import { muxEventSubscribe } from "@/app/store/mps";
import { createEffect, createSignal, onCleanup, type Accessor } from "solid-js";

// Derived from the RPC signature rather than restated, so the two cannot drift.
type ProcessListResult = Awaited<ReturnType<typeof RpcApi.AgentProcessListCommand>>;
export type TrackedProcessInfo = ProcessListResult["processes"][number];
export type TrackingConfidence = ProcessListResult["confidence"];

export interface TrackedProcesses {
    /** Current members, newest additions appended. */
    list: Accessor<TrackedProcessInfo[]>;
    /** How reliable this platform's tracking is; `none` = no tracker. */
    confidence: Accessor<TrackingConfidence>;
    /** Re-fetch the snapshot — refreshes memory figures, which events don't carry. */
    refresh: () => void;
}

export function useTrackedProcesses(blockId: () => string | undefined): TrackedProcesses {
    const [list, setList] = createSignal<TrackedProcessInfo[]>([]);
    const [confidence, setConfidence] = createSignal<TrackingConfidence>("none");

    const fetchSnapshot = (id: string) => {
        if (!TabRpcClient) return;
        void RpcApi.AgentProcessListCommand(TabRpcClient, { block_id: id })
            .then((res) => {
                setList(res.processes ?? []);
                setConfidence(res.confidence);
            })
            // Older backends without the command stay in delta-only mode and
            // still converge on the next event.
            .catch(() => {});
    };

    // An effect, not onMount: the drawer's shell sub-block usually doesn't
    // exist yet when this mounts (it's created on first open), and a pane can
    // replace its shell. Re-subscribes whenever the id changes, and clears
    // the list so a stale block's processes never show under a new one.
    createEffect(() => {
        const id = blockId();
        if (!id) {
            setList([]);
            setConfidence("none");
            return;
        }

        // Deltas are subscribed BEFORE the snapshot is requested: the reverse
        // order drops every event that lands during the round trip, because
        // the snapshot response overwrites the list wholesale. Adds that
        // arrive pre-seed are kept and merged in; a pid already in the
        // snapshot is not duplicated.
        let seeded = false;
        const pending: TrackedProcessInfo[] = [];
        const exitedBeforeSeed = new Set<number>();

        const upsert = (p: TrackedProcessInfo) =>
            setList((cur) => (cur.some((x) => x.pid === p.pid) ? cur : [...cur, p]));

        const unsubAdded = muxEventSubscribe({
            eventType: "agent:process-added",
            scope: MOS.makeORef("block", id),
            handler: (event) => {
                const p = (event.data as { process?: TrackedProcessInfo } | undefined)?.process;
                if (!p) return;
                if (seeded) upsert(p);
                else pending.push(p);
            },
        });
        const unsubExited = muxEventSubscribe({
            eventType: "agent:process-exited",
            scope: MOS.makeORef("block", id),
            handler: (event) => {
                const pid = (event.data as { pid?: number } | undefined)?.pid;
                if (pid == null) return;
                if (seeded) setList((cur) => cur.filter((p) => p.pid !== pid));
                else exitedBeforeSeed.add(pid);
            },
        });

        const seed = (processes: TrackedProcessInfo[]) => {
            const merged = [...processes];
            for (const p of pending) {
                if (!merged.some((x) => x.pid === p.pid)) merged.push(p);
            }
            setList(merged.filter((p) => !exitedBeforeSeed.has(p.pid)));
            seeded = true;
        };

        if (!TabRpcClient) {
            seed([]);
        } else {
            void RpcApi.AgentProcessListCommand(TabRpcClient, { block_id: id })
                .then((res) => {
                    setConfidence(res.confidence);
                    seed(res.processes ?? []);
                })
                .catch(() => seed([]));
        }

        onCleanup(() => {
            unsubAdded?.();
            unsubExited?.();
        });
    });

    return {
        list,
        confidence,
        refresh: () => {
            const id = blockId();
            if (id) fetchSnapshot(id);
        },
    };
}
