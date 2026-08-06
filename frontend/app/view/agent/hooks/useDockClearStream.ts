// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * useDockClearStream — subscribes this pane's block scope to `dock:clear`
 * (published by `handle_muxspect_dock_clear` in response to a `muxspect
 * dock clear` request) and force-cancels the matching `ToolNode` when one
 * arrives.
 *
 * Server-side scope routing (`block:<blockId>`, same mechanism
 * useShellNodeStream's `shell_node_create` subscription uses) means a
 * renderer not currently displaying this block never receives the event —
 * no client-side block-id filtering needed here. The one filter this hook
 * still performs itself is "is `node_id` still present in *my* document,"
 * via the reducer's own `ForceCancelToolNode` no-op path (already resolved
 * or a stale/duplicate event both just no-op).
 *
 * Installed at BODY scope by the caller, same early-return-safety
 * rationale as useShellNodeStream/useToolChunkStream.
 *
 * See docs/specs/SPEC_MUXSPECT_DOCK_DIAGNOSIS_AND_REMEDIATION_2026_08_06.md §3.2.
 */

import { onCleanup } from "solid-js";
import type { AgentPaneModel } from "@/app/store/agent-pane-model";
import { waveEventSubscribe } from "@/app/store/wps";
import { WpsEvent } from "@/app/store/wps-events";

export interface UseDockClearStreamOptions {
    blockId: string;
    model: AgentPaneModel;
}

export function useDockClearStream(opts: UseDockClearStreamOptions): void {
    const unsub = waveEventSubscribe({
        eventType: WpsEvent.DockClear,
        scope: `block:${opts.blockId}`,
        handler: (event: any) => {
            const d = event?.data;
            if (!d || typeof d !== "object") return;
            const nodeId = typeof d.node_id === "string" ? d.node_id : "";
            if (!nodeId) return;
            // model.dispatchDoc is the disposal-safe wrapper — required
            // here, not the raw (throwing) dispatch: this handler can fire
            // after the pane's slot unregisters (pane closed, or the
            // documented CASCADE_DETECTED unmount race) and
            // dispatchToSubjects in wps.ts invokes handlers with no
            // try/catch, so a throw here would be uncaught. reagentx P1 on
            // PR #2432 — mirrors useCompactionStream.ts's use of
            // opts.model.dispatchPane for the identical reason.
            opts.model.dispatchDoc({ type: "ForceCancelToolNode", nodeId }, "system");
        },
    });
    onCleanup(() => { try { unsub(); } catch { /* ignore */ } });
}
