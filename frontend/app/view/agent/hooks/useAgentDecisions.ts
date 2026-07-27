// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * useAgentDecisions — permission decision queue + decide handler.
 *
 * Extracted verbatim from agent-view.tsx. `pendingDecisions()` returns
 * every ToolNode in `pending_approval` (oldest first); `handleDecide`
 * optimistically transitions the matching node and forwards the decision
 * to the sidecar. Spec: docs/specs/SPEC_DECISION_PROMPT_2026_04_24.md.
 */

import { dispatch as dispatchDoc } from "@/app/store/agent-document-store";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import type { DocumentNode, ToolNode } from "../types";
import type { DecisionOutcome } from "../components/AgentDecisionPanel";
import type { LogFn } from "./useAgentControllerStatus";

export interface UseAgentDecisionsOptions {
    blockId: string;
    getDocument: () => DocumentNode[];
    log: LogFn;
}

export interface UseAgentDecisionsResult {
    pendingDecisions: () => ToolNode[];
    handleDecide: (decision: DecisionOutcome) => void;
}

export function useAgentDecisions(opts: UseAgentDecisionsOptions): UseAgentDecisionsResult {
    // Pending decision queue — every ToolNode whose
    // `status === "pending_approval"`, oldest first. The decision
    // panel renders the head; Allow / Deny clears the node by
    // transitioning its status. Defer is HANDLED INSIDE THE PANEL
    // (it minimizes locally) — per
    // docs/specs/SPEC_DECISION_PROMPT_DESIGN_2026_04_25.md §7,
    // the parent must NOT filter pending.
    const pendingDecisions = (): ToolNode[] => {
        const docs = opts.getDocument();
        const out: ToolNode[] = [];
        for (const n of docs) {
            if (n.type === "tool" && n.status === "pending_approval") out.push(n);
        }
        return out;
    };

    const handleDecide = (decision: DecisionOutcome) => {
        // Optimistic UI update — flip the ToolNode out of
        // pending_approval immediately so the panel disappears (or
        // advances to the next pending request). The backend write
        // happens in parallel; if it fails we log but don't try to
        // roll back the visual transition.
        // Dispatch through the reducer (StreamFlush.updatedNodes) so
        // slot.state stays in sync. Find the matching pending tool node
        // by request_id, then build the updated node.
        const updated: ToolNode[] = [];
        for (const n of opts.getDocument()) {
            if (n.type !== "tool" || n.status !== "pending_approval") continue;
            if (n.pendingPermission?.request_id !== decision.request_id) continue;
            updated.push({
                ...n,
                status: decision.outcome === "allow" ? "running" : "denied",
                pendingPermission: undefined,
                // Approval can take arbitrarily long — a call that waited
                // >30s in pending_approval must not read as already past
                // any elapsed-time threshold the instant it starts
                // executing. Refresh `timestamp` to now on allow, so
                // consumers (ToolElapsedTicker, tool-adapter.ts's dock
                // promotion) time from actual execution start, not from
                // when the call was first initiated. reagentx P1, PR #2309.
                timestamp: decision.outcome === "allow" ? Date.now() : n.timestamp,
            });
        }
        if (updated.length > 0) {
            dispatchDoc(
                opts.blockId,
                { type: "StreamFlush", newNodes: [], updatedNodes: updated },
                "user",
            );
        }
        // Send the decision to the sidecar so it can record + audit it.
        // Delivery routes: rules persistence (path 1) or interactive
        // subprocess stdin (path 2) per SPEC_DECISION_PROMPT_2026_04_24.md §9.1.
        void RpcApi.ToolDecisionCommand(TabRpcClient, {
            blockid: opts.blockId,
            request_id: decision.request_id,
            outcome: decision.outcome,
            scope: decision.scope,
            feedback: decision.feedback,
        }).catch((err: unknown) => {
            opts.log("error", `tool:decision failed: ${String(err)}`);
        });
    };

    return { pendingDecisions, handleDecide };
}
