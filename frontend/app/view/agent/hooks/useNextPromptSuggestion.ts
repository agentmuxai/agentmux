// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * useNextPromptSuggestion — ghost-text "predicted next user message" in the
 * agent pane composer.
 *
 * Mirrors Claude Code CLI's own interactive-mode feature (a dimmed
 * suggestion shown in the empty input box after a turn finishes, Tab
 * accepts it): that native mechanism is unreachable from AgentMux because
 * it's unconditionally suppressed in non-interactive (`-p`) mode, which is
 * how AgentMux always drives the CLI. This reimplements the same UX as a
 * second Ambient Model Call gateway purpose. See
 * docs/specs/SPEC_AMBIENT_GHOST_TEXT_NEXT_PROMPT_2026_07_03.md.
 *
 * On every completed agent turn, sends recent session output to
 * claude-haiku-4-5-20251001 and asks for a short, natural next user message.
 * The result is written to `term:next_prompt_suggestion` block meta, which
 * the composer reads as ghost text (dimmed, shown only while the input is
 * empty; Tab accepts it into the real input; any other keystroke dismisses
 * it — see AgentFooter.tsx).
 *
 * Unlike the read-only activity summary (which persists across turns), a
 * stale suggestion here is a correctness bug, not a cosmetic one — it can
 * put words in the user's mouth. Three independent guards:
 *   1. Cleared the instant a new turn starts (`Submitting`) — a suggestion
 *      from turn N is meaningless (and misleading) once turn N+1 has begun.
 *      `term:ambient_summary` deliberately persists across turns; this key
 *      deliberately does not.
 *   2. `activeTurnId !== myTurnId` at write time — a newer turn has already
 *      started (same belt-and-suspenders pattern as
 *      useAgentActivitySummary.ts, on top of the backend gateway's own
 *      cancellation).
 *   3. `!opts.isComposerEmpty()` at write time — the user has started typing
 *      their own message since this request was issued. Checked when the
 *      response arrives (not just "was the composer empty when we sent the
 *      request") specifically to close the race where the RPC resolves
 *      *after* the user already typed something: without this check, a late
 *      response could silently overwrite whatever cleared the suggestion
 *      when typing started (see AgentFooter.tsx's handleInput).
 */

import { createEffect, on, type Accessor } from "solid-js";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { makeORef } from "@/app/store/wos";
import { ObjectService } from "@/app/store/services";
import { fireAndForget } from "@/util/util";
import { recordTurn } from "@/app/store/token-usage";
import type { TurnPhase } from "@/app/store/agent-pane-state/types";

export interface UseNextPromptSuggestionOptions {
    blockId: string;
    turnPhase: Accessor<TurnPhase>;
    /** Checked at write time — see the module doc comment, guard 3. */
    isComposerEmpty: () => boolean;
}

function clearSuggestion(blockId: string): void {
    fireAndForget(() =>
        ObjectService.UpdateObjectMeta(makeORef("block", blockId), {
            "term:next_prompt_suggestion": null,
        } as any)
    );
}

export function useNextPromptSuggestion(opts: UseNextPromptSuggestionOptions): void {
    const { blockId, turnPhase, isComposerEmpty } = opts;

    // Scoped to this mount — see useAgentActivitySummary.ts's doc comment for
    // why the wire `generation` is Date.now() instead of this counter.
    let activeTurnId = 0;

    createEffect(on(turnPhase, (phase) => {
        if (phase.kind === "Submitting") {
            activeTurnId++;
            clearSuggestion(blockId); // guard 1 — see module doc comment
            return;
        }

        if (phase.kind === "Done") {
            const myTurnId = activeTurnId;

            RpcApi.NextPromptSuggestionCommand(
                TabRpcClient,
                { block_id: blockId, generation: Date.now() },
                { timeout: 20_000 },
            ).then((result) => {
                if (activeTurnId !== myTurnId) return; // superseded by a newer turn
                if (result.tokens) {
                    recordTurn("ambient:next_prompt_suggestion", result.tokens);
                }
                if (result.suggestion && isComposerEmpty()) {
                    fireAndForget(() =>
                        ObjectService.UpdateObjectMeta(makeORef("block", blockId), {
                            "term:next_prompt_suggestion": result.suggestion,
                        } as any)
                    );
                }
            }).catch(() => {
                // Silently ignore — no ghost text shows.
            });
        }
    }));
}
