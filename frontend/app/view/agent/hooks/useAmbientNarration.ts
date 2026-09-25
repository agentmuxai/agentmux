// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Ambient narration — short lines describing something AgentMux did on its own,
 * shown in the pane's conversation.
 *
 * First consumer: a tool call the harness detached to the background. The pane
 * goes quiet at that moment and the user is told nothing about why, or about
 * what is still running.
 *
 * Each accepted narration is handed to `onNarration` as an
 * `AmbientNarrationNode`; the caller inserts it into the pane's document, where
 * it renders in-flow in the agent's voice (see `AmbientNarrationBlock`). This
 * hook owns only the subscription and payload validation — retention is the
 * document store's job, so nothing is dropped here.
 *
 * WHY THE NODE IS MARKED, NOT DISGUISED. The line reads as the agent talking,
 * but the model did not say it and its transcript does not contain it. An
 * unmarked line would read back later as something the model claimed. The
 * rendered node carries a small trailing `ambient` tag for exactly that reason.
 *
 * Best-effort by construction. The backend caps concurrency and may cancel or
 * fail, in which case nothing arrives. Nothing here gates a UI state change on
 * a narration — the action being narrated has already happened and the pane
 * already reflects it.
 *
 * View-only: these never enter the CLI transcript, so they cannot alter what
 * the model sees on its next turn. See
 * docs/specs/SPEC_AMBIENT_NARRATION_INLINE_AGENT_VOICE_2026_09_24.md and
 * docs/reports/REPORT_AGENT_PANE_PROGRESS_INDICATORS_CONSOLIDATION_2026_09_09.md
 * §8.3.
 */

import { onCleanup } from "solid-js";

import { muxEventSubscribe } from "@/app/store/mps";
import { WpsEvent } from "@/app/store/mps-events";
import type { AmbientNarrationNode } from "../types";

let narrationSeq = 0;

export function useAmbientNarration(blockId: string, onNarration: (node: AmbientNarrationNode) => void): void {
    const unsub = muxEventSubscribe({
        eventType: WpsEvent.AmbientNarration,
        scope: `block:${blockId}`,
        handler: (event: unknown) => {
            const data = (event as { data?: { text?: unknown; kind?: unknown } } | undefined)?.data;
            const text = typeof data?.text === "string" ? data.text.trim() : "";
            if (!text) return;
            const kind = typeof data?.kind === "string" ? data.kind : "unknown";
            const at = Date.now();
            // Unique per arrival: two narrations in the same millisecond must not
            // collide on id, or the reducer's id-dedup would drop the second.
            onNarration({
                type: "ambient_narration",
                id: `ambient-${at}-${++narrationSeq}`,
                kind,
                text,
                timestamp: at,
            });
        },
    });
    onCleanup(() => {
        try {
            unsub();
        } catch {
            /* ignore */
        }
    });
}
