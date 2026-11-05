// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The memory re-injection controller of one agent pane, wired to the pane's RPCs and reducers. It is what
 * useAgentStream hands every frame that can start a hidden re-injection turn (a compaction boundary, a
 * fresh session) and asks at every session end. The controller itself and its rules are in
 * memory-reinjection-controller.ts; this is only the pane-specific callbacks.
 */

import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { workingFromPhase } from "@/app/store/agent-pane-state/types";
import { snapshot as paneSnapshot } from "@/app/store/agent-pane-state-store";
import type { AgentPaneModel } from "@/app/store/agent-pane-model";
import { createMemoryReinjectionController, type MemoryReinjectionController } from "./memory-reinjection-controller";
import { fetchMemoryReinjectionEntries } from "./memory-reinjection-fetch";
import { buildMemoryInjectedNode } from "./memory-injected";

export function createStreamMemoryReinjection(opts: {
    blockId: string;
    agentName: string | undefined;
    model: AgentPaneModel;
    /** The pane's context window for memory sizing, read when a re-injection is sized. */
    contextWindow: () => number;
}): MemoryReinjectionController {
    const { blockId, agentName, model, contextWindow } = opts;
    return createMemoryReinjectionController({
        contextWindow,
        now: () => Date.now(),
        // A real turn is genuinely in flight for the (#3502):
        // common auto-compaction case (compact_boundary lands mid an
        // ongoing turn) — dispatching TurnStart on top of it regresses
        // turnPhase from Streaming back to Submitting. Same read
        // agent-view.tsx's own handleSendMessage captures as
        // wasAlreadyWorking before deciding whether to dispatch TurnStart
        // at all. See memory-reinjection-controller.ts's module doc
        // comment, "fix 1".
        isPaneWorking: () => workingFromPhase(paneSnapshot(blockId)?.turnPhase ?? { kind: "Idle" }),
        fetchEntries: () => fetchMemoryReinjectionEntries(TabRpcClient, agentName ?? "", blockId),
        // Deliberately the raw send RPC, not the full sendMessage/pending-
        // zone path (§3.3 — that path is what creates the visible
        // UserMessageNode this feature must never produce). This DOES skip
        // sendMessage's own auth-guard/cmd:args-reapply preamble
        // (useAgentCommands.ts) — acceptable here because a reinjection
        // only ever fires immediately after a real compact_boundary, i.e.
        // strictly mid-session on an already-authenticated, already-running
        // process, not a cold user-initiated send where that preamble
        // matters. Flagged, not silently assumed.
        // `hidden: true` (#3502):
        // extract_digest_text's marker-text suppression can only see inside
        // its own 32 KB / ~30-line tail window, which large Personal memory
        // content (measured up to 76 KB in this PR's own §3.4.1 numbers)
        // routinely scrolls past. This flag is the authoritative gate
        // (session::set_hidden_reinjection_active) — set here, not
        // reconstructed from transcript bytes later.
        sendRpc: (message, deliveryId) =>
            RpcApi.AgentInputCommand(TabRpcClient, {
                blockid: blockId,
                message,
                message_id: `memreinject_${Date.now()}_${Math.random().toString(36).slice(2, 8)}`,
                hidden: true,
                // srv writes this delivery's card to the pane (CD2b).
                ...(deliveryId ? { delivery_id: deliveryId } : {}),
            }).then(() => undefined),
        // srv composes the message and the card (CD2b); an older srv rejects
        // and the controller composes here instead.
        compose: async (reason) => {
            const r = await RpcApi.ComposeCommand(TabRpcClient, { block_id: blockId, reason });
            if (!r.text || !r.delivery_id || !r.frame) return null;
            const node = buildMemoryInjectedNode(r.frame, {
                contextWindow: contextWindow(),
                now: Date.now(),
            });
            return node ? { deliveryId: r.delivery_id, text: r.text, node } : null;
        },
        // The catch-up turn's message (SPEC_RESUME_PHASE_CATCH_UP_TURN_
        // 2026_10_13.md): memory when asked for, then the block telling the
        // agent to say where it is picking up. A catch-up with no memory has no
        // card, so no delivery id and no node.
        composeCatchUp: async (reason, memory) => {
            const r = await RpcApi.ComposeCommand(TabRpcClient, { block_id: blockId, reason, catch_up: true, memory });
            if (!r.text) return null;
            const node = r.frame
                ? buildMemoryInjectedNode(r.frame, { contextWindow: contextWindow(), now: Date.now() })
                : null;
            return {
                text: r.text,
                ...(r.delivery_id ? { deliveryId: r.delivery_id } : {}),
                ...(node ? { node } : {}),
            };
        },
        // Claude Code's SessionStart hook now delivers the same memory at a
        // session start and after a compaction; this fallback fires only when
        // the hook didn't (SPEC_GLOBAL_MEMORY_DELIVERY_2026_09_27.md §7).
        claimFallback: (reason, boundaryUuid, eventAtMs) =>
            RpcApi.ClaimFallbackCommand(TabRpcClient, {
                block_id: blockId,
                reason,
                ...(boundaryUuid ? { boundary_uuid: boundaryUuid } : {}),
                ...(eventAtMs !== undefined ? { event_at_ms: eventAtMs } : {}),
            }).then((r) => ({ deliver: r.deliver, ...(r.foreign ? { foreign: true } : {}) })),
        // Reuses the REAL TurnStart/TurnStartFailed commands unmodified — a hidden
        // reinjection is a completely genuine turn state-machine-wise; only
        // its rendering differs. See memory-reinjection-controller.ts's
        // module doc comment. `content`/`hidden` are always exactly what the
        // controller itself passes (HIDDEN_TURN_PLACEHOLDER_CONTENT, true) —
        // this callback forwards verbatim rather than deciding anything, per
        // "fix 2" in that module's doc comment: the REAL memory content must
        // never enter reducer state, only sendRpc's own argument above.
        dispatchTurnStart: (content, hidden) => {
            model.dispatchPane({ type: "TurnStart", at: Date.now(), content, hidden }, "system");
        },
        dispatchTurnStartFailed: () => {
            model.dispatchPane({ type: "TurnStartFailed" }, "system");
        },
    });
}
