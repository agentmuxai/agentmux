// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * useAgentActivitySummary — drives the live mini-summary in the agent pane header.
 *
 * On every completed agent turn, sends the last 30 lines of session output to
 * claude-haiku-4-5-20251001 and asks for a short phrase (~word_target words)
 * describing what was just done. The backend writes the result directly to the
 * `term:activity` block meta key, which agent-model.ts already surfaces in the
 * pane header (viewText fallback chain).
 *
 * Word target is derived from pane width so narrow panes get ~5 words and wide
 * panes can accommodate up to 12.
 *
 * On turn start (Submitting phase) the stale label is cleared so the header
 * doesn't show a phrase from the previous turn while the agent is working.
 */

import { createEffect, on, type Accessor } from "solid-js";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { makeORef } from "@/app/store/wos";
import { ObjectService } from "@/app/store/services";
import { fireAndForget } from "@/util/util";
import type { TurnPhase } from "@/app/store/agent-pane-state/types";

export interface UseAgentActivitySummaryOptions {
    blockId: string;
    turnPhase: Accessor<TurnPhase>;
    getRootWidth: () => number | undefined;
}

export function useAgentActivitySummary(opts: UseAgentActivitySummaryOptions): void {
    const { blockId, turnPhase, getRootWidth } = opts;

    createEffect(on(turnPhase, (phase) => {
        if (phase.kind === "Submitting") {
            // Clear the stale label immediately when the user sends a new message.
            fireAndForget(() =>
                ObjectService.UpdateObjectMeta(makeORef("block", blockId), {
                    "term:activity": null,
                } as any)
            );
            return;
        }

        if (phase.kind === "Done") {
            const rootWidth = getRootWidth() ?? 400;
            // Approximate available header text width; subtract space for icon, name, buttons.
            const textWidth = Math.max(0, rootWidth - 280);
            // ~48px per word at typical proportional font size.
            const wordTarget = Math.max(5, Math.min(12, Math.floor(textWidth / 48)));

            RpcApi.AgentActivitySummaryCommand(
                TabRpcClient,
                { block_id: blockId, word_target: wordTarget },
                { timeout: 20_000 },
            ).catch(() => {
                // Silently ignore errors — the header just stays blank.
            });
        }
    }));
}
