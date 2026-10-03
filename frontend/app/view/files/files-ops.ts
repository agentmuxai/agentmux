// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Copy and move for the Files pane: the shared clipboard (Cut/Copy/Paste work
 * across Hangar panes), and the jobs srv runs (`fs.op.*`), followed through
 * their `files:op` events: progress, a conflict waiting for an answer, the
 * end. docs/specs/SPEC_FILE_BROWSER_PANE_2026_10_01.md §7.1, §7.2.
 */

import { makeORef } from "@/app/store/mos";
import { muxEventSubscribe } from "@/app/store/mps";
import { WpsEvent } from "@/app/store/mps-events";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import type { FsOpEvent } from "@/types/rpc/FsOpEvent";
import { createSignal } from "solid-js";
import { baseName, crumbsOf } from "./files-path";

// ── Clipboard ───────────────────────────────────────────────────────────

export interface FilesClipboard {
    kind: "copy" | "cut";
    paths: string[];
    /** The SSH connection the paths are on; absent for this computer. */
    connection?: string;
}

/** One clipboard for every Hangar pane in this window, like a file manager's. */
const [clipboard, setClipboardSignal] = createSignal<FilesClipboard | null>(null);
export { clipboard };

export function setClipboard(c: FilesClipboard | null): void {
    setClipboardSignal(c);
}

/** The drive or share a path is on: a move within one is a rename, across
 *  two it's a copy then delete. Case-insensitive, as Windows roots are. */
export function rootOf(path: string): string {
    return (crumbsOf(path)[0]?.path ?? "").toLowerCase();
}

/** Whether every source is on the same drive as `dest`. */
export function sameVolume(sources: readonly string[], dest: string): boolean {
    const root = rootOf(dest);
    return sources.every((s) => rootOf(s) === root);
}

// ── Jobs ────────────────────────────────────────────────────────────────

export type OpView = FsOpEvent & {
    /** A short name for the status line: the one source, or "3 items". */
    label: string;
};

/** Follows the jobs started from one pane (the events are scoped to it). */
export class FilesOps {
    readonly ops: () => OpView[];
    private readonly setOps: (fn: (o: OpView[]) => OpView[]) => void;
    private readonly labels = new Map<string, string>();
    /** `fs.op.start` calls awaiting their reply. srv starts the op before it
     *  answers, so its events, even the last, can arrive first (ReAgent on
     *  #4221): they wait here until the reply names the op. */
    private pendingStarts = 0;
    private readonly early = new Map<string, FsOpEvent>();
    private unsubscribe: (() => void) | null;
    /** Called when an op ends, with what to tell the user. */
    onFinished: ((op: OpView) => void) | null = null;

    constructor(private readonly blockId: string) {
        const [ops, setOps] = createSignal<OpView[]>([]);
        this.ops = ops;
        this.setOps = setOps;
        this.unsubscribe = muxEventSubscribe({
            eventType: WpsEvent.FilesOp,
            scope: makeORef("block", blockId),
            handler: (event) => this.onEvent((event as { data?: FsOpEvent }).data),
        });
    }

    private onEvent(data: FsOpEvent | undefined): void {
        if (!data?.op_id) return;
        if (!this.labels.has(data.op_id) && this.pendingStarts > 0) {
            // Keep a final state over a later-looking progress tick.
            const held = this.early.get(data.op_id);
            if (!held || !isFinal(held.state)) this.early.set(data.op_id, data);
            return;
        }
        this.apply(data);
    }

    private apply(data: FsOpEvent): void {
        const view: OpView = { ...data, label: this.labels.get(data.op_id) ?? "items" };
        const finished = isFinal(data.state);
        this.setOps((list) => {
            const rest = list.filter((o) => o.op_id !== data.op_id);
            return finished ? rest : [...rest, view];
        });
        if (finished) {
            this.labels.delete(data.op_id);
            this.onFinished?.(view);
        }
    }

    /** Starts a copy or move of `sources` into `destDir`; either side may be
     *  an SSH host (`sourceConnection`, `connection`; remote terminals spec
     *  §6.3), this computer when absent. */
    async start(
        kind: "copy" | "move",
        sources: string[],
        destDir: string,
        where: { sourceConnection?: string; connection?: string } = {}
    ): Promise<string> {
        this.pendingStarts++;
        let res: { op_id: string };
        try {
            const hosts = {
                ...(where.sourceConnection ? { source_connection: where.sourceConnection } : {}),
                ...(where.connection ? { connection: where.connection } : {}),
            };
            res = await RpcApi.FsOpStartCommand(
                TabRpcClient,
                { kind, sources, dest_dir: destDir, block_id: this.blockId, ...hosts },
                // Reaching a host may first ask the user something over ssh.
                Object.keys(hosts).length > 0 ? { timeout: 180_000 } : undefined
            );
        } finally {
            this.pendingStarts--;
        }
        const label = sources.length === 1 ? baseName(sources[0]) : `${sources.length} items`;
        this.labels.set(res.op_id, label);
        const held = this.early.get(res.op_id);
        this.early.delete(res.op_id);
        if (held) {
            // It already reported, maybe finished: replay that, named.
            this.apply(held);
        } else {
            this.showStarted(res.op_id, kind, sources.length, label);
        }
        this.flushUnclaimed();
        return res.op_id;
    }

    /** Events nobody's reply claimed (none should remain once no start is in
     *  flight) are shown as they are rather than dropped. */
    private flushUnclaimed(): void {
        if (this.pendingStarts > 0) return;
        for (const [id, ev] of this.early) {
            this.early.delete(id);
            this.apply(ev);
        }
    }

    /** Show a just-started op before its first event. */
    private showStarted(opId: string, kind: "copy" | "move", count: number, label: string): void {
        this.setOps((list) =>
            list.some((o) => o.op_id === opId)
                ? list.map((o) => (o.op_id === opId ? { ...o, label } : o))
                : [
                      ...list,
                      {
                          op_id: opId,
                          kind,
                          state: "running",
                          done_items: 0,
                          total_items: count,
                          done_bytes: 0,
                          total_bytes: 0,
                          label,
                      } as OpView,
                  ]
        );
    }

    async resolve(opId: string, choice: "replace" | "skip" | "keep_both", applyToAll: boolean): Promise<void> {
        await RpcApi.FsOpResolveCommand(TabRpcClient, { op_id: opId, choice, apply_to_all: applyToAll });
    }

    async cancel(opId: string): Promise<void> {
        await RpcApi.FsOpCancelCommand(TabRpcClient, { op_id: opId });
    }

    dispose(): void {
        this.unsubscribe?.();
        this.unsubscribe = null;
    }
}

const isFinal = (state: FsOpEvent["state"]): boolean => state === "done" || state === "failed" || state === "canceled";

/** "Copying report.pdf · 3 of 10 · 45%" for the status line. */
export function opProgressText(op: OpView): string {
    const verb = op.kind === "move" ? "Moving" : "Copying";
    const parts = [`${verb} ${op.label}`];
    if (op.total_items > 1) parts.push(`${Math.min(op.done_items + 1, op.total_items)} of ${op.total_items}`);
    if (op.total_bytes > 0) parts.push(`${Math.floor((op.done_bytes / op.total_bytes) * 100)}%`);
    return parts.join(" · ");
}

/** What the status line says when an op ends. */
export function opFinishedText(op: OpView): { text: string; tone: "info" | "error" } {
    const verb = op.kind === "move" ? "Moved" : "Copied";
    // srv gives a reason when it stopped the op itself (a conflict nobody
    // answered for an hour): say that, not just "canceled" (ReAgent on #4221).
    if (op.state === "canceled" && op.error) return { text: op.error, tone: "error" };
    if (op.state === "canceled") return { text: `${op.kind === "move" ? "Move" : "Copy"} canceled after ${op.done_items} of ${op.total_items}`, tone: "info" };
    if (op.state === "failed") return { text: `Couldn't ${op.kind}: ${op.error ?? "it failed"}`, tone: "error" };
    const failed = op.failures?.length ?? 0;
    if (failed > 0) {
        const first = op.failures![0];
        return { text: `${verb} ${op.done_items - failed} of ${op.total_items}; ${baseName(first.path)}: ${first.error ?? "failed"}${failed > 1 ? ` (and ${failed - 1} more)` : ""}`, tone: "error" };
    }
    return { text: `${verb} ${op.label}`, tone: "info" };
}
